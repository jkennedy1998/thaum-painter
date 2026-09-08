//! Raster-channel interpolation (J 2026-09-07): the second real per-property-channel
//! resolver, beside `interp_move`. Each raster keyframe (a solid property block) owns
//! a painted canvas; the empties between them resolve per the empty's authored mode:
//!
//! - **hold** — carries the previous keyframe's canvas through the empty.
//! - **interpolate** — blends the previous keyframe's canvas into the next one across
//!   the empty's span, with the empty's ease ends bending progress exactly like the
//!   move channel's. When a shape-fade resolver is injected (the live app builds
//!   one from the loaded typeface), matched cells' graphics resolve through the
//!   renderer's gradient tour instead of the halfway cutoff — see
//!   `thaum-renderer/domain/cell-graphic/shape-fade`. Per cell, matched by grid position:
//!   - **color** (flat RGB) lerps channel-by-channel, then resolves to the
//!     nearest indexed palette color.
//!   - **weight** lerps numerically — continuous, the other easy part.
//!   - **graphic** (the character / sprite) is discrete: a hard cutoff at the halfway
//!     crossing. The first half of the transition shows the previous keyframe's
//!     graphic, the second half the next one's. The eases still shape *when* the flip
//!     happens, which reads as an intentional swap rather than a pop.
//!   - a material color is discrete like a graphic and rides the same cutoff.
//!   - a cell present in only one keyframe shows during the half its side is
//!     active, with its weight fading toward Zero across that half. Its glyph
//!     also walks the shape-fade system toward/from `▪`, the deliberately
//!     low-coverage clear-transition glyph, before the cell vanishes or
//!     appears at the halfway crossing.
//! - **smear** — interior-only raster mode: conservatively matches compatible
//!   cells across the two keyframes, transports them along discrete 3D paths, and
//!   leaves a weight-tapered cell trail. Each sample uses the same weighted glyph
//!   fade/color rules as ordinary interpolation; unmatched cells use its one-sided
//!   fade rather than becoming speculative trails.
//! - **loop_out / loop_in** — edge-locked modes: replay the authored region through
//!   the trailing / leading blank, resolving each mapped breath through this module.
//!
//! Scrubbing to a blank with no solvable content still resolves `None` — nothing
//! renders there, matching the pre-interpolation behavior.

use std::collections::BTreeSet;

use thaum_renderer_domain::shape_fade::fade::ShapeFade;
use thaum_renderer_domain::{CellGraphic, CellWeight};

use crate::brush::{effective_cell, Canvas, PaintedCell};
use crate::interp_move;
use crate::legacy_indexed_palette::nearest_indexed_rgb;
use crate::paint_color::PaintColor;
use crate::storage::SharedDocumentPropertyBlock;

/// Resolves the painted canvas authored for `breath` across one raster property
/// track's blocks, or `None` when nothing resolves. `canvas_of` supplies a
/// block's own painted canvas (solids only ever need it). Breaths past the
/// trailing blank's stored extent resolve AS the trailing blank, so a loop-out
/// keeps playing in the infinite region instead of dropping to nothing.
pub fn resolve_raster_canvas(
    blocks: &[SharedDocumentPropertyBlock],
    breath: u32,
    canvas_of: impl Fn(&SharedDocumentPropertyBlock) -> Option<Canvas>,
    graphic_fade: Option<&ShapeFade>,
) -> Option<Canvas> {
    let index = blocks
        .iter()
        .position(|block| {
            crate::properties::breath_in_span(breath, block.start_breath, block.length_breaths)
        })
        .or_else(|| {
            // Past the stored extent: the trailing blank covers to infinity.
            blocks
                .last()
                .filter(|block| block.is_blank)
                .map(|_| blocks.len() - 1)
        })?;
    resolve_block(blocks, index, breath, canvas_of, graphic_fade)
}

/// Resolves one block at `breath` (the breath is always inside the block's span
/// or mapped into it by the loop resolver, which cannot re-enter a loop blank).
fn resolve_block(
    blocks: &[SharedDocumentPropertyBlock],
    index: usize,
    breath: u32,
    canvas_of: impl Fn(&SharedDocumentPropertyBlock) -> Option<Canvas>,
    graphic_fade: Option<&ShapeFade>,
) -> Option<Canvas> {
    let block = &blocks[index];
    if !block.is_blank {
        return canvas_of(block);
    }
    match crate::interp_mode::resolve_mode(block.interpretation.as_deref()) {
        "loop_out" => resolve_loop(blocks, index, breath, false, canvas_of, graphic_fade),
        "loop_in" => resolve_loop(blocks, index, breath, true, canvas_of, graphic_fade),
        "hold" => solid_canvas_before(blocks, index, &canvas_of)
            .or_else(|| solid_canvas_after(blocks, index, &canvas_of)),
        "smear" => resolve_smear(blocks, index, breath, canvas_of, graphic_fade),
        _ => resolve_interpolate(blocks, index, breath, canvas_of, graphic_fade),
    }
}

/// Hold: the nearest solid keyframe to the LEFT holds its canvas through the
/// empty. With nothing behind (the empty starts the track) the nearest solid
/// to the right holds instead; with no content at all nothing resolves.
fn solid_canvas_before(
    blocks: &[SharedDocumentPropertyBlock],
    index: usize,
    canvas_of: impl Fn(&SharedDocumentPropertyBlock) -> Option<Canvas>,
) -> Option<Canvas> {
    blocks[..index]
        .iter()
        .rev()
        .find(|block| !block.is_blank)
        .and_then(canvas_of)
}

/// The nearest solid keyframe entirely right of `index`, if any.
fn solid_canvas_after(
    blocks: &[SharedDocumentPropertyBlock],
    index: usize,
    canvas_of: impl Fn(&SharedDocumentPropertyBlock) -> Option<Canvas>,
) -> Option<Canvas> {
    blocks[index + 1..]
        .iter()
        .find(|block| !block.is_blank)
        .and_then(canvas_of)
}

/// Interpolate: blend the previous keyframe's canvas into the next one across
/// the empty's span, progress bent by the same ease model as the move channel.
/// A missing side degrades to holding the existing side's canvas.
fn resolve_interpolate(
    blocks: &[SharedDocumentPropertyBlock],
    index: usize,
    breath: u32,
    canvas_of: impl Fn(&SharedDocumentPropertyBlock) -> Option<Canvas>,
    graphic_fade: Option<&ShapeFade>,
) -> Option<Canvas> {
    let previous = solid_canvas_before(blocks, index, &canvas_of);
    let next = solid_canvas_after(blocks, index, &canvas_of);
    match (previous, next) {
        (Some(from), Some(to)) => {
            let progress = interp_move::empty_progress(&blocks[index], breath);
            Some(blend_canvases(&from, &to, progress, graphic_fade))
        }
        (only, None) => only,
        (None, only) => only,
    }
}

/// Smear: the raster-only interior-empty mode. The smear encapsulation owns
/// correspondence and trail construction; this channel resolver retains
/// keyframe lookup, eased progress, and the ordinary missing-side fallback.
fn resolve_smear(
    blocks: &[SharedDocumentPropertyBlock],
    index: usize,
    breath: u32,
    canvas_of: impl Fn(&SharedDocumentPropertyBlock) -> Option<Canvas>,
    graphic_fade: Option<&ShapeFade>,
) -> Option<Canvas> {
    let previous = solid_canvas_before(blocks, index, &canvas_of);
    let next = solid_canvas_after(blocks, index, &canvas_of);
    match (previous, next) {
        (Some(from), Some(to)) => {
            let progress = interp_move::empty_progress(&blocks[index], breath);
            Some(crate::raster_smear::smear_canvases(
                &from,
                &to,
                progress,
                graphic_fade,
            ))
        }
        (only, None) => only,
        (None, only) => only,
    }
}

/// Loop out / loop in: repeat the authored region (first keyframe start through
/// last keyframe end) through the edge blank, mapping the breath back into the
/// region and resolving it there (depth-one, identical to the move channel).
fn resolve_loop(
    blocks: &[SharedDocumentPropertyBlock],
    index: usize,
    breath: u32,
    mirror: bool,
    canvas_of: impl Fn(&SharedDocumentPropertyBlock) -> Option<Canvas>,
    graphic_fade: Option<&ShapeFade>,
) -> Option<Canvas> {
    let blank = &blocks[index];
    let first_solid = blocks.iter().position(|block| !block.is_blank)?;
    let last_solid = blocks.iter().rposition(|block| !block.is_blank)?;
    let region_start = blocks[first_solid].start_breath;
    let region_end = blocks[last_solid].start_breath + blocks[last_solid].length_breaths;
    let loop_len = region_end.saturating_sub(region_start);
    if loop_len == 0 {
        return None;
    }
    let mapped = if mirror {
        let blank_end = blank.start_breath + blank.length_breaths;
        let distance = blank_end.saturating_sub(1).saturating_sub(breath);
        region_end - 1 - (distance % loop_len)
    } else {
        let distance = breath.saturating_sub(blank.start_breath);
        region_start + (distance % loop_len)
    };
    let mapped_index = blocks.iter().position(|block| {
        crate::properties::breath_in_span(mapped, block.start_breath, block.length_breaths)
    })?;
    resolve_block(blocks, mapped_index, mapped, canvas_of, graphic_fade)
}

/// Blends two keyframe canvases at `progress` (0 = fully `from`, 1 = fully
/// `to`). Cells match by grid position; authored blanks count as absent via
/// the unified empty-cell read seam. See the module header for per-channel
/// rules: flat RGB lerp then palette resolution, continuous weight lerp,
/// discrete graphic/material cutoff at the halfway crossing, and half-span
/// appear/disappear for one-sided cells.
pub fn blend_canvases(
    from: &Canvas,
    to: &Canvas,
    progress: f32,
    graphic_fade: Option<&ShapeFade>,
) -> Canvas {
    let progress = progress.clamp(0.0, 1.0);
    let from_active = progress < 0.5;
    let mut blended = Canvas::new();
    let positions: BTreeSet<_> = from.keys().chain(to.keys()).collect();
    for position in positions {
        let from_cell = effective_cell(from.get(position));
        let to_cell = effective_cell(to.get(position));
        match (from_cell, to_cell) {
            (Some(a), Some(b)) => {
                blended.insert(
                    *position,
                    blend_cells(a, b, progress, from_active, graphic_fade),
                );
            }
            (Some(a), None) if from_active => {
                blended.insert(
                    *position,
                    fade_one_sided_cell(a, progress, true, graphic_fade),
                );
            }
            (None, Some(b)) if !from_active => {
                blended.insert(
                    *position,
                    fade_one_sided_cell(b, progress, false, graphic_fade),
                );
            }
            _ => {}
        }
    }
    blended
}

fn blend_cells(
    from: &PaintedCell,
    to: &PaintedCell,
    progress: f32,
    from_active: bool,
    graphic_fade: Option<&ShapeFade>,
) -> PaintedCell {
    blend_cells_with_weight_scale(from, to, progress, from_active, 1.0, graphic_fade)
}

/// Shared raster-cell appearance resolution. Smear supplies a decreasing
/// `weight_scale` for its trail samples before glyph selection, so ShapeFade
/// selects a glyph that is actually available at the rendered trail weight.
pub(crate) fn blend_cells_with_weight_scale(
    from: &PaintedCell,
    to: &PaintedCell,
    progress: f32,
    from_active: bool,
    weight_scale: f32,
    graphic_fade: Option<&ShapeFade>,
) -> PaintedCell {
    let interpolated_weight = lerp_weight(from.weight_index, to.weight_index, progress);
    let weight_index = scale_weight(interpolated_weight, weight_scale);
    PaintedCell {
        // Discrete channel. With an injected shape-fade resolver and two
        // glyph-backed cells, the graphic walks the renderer's gradient tour
        // (image-only, monotone toward the target). Without one — and for
        // sprite-backed cells, whose tours would need sprite-identity keys —
        // the hard cutoff at the halfway crossing stands, shaped by the eases.
        graphic: resolve_graphic(
            from,
            to,
            progress,
            from_active,
            renderer_weight(from.weight_index),
            renderer_weight(to.weight_index),
            renderer_weight(weight_index),
            graphic_fade,
        ),
        color: blend_colors(from.color, to.color, progress, from_active),
        weight_index,
    }
}

fn scale_weight(weight_index: i64, scale: f32) -> i64 {
    (weight_index as f32 * scale.clamp(0.0, 1.0)).round() as i64
}

/// The shape-fade endpoint used in place of a truly absent cell. `▪` is a
/// deliberately low-coverage loaded glyph, so it carries the transition toward
/// less-covered raster content while still using the shared gradient tour.
const CLEAR_TRANSITION_GLYPH: char = '▪';

/// Resolves a present cell toward/from the clear-transition glyph over its
/// active half, while preserving the existing numeric weight fade. Without a
/// loaded shape-fade graph (or for sprites), the graphic keeps its old behavior.
fn fade_one_sided_cell(
    cell: &PaintedCell,
    progress: f32,
    fading_out: bool,
    graphic_fade: Option<&ShapeFade>,
) -> PaintedCell {
    let mut faded = cell.clone();
    faded.weight_index = fade_one_sided_weight(cell.weight_index, progress);
    let graphic_progress = if fading_out {
        progress * 2.0
    } else {
        (progress - 0.5) * 2.0
    }
    .clamp(0.0, 1.0);
    if let (Some(fade), CellGraphic::Glyph(glyph)) = (graphic_fade, &cell.graphic) {
        let (from, to) = if fading_out {
            (*glyph, CLEAR_TRANSITION_GLYPH)
        } else {
            (CLEAR_TRANSITION_GLYPH, *glyph)
        };
        if let Some(graphic) = fade.resolve_weighted_shape_fade(
            from,
            if fading_out {
                renderer_weight(cell.weight_index)
            } else {
                CellWeight::Zero
            },
            to,
            if fading_out {
                CellWeight::Zero
            } else {
                renderer_weight(cell.weight_index)
            },
            renderer_weight(faded.weight_index),
            graphic_progress,
        ) {
            faded.graphic = CellGraphic::Glyph(graphic);
        }
    }
    faded
}

/// One-sided-cell weight fade: the cell exists in only one keyframe, so across
/// its active half its weight runs from the authored value toward Zero (out)
/// or from Zero toward the authored value (in). `(2*progress - 1).abs()` maps
/// the active half onto 1..0..1 (full weight at the span edges, Zero at the
/// halfway crossing); the fade never overshoots the authored magnitude.
fn fade_one_sided_weight(authored: i64, progress: f32) -> i64 {
    let scaled = authored as f32 * (2.0 * progress - 1.0).abs();
    scaled.round().clamp(0.0, (authored.abs() as f32).max(0.0)) as i64 * authored.signum()
}

/// The graphic for one matched cell at `progress`. With an injected fade
/// resolver and glyph-backed cells on both sides, the renderer's gradient tour
/// picks the glyph (image-only, monotone toward the target); any fallback —
/// no resolver, sprite-backed cells, an unresolvable pair — keeps the
/// halfway hard cutoff, shaped by the eases.
fn resolve_graphic(
    from: &PaintedCell,
    to: &PaintedCell,
    progress: f32,
    from_active: bool,
    from_weight: CellWeight,
    to_weight: CellWeight,
    output_weight: CellWeight,
    graphic_fade: Option<&ShapeFade>,
) -> CellGraphic {
    let hard_cutoff = || {
        if from_active {
            from.graphic.clone()
        } else {
            to.graphic.clone()
        }
    };
    let (Some(fade), CellGraphic::Glyph(from_glyph), CellGraphic::Glyph(to_glyph)) =
        (graphic_fade, &from.graphic, &to.graphic)
    else {
        return hard_cutoff();
    };
    match fade.resolve_weighted_shape_fade(
        *from_glyph,
        from_weight,
        *to_glyph,
        to_weight,
        output_weight,
        progress,
    ) {
        Some(resolved) => CellGraphic::Glyph(resolved),
        None => hard_cutoff(),
    }
}

/// Flat RGB lerps channel-by-channel, then snaps to the indexed palette so
/// interpolated raster content never creates a color outside that system.
/// Anything else (material colors) is discrete and rides the graphic's halfway
/// cutoff.
fn blend_colors(from: PaintColor, to: PaintColor, progress: f32, from_active: bool) -> PaintColor {
    match (from, to) {
        (PaintColor::FlatRgb(r1, g1, b1), PaintColor::FlatRgb(r2, g2, b2)) => {
            let [red, green, blue] = nearest_indexed_rgb([
                lerp_channel(r1, r2, progress),
                lerp_channel(g1, g2, progress),
                lerp_channel(b1, b2, progress),
            ]);
            PaintColor::FlatRgb(red, green, blue)
        }
        (from, to) => {
            if from_active {
                from
            } else {
                to
            }
        }
    }
}

fn lerp_channel(from: u8, to: u8, progress: f32) -> u8 {
    (from as f32 + (to as f32 - from as f32) * progress)
        .round()
        .clamp(0.0, 255.0) as u8
}

fn renderer_weight(weight_index: i64) -> CellWeight {
    CellWeight::from_index_clamped(weight_index as i32)
}

fn lerp_weight(from: i64, to: i64, progress: f32) -> i64 {
    (from as f32 + (to as f32 - from as f32) * progress).round() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brush::write_cell;
    use crate::legacy_indexed_palette::legacy_indexed_palette;
    use crate::paint_color::PaintColor;
    use serde_json::json;
    use thaum_renderer_domain::shape_fade::neighbor_graph::FadeTileProvider;
    use thaum_renderer_domain::{
        CellGraphic, CellMaterialId, CellPoint, GlyphTileRaster, GLYPH_TILE_HEIGHT,
        GLYPH_TILE_WIDTH,
    };

    fn point(x: i32, y: i32) -> CellPoint {
        CellPoint { x, y, z: 0 }
    }

    fn cell(glyph: char, color: PaintColor, weight: i64) -> PaintedCell {
        PaintedCell {
            graphic: CellGraphic::Glyph(glyph),
            color,
            weight_index: weight,
        }
    }

    fn canvas_with(entries: &[(CellPoint, PaintedCell)]) -> Canvas {
        let mut canvas = Canvas::new();
        for (position, painted) in entries {
            write_cell(&mut canvas, *position, painted.clone());
        }
        canvas
    }

    fn solid(id: &str, start: u32, length: u32) -> SharedDocumentPropertyBlock {
        SharedDocumentPropertyBlock {
            id: id.to_string(),
            start_breath: start,
            length_breaths: length,
            is_blank: false,
            value: Some(json!({})),
            interpretation: None,
            ease_out_percent: None,
            ease_in_percent: None,
        }
    }

    fn blank(
        id: &str,
        start: u32,
        length: u32,
        interpretation: Option<&str>,
        ease_out: Option<u8>,
        ease_in: Option<u8>,
    ) -> SharedDocumentPropertyBlock {
        SharedDocumentPropertyBlock {
            id: id.to_string(),
            start_breath: start,
            length_breaths: length,
            is_blank: true,
            value: None,
            interpretation: interpretation.map(str::to_string),
            ease_out_percent: ease_out,
            ease_in_percent: ease_in,
        }
    }

    #[test]
    fn matched_cells_snap_interpolated_color_to_the_palette_and_cut_the_graphic_at_halfway() {
        let [from_red, from_green, from_blue] = legacy_indexed_palette()[0];
        let [to_red, to_green, to_blue] = legacy_indexed_palette()[24];
        let from = canvas_with(&[(
            point(1, 1),
            cell(
                'a',
                PaintColor::flat_rgb(from_red, from_green, from_blue),
                0,
            ),
        )]);
        let to = canvas_with(&[(
            point(1, 1),
            cell('b', PaintColor::flat_rgb(to_red, to_green, to_blue), 4),
        )]);
        // First half: previous keyframe's graphic, a palette color, and blended weight.
        let first_half = blend_canvases(&from, &to, 0.25, None);
        let blended = first_half.get(&point(1, 1)).unwrap();
        assert_eq!(blended.graphic, CellGraphic::Glyph('a'));
        assert!(
            matches!(blended.color, PaintColor::FlatRgb(red, green, blue) if legacy_indexed_palette().contains(&[red, green, blue]))
        );
        assert_eq!(blended.weight_index, 1);
        // Second half: next keyframe's graphic, still a palette color.
        let second_half = blend_canvases(&from, &to, 0.75, None);
        let blended = second_half.get(&point(1, 1)).unwrap();
        assert_eq!(blended.graphic, CellGraphic::Glyph('b'));
        assert!(
            matches!(blended.color, PaintColor::FlatRgb(red, green, blue) if legacy_indexed_palette().contains(&[red, green, blue]))
        );
        assert_eq!(blended.weight_index, 3);
        // Palette endpoints still resolve exactly.
        assert_eq!(blend_canvases(&from, &to, 0.0, None), from);
        assert_eq!(blend_canvases(&from, &to, 1.0, None), to);
    }

    #[test]
    fn one_sided_cells_show_only_during_their_side_active_half_and_fade_toward_zero_weight() {
        let from = canvas_with(&[(point(0, 0), cell('x', PaintColor::flat_rgb(1, 2, 3), 4))]);
        let to = canvas_with(&[(point(2, 2), cell('y', PaintColor::flat_rgb(4, 5, 6), 4))]);
        let first_half = blend_canvases(&from, &to, 0.25, None);
        let fading = first_half.get(&point(0, 0)).unwrap();
        assert!(
            fading.weight_index < 4 && fading.weight_index > 0,
            "from-side cell fades toward Zero early ({})",
            fading.weight_index
        );
        assert!(
            !first_half.contains_key(&point(2, 2)),
            "to-side cell hidden early"
        );
        let near_midpoint = blend_canvases(&from, &to, 0.49, None);
        assert_eq!(
            near_midpoint.get(&point(0, 0)).unwrap().weight_index,
            0,
            "the from-side cell bottoms out at Zero right before vanishing"
        );
        let second_half = blend_canvases(&from, &to, 0.75, None);
        assert!(
            !second_half.contains_key(&point(0, 0)),
            "from-side cell hidden late"
        );
        let growing = second_half.get(&point(2, 2)).unwrap();
        assert!(
            growing.weight_index < 4 && growing.weight_index > 0,
            "to-side cell grows in from Zero late ({})",
            growing.weight_index
        );
        // The ends resolve exactly.
        assert_eq!(
            blend_canvases(&from, &to, 0.0, None).get(&point(0, 0)),
            from.get(&point(0, 0))
        );
        assert_eq!(
            blend_canvases(&from, &to, 1.0, None).get(&point(2, 2)),
            to.get(&point(2, 2))
        );
    }

    struct ClearTransitionTiles;

    impl FadeTileProvider for ClearTransitionTiles {
        fn tiles(&self) -> Vec<(char, GlyphTileRaster)> {
            let mut solid = GlyphTileRaster {
                width: GLYPH_TILE_WIDTH,
                height: GLYPH_TILE_HEIGHT,
                alpha: [255; 12 * 16],
            };
            let mut clear = solid.clone();
            clear.alpha.fill(0);
            clear.alpha[0] = 32;
            solid.alpha[0] = 255;
            vec![('x', solid), (CLEAR_TRANSITION_GLYPH, clear)]
        }
    }

    #[test]
    fn one_sided_cells_walk_through_the_low_coverage_clear_transition_glyph() {
        let fade = ShapeFade::build(&ClearTransitionTiles);
        let authored = cell('x', PaintColor::flat_rgb(1, 2, 3), 4);
        let faded = fade_one_sided_cell(&authored, 0.5, true, Some(&fade));
        let starting = fade_one_sided_cell(&authored, 0.5, false, Some(&fade));
        let grown = fade_one_sided_cell(&authored, 1.0, false, Some(&fade));
        assert_eq!(faded.graphic, CellGraphic::Glyph(CLEAR_TRANSITION_GLYPH));
        assert_eq!(starting.graphic, CellGraphic::Glyph(CLEAR_TRANSITION_GLYPH));
        assert_eq!(starting.weight_index, 0);
        assert_eq!(grown.graphic, CellGraphic::Glyph('x'));
    }

    #[test]
    fn authored_blanks_count_as_absent_when_matching() {
        let from = canvas_with(&[(point(0, 0), cell('x', PaintColor::flat_rgb(1, 2, 3), 1))]);
        let to = canvas_with(&[(point(0, 0), cell(' ', PaintColor::flat_rgb(9, 9, 9), 1))]);
        // The `to` cell is an authored blank: the position resolves as from-only.
        let blended = blend_canvases(&from, &to, 0.25, None);
        assert_eq!(blended.get(&point(0, 0)), from.get(&point(0, 0)));
        let blended = blend_canvases(&from, &to, 0.75, None);
        assert!(blended.is_empty(), "authored blank erases the cell late");
    }

    #[test]
    fn material_colors_ride_the_graphic_cutoff() {
        let from = canvas_with(&[(
            point(0, 0),
            cell('a', PaintColor::material(CellMaterialId::GrayScale), 1),
        )]);
        let to = canvas_with(&[(point(0, 0), cell('b', PaintColor::flat_rgb(10, 20, 30), 1))]);
        let first_half = blend_canvases(&from, &to, 0.25, None);
        assert_eq!(
            first_half.get(&point(0, 0)).unwrap().color,
            PaintColor::material(CellMaterialId::GrayScale)
        );
        let second_half = blend_canvases(&from, &to, 0.75, None);
        assert_eq!(
            second_half.get(&point(0, 0)).unwrap().color,
            PaintColor::flat_rgb(10, 20, 30)
        );
    }

    #[test]
    fn a_solid_block_resolves_its_own_canvas() {
        let blocks = vec![solid("a", 0, 4), blank("tail", 4, 1, None, None, None)];
        let canvas = canvas_with(&[(point(1, 1), cell('a', PaintColor::flat_rgb(1, 2, 3), 1))]);
        let canvases = [("a", canvas.clone()), ("tail", Canvas::new())];
        let resolve = |breath: u32| {
            resolve_raster_canvas(
                &blocks,
                breath,
                |block| {
                    canvases
                        .iter()
                        .find(|(id, _)| *id == block.id)
                        .map(|(_, canvas)| canvas.clone())
                },
                None,
            )
        };
        assert_eq!(resolve(0), Some(canvas));
        assert_eq!(resolve(3).unwrap().len(), 1);
    }

    #[test]
    fn an_interpolating_empty_blends_across_its_span_with_eases() {
        let blocks = vec![
            solid("a", 0, 4),
            blank("b", 4, 4, None, None, None),
            solid("c", 8, 4),
            blank("tail", 12, 1, None, None, None),
        ];
        let canvas_a = canvas_with(&[(point(1, 1), cell('a', PaintColor::flat_rgb(0, 0, 0), 0))]);
        let canvas_c =
            canvas_with(&[(point(1, 1), cell('b', PaintColor::flat_rgb(80, 80, 80), 8))]);
        let canvases = [
            ("a", canvas_a),
            ("b", Canvas::new()),
            ("c", canvas_c),
            ("tail", Canvas::new()),
        ];
        let resolve = |breath: u32| {
            resolve_raster_canvas(
                &blocks,
                breath,
                |block| {
                    canvases
                        .iter()
                        .find(|(id, _)| *id == block.id)
                        .map(|(_, canvas)| canvas.clone())
                },
                None,
            )
        };
        // Same linear walk as the move channel: 4 empty breaths -> t = 1/5..4/5.
        // Breath 4 (t=0.2, first half): graphic 'a', weight 8*0.2 = 1.6 -> 2.
        let breath4 = resolve(4).unwrap();
        let cell4 = breath4.get(&point(1, 1)).unwrap();
        assert_eq!(cell4.graphic, CellGraphic::Glyph('a'));
        assert_eq!(cell4.weight_index, 2);
        assert!(
            matches!(cell4.color, PaintColor::FlatRgb(red, green, blue) if legacy_indexed_palette().contains(&[red, green, blue])),
            "interpolated color must resolve to the indexed palette"
        );
        // Breath 5 (t=0.4): weight 8*0.4 = 3.2 -> 3.
        assert_eq!(
            resolve(5).unwrap().get(&point(1, 1)).unwrap().weight_index,
            3
        );
        // Breath 7 (t=0.8, second half): graphic 'b', weight 8*0.8 = 6.4 -> 6.
        let breath7 = resolve(7).unwrap();
        let cell7 = breath7.get(&point(1, 1)).unwrap();
        assert_eq!(cell7.graphic, CellGraphic::Glyph('b'));
        assert_eq!(cell7.weight_index, 6);
        // Breath 8 lands on the next keyframe exactly.
        assert_eq!(
            resolve(8).unwrap().get(&point(1, 1)).unwrap().weight_index,
            8
        );

        // Full ease-out on the same empty: the first breath barely departs.
        let blocks = vec![
            solid("a", 0, 4),
            blank("b", 4, 4, Some("interpolate"), Some(100), None),
            solid("c", 8, 4),
            blank("tail", 12, 1, None, None, None),
        ];
        let resolve = |breath: u32| {
            resolve_raster_canvas(
                &blocks,
                breath,
                |block| {
                    canvases
                        .iter()
                        .find(|(id, _)| *id == block.id)
                        .map(|(_, canvas)| canvas.clone())
                },
                None,
            )
        };
        let eased = resolve(4).unwrap().get(&point(1, 1)).unwrap().weight_index;
        assert!(
            eased < 2,
            "ease-out must start slower than linear ({eased} < 2)"
        );
    }

    #[test]
    fn an_interior_smear_empty_resolves_transported_cell_trails() {
        let blocks = vec![
            solid("a", 0, 4),
            blank("b", 4, 4, Some("smear"), None, None),
            solid("c", 8, 4),
            blank("tail", 12, 1, None, None, None),
        ];
        let source = canvas_with(&[(point(0, 0), cell('x', PaintColor::flat_rgb(1, 2, 3), 4))]);
        let target = canvas_with(&[(point(4, 0), cell('x', PaintColor::flat_rgb(1, 2, 3), 4))]);
        let canvases = [
            ("a", source.clone()),
            ("b", Canvas::new()),
            ("c", target.clone()),
            ("tail", Canvas::new()),
        ];
        let resolve = |breath: u32| {
            resolve_raster_canvas(
                &blocks,
                breath,
                |block| {
                    canvases
                        .iter()
                        .find(|(id, _)| *id == block.id)
                        .map(|(_, canvas)| canvas.clone())
                },
                None,
            )
        };

        let smeared = resolve(6).unwrap();
        assert!(smeared.contains_key(&point(0, 0)), "visible trail");
        assert!(smeared.contains_key(&point(1, 0)), "discrete trail cell");
        assert!(smeared.contains_key(&point(2, 0)), "transported head");
        assert_eq!(resolve(0), Some(source));
        assert_eq!(resolve(8), Some(target));
    }

    #[test]
    fn hold_and_one_sided_interpolates_hold_the_existing_side() {
        // Hold carries the previous keyframe's canvas through the empty.
        let blocks = vec![
            solid("a", 0, 4),
            blank("b", 4, 4, Some("hold"), None, None),
            solid("c", 8, 4),
            blank("tail", 12, 1, None, None, None),
        ];
        let canvas_a = canvas_with(&[(point(0, 0), cell('a', PaintColor::flat_rgb(1, 1, 1), 1))]);
        let canvases = [
            ("a", canvas_a.clone()),
            ("b", Canvas::new()),
            ("c", Canvas::new()),
            ("tail", Canvas::new()),
        ];
        let resolve = |blocks: &[SharedDocumentPropertyBlock], breath: u32| {
            resolve_raster_canvas(
                blocks,
                breath,
                |block| {
                    canvases
                        .iter()
                        .find(|(id, _)| *id == block.id)
                        .map(|(_, canvas)| canvas.clone())
                },
                None,
            )
        };
        assert_eq!(resolve(&blocks, 6), Some(canvas_a.clone()));
        // A leading empty with only a next solid holds it (nothing behind): the
        // held side is solid "c"'s canvas (empty here), not a blend.
        let blocks = vec![
            blank("lead", 0, 2, None, None, None),
            solid("c", 2, 4),
            blank("tail", 6, 1, None, None, None),
        ];
        assert_eq!(resolve(&blocks, 0), Some(Canvas::new()));
        assert_eq!(resolve(&blocks, 1), Some(Canvas::new()));
    }

    #[test]
    fn an_interpolating_empty_with_only_a_previous_side_holds_it_through_the_tail() {
        // Matching the move channel: a missing side degrades to holding the
        // existing side, so the last keyframe persists through the trailing
        // blank instead of the layer dropping to nothing after it.
        let blocks = vec![solid("a", 0, 4), blank("tail", 4, 2, None, None, None)];
        let canvas_a = canvas_with(&[(point(0, 0), cell('a', PaintColor::flat_rgb(1, 1, 1), 1))]);
        let canvases = [("a", canvas_a.clone()), ("tail", Canvas::new())];
        let resolve = |breath: u32| {
            resolve_raster_canvas(
                &blocks,
                breath,
                |block| {
                    canvases
                        .iter()
                        .find(|(id, _)| *id == block.id)
                        .map(|(_, canvas)| canvas.clone())
                },
                None,
            )
        };
        assert_eq!(resolve(4), Some(canvas_a.clone()));
        assert_eq!(resolve(5), Some(canvas_a));
    }

    #[test]
    fn loop_out_replays_the_authored_region_through_the_trailing_blank() {
        let blocks = vec![
            solid("a", 0, 4),
            blank("b", 4, 4, Some("hold"), None, None),
            solid("c", 8, 4),
            blank("tail", 12, 4, Some("loop_out"), None, None),
        ];
        let canvas_a = canvas_with(&[(point(0, 0), cell('a', PaintColor::flat_rgb(1, 1, 1), 1))]);
        let canvases = [
            ("a", canvas_a.clone()),
            ("b", Canvas::new()),
            ("c", Canvas::new()),
            ("tail", Canvas::new()),
        ];
        let resolve = |breath: u32| {
            resolve_raster_canvas(
                &blocks,
                breath,
                |block| {
                    canvases
                        .iter()
                        .find(|(id, _)| *id == block.id)
                        .map(|(_, canvas)| canvas.clone())
                },
                None,
            )
        };
        // Region 0..12: breath 12 wraps to 0 (solid a), 16 wraps into the hold
        // blank (also a), 20 wraps to solid c.
        assert_eq!(resolve(12), Some(canvas_a.clone()));
        assert_eq!(resolve(16), Some(canvas_a.clone()));
        assert_eq!(resolve(20).unwrap().len(), 0);
        // Far past the stored extent the loop keeps playing.
        assert_eq!(resolve(24), Some(canvas_a));
    }

    #[test]
    fn a_track_with_no_solids_resolves_nothing() {
        let blocks = vec![blank("tail", 0, 24, None, None, None)];
        assert_eq!(
            resolve_raster_canvas(&blocks, 5, |_| Some(Canvas::new()), None),
            None
        );
        assert_eq!(
            resolve_raster_canvas(&[], 0, |_| Some(Canvas::new()), None),
            None
        );
    }
}
