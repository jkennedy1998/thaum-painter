//! Discrete raster smear interpolation. This module owns correspondence and
//! swept-cell trails only; `interp_raster` remains the channel dispatcher and
//! owns the shared appearance blend used for each emitted sample.

use std::collections::{BTreeMap, BTreeSet};

use thaum_renderer_domain::shape_fade::fade::ShapeFade;
use thaum_renderer_domain::{CellGraphic, CellPoint};

use crate::brush::{effective_cell, Canvas, PaintedCell};
use crate::interp_raster::{blend_canvases, blend_cells_with_weight_scale};

/// Maximum Manhattan displacement considered by the conservative first-pass
/// matcher. Larger movement degrades to the ordinary one-sided raster fade,
/// rather than guessing a long, identity-breaking trail.
const MAX_MATCH_DISTANCE: u32 = 8;
const MIN_MATCH_SCORE: i32 = 64;

#[derive(Clone)]
struct Correspondence {
    source: CellPoint,
    target: CellPoint,
    score: i32,
}

#[derive(Clone)]
struct Splat {
    cell: PaintedCell,
    /// A transported cell at its current position, or a source/target fallback,
    /// outranks an inferred trail sample.
    is_head: bool,
    score: i32,
    distance_to_head: u32,
}

/// Resolves a visible cell-trail transition from `from` to `to`. It first
/// accepts only bounded, one-to-one correspondences with compatible cell
/// evidence, then sweeps each accepted cell over a discrete 3D line. A trail
/// peaks at mid-transition and collapses precisely onto either endpoint.
///
/// Unmatched cells are intentionally delegated to ordinary raster blending:
/// disappearances/disocclusions fade instead of receiving a speculative trail.
pub fn smear_canvases(
    from: &Canvas,
    to: &Canvas,
    progress: f32,
    graphic_fade: Option<&ShapeFade>,
) -> Canvas {
    let progress = progress.clamp(0.0, 1.0);
    // Keyframe breaths must preserve every authored cell exactly, including a
    // legacy flat RGB value that is not one of the indexed palette entries.
    if progress <= 0.0 {
        return from.clone();
    }
    if progress >= 1.0 {
        return to.clone();
    }
    let correspondences = correspondences(from, to);

    // First resolve only unmatched content with the established one-sided
    // fallback. It participates as a head, so a speculative trail cannot cover
    // a real appearing/disappearing cell.
    let mut unmatched_from = from.clone();
    let mut unmatched_to = to.clone();
    for pair in &correspondences {
        unmatched_from.remove(&pair.source);
        unmatched_to.remove(&pair.target);
    }
    let mut splats: BTreeMap<CellPoint, Splat> =
        blend_canvases(&unmatched_from, &unmatched_to, progress, graphic_fade)
            .into_iter()
            .map(|(position, cell)| {
                (
                    position,
                    Splat {
                        cell,
                        is_head: true,
                        score: 0,
                        distance_to_head: 0,
                    },
                )
            })
            .collect();

    let exposure_start = (progress - trail_window(progress)).max(0.0);
    for pair in correspondences {
        let Some(from_cell) = effective_cell(from.get(&pair.source)) else {
            continue;
        };
        let Some(to_cell) = effective_cell(to.get(&pair.target)) else {
            continue;
        };
        let path = swept_path(pair.source, pair.target, exposure_start, progress);
        let path_len = path.len().saturating_sub(1) as u32;
        for (index, (position, local_progress)) in path.into_iter().enumerate() {
            let distance_to_head = path_len.saturating_sub(index as u32);
            let is_head = distance_to_head == 0;
            // Keep the tail visibly present but subordinate to the head. The
            // weight scale is applied *before* shape-fade selection so its glyph
            // exists at the weight the renderer will draw.
            let weight_scale = if path_len == 0 {
                1.0
            } else {
                0.35 + 0.65 * (index as f32 / path_len as f32)
            };
            let cell = blend_cells_with_weight_scale(
                from_cell,
                to_cell,
                local_progress,
                local_progress < 0.5,
                weight_scale,
                graphic_fade,
            );
            insert_splat(
                &mut splats,
                position,
                Splat {
                    cell,
                    is_head,
                    score: pair.score,
                    distance_to_head,
                },
            );
        }
    }

    splats
        .into_iter()
        .map(|(position, splat)| (position, splat.cell))
        .collect()
}

/// A rounded 3D DDA line over the visible segment of one correspondence. The
/// output is deduplicated because shallow movement can round adjacent samples
/// to the same cell. The final sample always carries the true current progress
/// (rather than its rounded line-step progress) so glyph/color/weight land
/// cleanly at the authored endpoints.
fn swept_path(
    source: CellPoint,
    target: CellPoint,
    start_progress: f32,
    head_progress: f32,
) -> Vec<(CellPoint, f32)> {
    let dx = target.x - source.x;
    let dy = target.y - source.y;
    let dz = target.z - source.z;
    let steps = dx.abs().max(dy.abs()).max(dz.abs());
    if steps == 0 {
        return vec![(source, head_progress)];
    }
    let start_step =
        ((start_progress.clamp(0.0, 1.0) * steps as f32).round() as i32).clamp(0, steps);
    let head_step =
        ((head_progress.clamp(0.0, 1.0) * steps as f32).round() as i32).clamp(start_step, steps);
    let mut path = Vec::new();
    let mut seen = BTreeSet::new();
    for step in start_step..=head_step {
        let ratio = step as f32 / steps as f32;
        let position = CellPoint {
            x: source.x + (dx as f32 * ratio).round() as i32,
            y: source.y + (dy as f32 * ratio).round() as i32,
            z: source.z + (dz as f32 * ratio).round() as i32,
        };
        if seen.insert(position) {
            let local_progress = if step == head_step {
                head_progress
            } else {
                ratio
            };
            path.push((position, local_progress));
        }
    }
    path
}

/// The trail is half the full correspondence path at its largest, with a
/// smooth zero-to-zero envelope. The timeline already supplied eased progress,
/// so its authored ease also shapes the visible smear length.
fn trail_window(progress: f32) -> f32 {
    0.5 * (4.0 * progress * (1.0 - progress)).clamp(0.0, 1.0)
}

fn correspondences(from: &Canvas, to: &Canvas) -> Vec<Correspondence> {
    let mut candidates = Vec::new();
    for (source, from_cell) in from {
        let Some(from_cell) = effective_cell(Some(from_cell)) else {
            continue;
        };
        for (target, to_cell) in to {
            let Some(to_cell) = effective_cell(Some(to_cell)) else {
                continue;
            };
            let Some(score) = match_score(*source, from_cell, *target, to_cell) else {
                continue;
            };
            candidates.push(Correspondence {
                source: *source,
                target: *target,
                score,
            });
        }
    }
    // A stable global order makes the greedy one-to-one assignment scrub-safe.
    candidates.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.source.cmp(&right.source))
            .then_with(|| left.target.cmp(&right.target))
    });

    let mut used_sources = BTreeSet::new();
    let mut used_targets = BTreeSet::new();
    candidates
        .into_iter()
        .filter(|candidate| {
            candidate.score >= MIN_MATCH_SCORE
                && used_sources.insert(candidate.source)
                && used_targets.insert(candidate.target)
        })
        .collect()
}

/// Compatibility scoring is deliberately small and conservative for the first
/// pass. Exact graphics are strongest; changed glyphs are allowed only when
/// color/weight evidence makes the same moving detail plausible. Once matched,
/// the renderer ShapeFade resolver—not this scorer—chooses visual glyph steps.
fn match_score(
    source: CellPoint,
    from: &PaintedCell,
    target: CellPoint,
    to: &PaintedCell,
) -> Option<i32> {
    let distance = manhattan_distance(source, target);
    if distance > MAX_MATCH_DISTANCE {
        return None;
    }
    let graphic_score = match (&from.graphic, &to.graphic) {
        (CellGraphic::Glyph(left), CellGraphic::Glyph(right)) if left == right => 80,
        (CellGraphic::Glyph(_), CellGraphic::Glyph(_)) => 40,
        (left, right) if left == right => 80,
        // Sprite identity does not have a cross-sprite shape tour yet, so
        // different sprites are not legitimate first-pass correspondences.
        _ => return None,
    };
    let color_score = if from.color == to.color { 32 } else { 0 };
    let weight_delta = from.weight_index.abs_diff(to.weight_index).min(24) as i32;
    let weight_score = 24 - weight_delta;
    Some(graphic_score + color_score + weight_score - distance as i32)
}

fn manhattan_distance(left: CellPoint, right: CellPoint) -> u32 {
    left.x.abs_diff(right.x) + left.y.abs_diff(right.y) + left.z.abs_diff(right.z)
}

fn insert_splat(output: &mut BTreeMap<CellPoint, Splat>, position: CellPoint, candidate: Splat) {
    let replace = output
        .get(&position)
        .map(|current| {
            (
                candidate.is_head,
                candidate.score,
                std::cmp::Reverse(candidate.distance_to_head),
            ) > (
                current.is_head,
                current.score,
                std::cmp::Reverse(current.distance_to_head),
            )
        })
        .unwrap_or(true);
    if replace {
        output.insert(position, candidate);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brush::write_cell;
    use crate::paint_color::PaintColor;

    fn point(x: i32, y: i32) -> CellPoint {
        CellPoint { x, y, z: 0 }
    }

    fn cell(glyph: char, color: PaintColor, weight: i64) -> PaintedCell {
        PaintedCell {
            graphic: CellGraphic::Glyph(glyph),
            color,
            weight_index: weight,
            shader_stack: Vec::new(),
        }
    }

    fn canvas(entries: &[(CellPoint, PaintedCell)]) -> Canvas {
        let mut canvas = Canvas::new();
        for (position, cell) in entries {
            write_cell(&mut canvas, *position, cell.clone());
        }
        canvas
    }

    #[test]
    fn translation_emits_a_weight_tapered_cell_trail_and_lands_exactly_on_endpoints() {
        let source_cell = cell('x', PaintColor::flat_rgb(4, 5, 6), 4);
        let from = canvas(&[(point(0, 0), source_cell.clone())]);
        let to = canvas(&[(point(4, 0), source_cell.clone())]);

        assert_eq!(smear_canvases(&from, &to, 0.0, None), from);
        assert_eq!(smear_canvases(&from, &to, 1.0, None), to);

        let midpoint = smear_canvases(&from, &to, 0.5, None);
        assert!(
            midpoint.contains_key(&point(0, 0)),
            "trail starts at source"
        );
        assert!(midpoint.contains_key(&point(1, 0)), "trail is discrete");
        assert!(midpoint.contains_key(&point(2, 0)), "head is transported");
        assert_eq!(midpoint.get(&point(2, 0)).unwrap().weight_index, 4);
        assert!(midpoint.get(&point(0, 0)).unwrap().weight_index < 4);
    }

    #[test]
    fn low_evidence_pairs_remain_unmatched_and_use_the_existing_half_span_fade() {
        let from = canvas(&[(point(0, 0), cell('a', PaintColor::flat_rgb(1, 1, 1), 4))]);
        let to = canvas(&[(point(4, 0), cell('b', PaintColor::flat_rgb(9, 9, 9), 4))]);
        let early = smear_canvases(&from, &to, 0.25, None);
        assert!(early.contains_key(&point(0, 0)));
        assert!(!early.contains_key(&point(4, 0)));
        let late = smear_canvases(&from, &to, 0.75, None);
        assert!(!late.contains_key(&point(0, 0)));
        assert!(late.contains_key(&point(4, 0)));
    }

    #[test]
    fn heads_win_over_crossing_trails() {
        let head = cell('h', PaintColor::flat_rgb(1, 1, 1), 4);
        let trail = cell('t', PaintColor::flat_rgb(2, 2, 2), 1);
        let mut output = BTreeMap::new();
        insert_splat(
            &mut output,
            point(1, 1),
            Splat {
                cell: head.clone(),
                is_head: true,
                score: 10,
                distance_to_head: 0,
            },
        );
        insert_splat(
            &mut output,
            point(1, 1),
            Splat {
                cell: trail,
                is_head: false,
                score: 999,
                distance_to_head: 0,
            },
        );
        assert_eq!(output.get(&point(1, 1)).unwrap().cell, head);
    }
}
