//! Live detailer stroke accumulation and resolution (project 6kbosu52).
//! A detailer drag paints ink into subcell comparison-resolution buffers,
//! one per cell the pointer crosses (J 2026-11-09: "it needs to work so the
//! user can draw across multiple cells potentially effecting them ... needs
//! to work within the same stroke"). Each buffer is seeded from its cell's
//! currently authored glyph (or blank, for an additive press on an empty
//! cell); release commits whatever glyph each changed buffer resolves to.
//! Color and weight are never touched by a stroke — out of scope for the
//! interaction entirely, not merely lock-gated (J 2026-11-08).

use std::collections::BTreeMap;

use thaum_renderer_domain::shape_fade::fade::ShapeFade;
use thaum_renderer_domain::{CellPoint, CellWeight, GlyphTileRaster};

pub use crate::tool_state::DetailerMode;
use crate::tool_state::PaintHand;

/// One in-progress detailer stroke: the acting hand, the cell being
/// detailed, the render weight the buffer resolves against (captured at
/// stroke start so a weight change elsewhere never retargets the
/// comparison graph mid-stroke), and the live subcell ink buffer itself.
/// One drag can hold one of these per cell it crosses.
#[derive(Debug, Clone)]
pub struct DetailerStroke {
    pub hand: PaintHand,
    pub position: CellPoint,
    pub mode: DetailerMode,
    pub weight: CellWeight,
    buffer_width: usize,
    buffer_height: usize,
    /// Ink amount per subcell pixel (row-major), seeded from the cell's
    /// current authored glyph's own comparison tile.
    buffer: Vec<u8>,
    /// True once a paint sample actually moved at least one buffer pixel
    /// off its seed value. A stroke that never moved ink commits nothing.
    changed: bool,
}

impl DetailerStroke {
    /// Begins a stroke seeded from `seed` — the detailed cell's current
    /// glyph, already at comparison resolution (see
    /// `thaum_renderer_domain::shape_fade::compare_tile`). Additive/
    /// subtractive edit what's already there, never a blank canvas.
    pub fn begin(
        hand: PaintHand,
        position: CellPoint,
        mode: DetailerMode,
        weight: CellWeight,
        seed: GlyphTileRaster,
    ) -> Self {
        Self {
            hand,
            position,
            mode,
            weight,
            buffer_width: seed.width,
            buffer_height: seed.height,
            buffer: seed.alpha,
            changed: false,
        }
    }

    pub fn buffer_dimensions(&self) -> (usize, usize) {
        (self.buffer_width, self.buffer_height)
    }

    /// Paints ink at one subcell pixel: additive raises it toward full ink,
    /// subtractive lowers it toward none, both by `strength`, saturating at
    /// the ink range's ends. An out-of-bounds coordinate is a no-op.
    pub fn paint(&mut self, x: usize, y: usize, strength: u8) {
        if x >= self.buffer_width || y >= self.buffer_height {
            return;
        }
        let index = y * self.buffer_width + x;
        let before = self.buffer[index];
        self.buffer[index] = match self.mode {
            DetailerMode::Additive => before.saturating_add(strength),
            DetailerMode::Subtractive => before.saturating_sub(strength),
        };
        if self.buffer[index] != before {
            self.changed = true;
        }
    }

    /// Paints the brush footprint: a square of `size` subcell pixels
    /// centered on the sample, the same square shape the cell brush draws
    /// at size 1..=5, scaled down into the buffer. Size 1 is the single
    /// sample pixel; the footprint simply clips at the buffer's edges
    /// (J 2026-11-09: "maybe size for this tool could effect the sub pixels
    /// that it is effecting").
    pub fn paint_footprint(&mut self, x: usize, y: usize, size: i32, strength: u8) {
        let size = size.clamp(1, 5);
        let left_radius = (size - 1) / 2;
        let right_radius = size - 1 - left_radius;
        for dy in -left_radius..=right_radius {
            for dx in -left_radius..=right_radius {
                let sample_x = x as i64 + dx as i64;
                let sample_y = y as i64 + dy as i64;
                if sample_x < 0 || sample_y < 0 {
                    continue;
                }
                self.paint(sample_x as usize, sample_y as usize, strength);
            }
        }
    }

    /// Whether any paint sample moved this stroke's buffer off its seed.
    pub fn changed(&self) -> bool {
        self.changed
    }

    /// The live buffer as a comparison-resolution tile, for resolving a
    /// preview or the final commit.
    pub fn tile(&self) -> GlyphTileRaster {
        GlyphTileRaster {
            width: self.buffer_width,
            height: self.buffer_height,
            alpha: self.buffer.clone(),
        }
    }

    /// Resolves the current buffer to the nearest loaded glyph within this
    /// stroke's captured weight — the live preview, and (read once more at
    /// release) the committed result. Always returns the nearest match
    /// regardless of distance: landing back on the cell's own
    /// already-authored glyph is a valid, expected outcome (J 2026-11-08:
    /// "ideally snap to nearest which might be the cell that is currently
    /// authored and that's ok").
    pub fn resolve(&self, fade: &ShapeFade) -> Option<char> {
        // Additive strokes resolve against inked glyphs only (J 2026-11-09:
        // the GUI resolved sparse additive marks to the invisible space
        // glyph, so drawn strokes authored "nothing"). Adding ink can never
        // legitimately produce the blank glyph; subtracting ink to nothing
        // is a valid full erase, so subtract keeps the blank candidate.
        match self.mode {
            DetailerMode::Additive => {
                fade.nearest_inked_glyph_at_weight(&self.tile(), self.weight)
            }
            DetailerMode::Subtractive => {
                fade.nearest_glyph_at_weight(&self.tile(), self.weight)
            }
        }
        .map(|(glyph, _distance)| glyph)
    }
}

/// The one graphic-only write requested when an active detail stroke ends.
/// The canvas-pointer seam consumes it in phase 6; the detail grid never
/// owns a document or performs this write itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DetailerCommit {
    pub hand: PaintHand,
    pub position: CellPoint,
    pub graphic: char,
}

/// Shared live state between the canvas target selector, detail grid, and
/// canvas preview. The grid changes only the stroke buffers through this
/// type; it cannot know or mutate document appearance beyond the pending
/// glyphs. One drag holds one stroke per cell it crossed, each seeded
/// independently; release turns every changed buffer into one queued
/// graphic-only commit.
#[derive(Debug, Default)]
pub struct DetailerSession {
    active: Option<PaintHand>,
    /// Add/erase captured at press. Cells the drag enters later seed with
    /// this mode, so a property flip mid-stroke can never retarget the live
    /// drag — it applies from the next press onward.
    stroke_mode: Option<DetailerMode>,
    /// One in-progress buffer per cell the stroke has crossed, keyed by
    /// that cell. Re-entering a cell keeps its live buffer.
    strokes: BTreeMap<CellPoint, DetailerStroke>,
    /// Each stroke's live resolved glyph, for preview and release commit.
    pending: BTreeMap<CellPoint, char>,
    /// Queued graphic-only writes from the last release, keyed by cell.
    pending_commits: BTreeMap<CellPoint, DetailerCommit>,
}

impl DetailerSession {
    /// Begins (or re-enters) one cell of the active multi-cell stroke and
    /// clears unconsumed prior releases. `resolve` is supplied by the caller
    /// because the live typeface fade is orchestration-owned and can change
    /// when the file changes its face. A cell already holding a live buffer
    /// keeps it: dragging back across a cell continues its edit instead of
    /// resetting it.
    pub fn begin(
        &mut self,
        stroke: DetailerStroke,
        resolve: impl FnOnce(&DetailerStroke) -> Option<char>,
    ) {
        if self.active.is_none() {
            self.active = Some(stroke.hand);
            self.stroke_mode = Some(stroke.mode);
        }
        let resolved = resolve(&stroke);
        let position = stroke.position;
        if let Some(graphic) = resolved {
            self.pending.insert(position, graphic);
        }
        self.strokes.entry(position).or_insert(stroke);
    }

    pub fn is_active(&self) -> bool {
        self.active.is_some()
    }

    pub fn active_hand(&self) -> Option<PaintHand> {
        self.active
    }

    /// The add/erase mode captured when this drag began. Cells entered
    /// mid-stroke seed with it rather than the hand's current property.
    pub fn stroke_mode(&self) -> Option<DetailerMode> {
        self.stroke_mode
    }

    pub fn has_stroke_at(&self, position: &CellPoint) -> bool {
        self.strokes.contains_key(position)
    }

    pub fn pending_graphic_at(&self, position: &CellPoint) -> Option<char> {
        self.pending.get(position).copied()
    }

    /// Every changed cell's live candidate: exactly what release would
    /// write, one (position, glyph) per crossed cell whose buffer actually
    /// moved ink and resolved. Untouched cells preview nothing — a buffer
    /// still at its seed has nothing to show yet.
    pub fn pending_previews(&self) -> Vec<(CellPoint, char)> {
        self.pending
            .iter()
            .filter(|(position, _)| {
                self.strokes
                    .get(position)
                    .is_some_and(|stroke| stroke.changed())
            })
            .map(|(p, g)| (*p, *g))
            .collect()
    }

    /// Paints one comparison pixel in `position`'s buffer and refreshes that
    /// cell's canvas preview candidate. Calls naming an uncrossed cell are a
    /// no-op.
    pub fn paint_at(
        &mut self,
        position: CellPoint,
        x: usize,
        y: usize,
        strength: u8,
        resolve: impl Fn(&DetailerStroke) -> Option<char>,
    ) {
        self.paint_at_cell_local(
            position,
            (x as f32 + 0.5) / self.strokes.get(&position).map(|s| s.buffer_width).unwrap_or(1) as f32,
            (y as f32 + 0.5) / self.strokes.get(&position).map(|s| s.buffer_height).unwrap_or(1) as f32,
            1,
            strength,
            resolve,
        );
    }

    /// Paints the brush footprint at the direct canvas pointer's position
    /// inside `position`'s cell. The pointer is normalized to that cell and
    /// then mapped to its typeface tile's own dimensions; no fixed UI grid
    /// participates.
    pub fn paint_at_cell_local(
        &mut self,
        position: CellPoint,
        local_x: f32,
        local_y: f32,
        brush_size: i32,
        strength: u8,
        resolve: impl Fn(&DetailerStroke) -> Option<char>,
    ) {
        let (x, y) = {
            let Some(stroke) = self.strokes.get(&position) else {
                return;
            };
            (
                (local_x.clamp(0.0, 1.0 - f32::EPSILON) * stroke.buffer_width as f32) as usize,
                (local_y.clamp(0.0, 1.0 - f32::EPSILON) * stroke.buffer_height as f32) as usize,
            )
        };
        let resolved = {
            let Some(stroke) = self.strokes.get_mut(&position) else {
                return;
            };
            stroke.paint_footprint(x, y, brush_size, strength);
            resolve(stroke)
        };
        match resolved {
            Some(graphic) => {
                let previous = self.pending.insert(position, graphic);
                if previous != resolved {
                    eprintln!(
                        "[detailer-debug] paint at {position:?} ({x},{y}) brush {brush_size} -> resolve {graphic:?} (was {previous:?})",
                    );
                }
            }
            None => {
                self.pending.remove(&position);
            }
        }
    }

    /// Ends the active direct-canvas stroke for `hand`. Another hand's
    /// release cannot commit or cancel it.
    pub fn finish_for_hand(&mut self, hand: PaintHand) {
        if self.active_hand() == Some(hand) {
            self.finish();
        }
    }

    /// Ends the active direct-canvas drag. Every cell whose buffer actually
    /// moved off its seed queues one graphic-only commit; buffers the drag
    /// never changed (a fast click, a re-crossed cell) queue nothing, and a
    /// buffer with no resolved glyph never writes.
    pub fn finish(&mut self) {
        let strokes = std::mem::take(&mut self.strokes);
        for (position, stroke) in strokes {
            let resolved = self.pending.get(&position).copied();
            eprintln!(
                "[detailer-debug] finish at {position:?} changed={} -> commit {resolved:?}",
                stroke.changed()
            );
            if let (true, Some(graphic)) = (stroke.changed(), resolved) {
                self.pending_commits.insert(
                    position,
                    DetailerCommit {
                        hand: stroke.hand,
                        position,
                        graphic,
                    },
                );
            }
        }
        self.pending.clear();
        self.active = None;
        self.stroke_mode = None;
    }

    /// Cancels the active stroke and any previously queued, unconsumed write.
    pub fn cancel(&mut self) {
        self.strokes.clear();
        self.pending.clear();
        self.active = None;
        self.stroke_mode = None;
        self.pending_commits.clear();
    }

    /// Transfers the release's queued writes to the document-owning
    /// consumer, one per changed cell, in cell order.
    pub fn take_pending_commits(&mut self) -> Vec<DetailerCommit> {
        std::mem::take(&mut self.pending_commits)
            .into_values()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use thaum_renderer_domain::shape_fade::neighbor_graph::FadeTileProvider;

    const BUFFER_WIDTH: usize = 4;
    const BUFFER_HEIGHT: usize = 4;
    const BUFFER_PIXELS: usize = BUFFER_WIDTH * BUFFER_HEIGHT;

    fn flat_tile(value: u8) -> GlyphTileRaster {
        GlyphTileRaster {
            width: BUFFER_WIDTH,
            height: BUFFER_HEIGHT,
            alpha: vec![value; BUFFER_PIXELS],
        }
    }

    fn point() -> CellPoint {
        CellPoint { x: 1, y: 1, z: 0 }
    }

    #[test]
    fn begin_seeds_the_buffer_from_the_current_glyphs_tile() {
        let mut seed = flat_tile(0);
        seed.alpha[5] = 200;
        let stroke = DetailerStroke::begin(
            PaintHand::Left,
            point(),
            DetailerMode::Additive,
            CellWeight::ONE,
            seed.clone(),
        );
        assert_eq!(stroke.tile(), seed);
        assert_eq!(stroke.buffer_dimensions(), (BUFFER_WIDTH, BUFFER_HEIGHT));
    }

    #[test]
    fn additive_raises_ink_and_saturates_at_full_coverage() {
        let mut stroke = DetailerStroke::begin(
            PaintHand::Left,
            point(),
            DetailerMode::Additive,
            CellWeight::ONE,
            flat_tile(200),
        );
        stroke.paint(0, 0, 40);
        assert_eq!(stroke.tile().alpha[0], 240);
        stroke.paint(0, 0, 40);
        assert_eq!(stroke.tile().alpha[0], 255, "saturates, never wraps");
    }

    #[test]
    fn subtractive_lowers_ink_and_saturates_at_no_coverage() {
        let mut stroke = DetailerStroke::begin(
            PaintHand::Left,
            point(),
            DetailerMode::Subtractive,
            CellWeight::ONE,
            flat_tile(20),
        );
        stroke.paint(0, 0, 15);
        assert_eq!(stroke.tile().alpha[0], 5);
        stroke.paint(0, 0, 15);
        assert_eq!(stroke.tile().alpha[0], 0, "saturates, never wraps");
    }

    #[test]
    fn painting_out_of_bounds_is_a_no_op() {
        let mut stroke = DetailerStroke::begin(
            PaintHand::Left,
            point(),
            DetailerMode::Additive,
            CellWeight::ONE,
            flat_tile(0),
        );
        stroke.paint(BUFFER_WIDTH, 0, 255);
        stroke.paint(0, BUFFER_HEIGHT, 255);
        assert_eq!(stroke.tile(), flat_tile(0));
    }

    struct TwoGlyphs;
    impl FadeTileProvider for TwoGlyphs {
        fn tiles(&self) -> Vec<(char, GlyphTileRaster)> {
            vec![('#', flat_tile(255)), (' ', flat_tile(0))]
        }
    }

    #[test]
    fn additive_resolve_never_lands_on_the_blank_glyph() {
        let fade = ShapeFade::build_weighted(&SingleWeight);
        let mut stroke = DetailerStroke::begin(
            PaintHand::Left,
            point(),
            DetailerMode::Additive,
            CellWeight::ONE,
            flat_tile(0),
        );
        assert_eq!(
            stroke.resolve(&fade),
            Some('#'),
            "additive resolves against inked candidates only: the blank tile is every sparse mark's nearest neighbor and would make drawn strokes resolve to the invisible space glyph"
        );

        for y in 0..BUFFER_HEIGHT {
            for x in 0..BUFFER_WIDTH {
                stroke.paint(x, y, 255);
            }
        }
        assert_eq!(
            stroke.resolve(&fade),
            Some('#'),
            "fully inked buffer resolves to the full-coverage glyph"
        );
    }

    #[test]
    fn subtractive_resolve_keeps_the_blank_glyph_for_full_erases() {
        let fade = ShapeFade::build_weighted(&SingleWeight);
        let mut stroke = DetailerStroke::begin(
            PaintHand::Left,
            point(),
            DetailerMode::Subtractive,
            CellWeight::ONE,
            flat_tile(255),
        );
        assert_eq!(stroke.resolve(&fade), Some('#'));

        for y in 0..BUFFER_HEIGHT {
            for x in 0..BUFFER_WIDTH {
                stroke.paint(x, y, 255);
            }
        }
        assert_eq!(
            stroke.resolve(&fade),
            Some(' '),
            "erasing everything is a valid full erase — subtract keeps the blank candidate"
        );
    }

    struct SingleWeight;
    impl thaum_renderer_domain::shape_fade::neighbor_graph::WeightedFadeTileProvider for SingleWeight {
        fn weight_count(&self) -> usize {
            1
        }
        fn tiles_at_weight(&self, _weight: CellWeight) -> Vec<(char, GlyphTileRaster)> {
            TwoGlyphs.tiles()
        }
    }

    #[test]
    fn resolve_stays_scoped_to_the_strokes_captured_weight() {
        // Weight ONE only has 'm'; weight ZERO only has 'A'/'B'. A stroke
        // captured at ZERO must never resolve to 'm', even if ONE's graph
        // would consider it closer.
        struct DivergentWeights;
        impl thaum_renderer_domain::shape_fade::neighbor_graph::WeightedFadeTileProvider
            for DivergentWeights
        {
            fn weight_count(&self) -> usize {
                2
            }
            fn tiles_at_weight(&self, weight: CellWeight) -> Vec<(char, GlyphTileRaster)> {
                match weight {
                    CellWeight::ONE => vec![('m', flat_tile(128))],
                    _ => vec![('A', flat_tile(255)), ('B', flat_tile(0))],
                }
            }
        }
        let fade = ShapeFade::build_weighted(&DivergentWeights);
        let stroke = DetailerStroke::begin(
            PaintHand::Left,
            point(),
            DetailerMode::Additive,
            CellWeight::ZERO,
            flat_tile(128),
        );
        let resolved = stroke.resolve(&fade).unwrap();
        assert_ne!(resolved, 'm', "weight ZERO's graph has no 'm' node to find");
    }

    #[test]
    fn session_updates_its_pending_glyph_and_queues_one_graphic_only_commit() {
        let mut session = DetailerSession::default();
        session.begin(
            DetailerStroke::begin(
                PaintHand::Left,
                point(),
                DetailerMode::Additive,
                CellWeight::ONE,
                flat_tile(0),
            ),
            |_| Some('.'),
        );
        assert_eq!(session.pending_graphic_at(&point()), Some('.'));
        assert_eq!(session.active_hand(), Some(PaintHand::Left));

        session.paint_at(point(), 0, 0, 255, |stroke| {
            (stroke.tile().alpha[0] == 255).then_some('#')
        });
        assert_eq!(session.pending_graphic_at(&point()), Some('#'));

        session.finish();
        assert!(!session.is_active());
        assert_eq!(session.pending_graphic_at(&point()), None);
        assert_eq!(
            session.take_pending_commits(),
            vec![DetailerCommit {
                hand: PaintHand::Left,
                position: point(),
                graphic: '#',
            }]
        );
        assert!(session.take_pending_commits().is_empty(), "release queues once");
    }

    #[test]
    fn a_never_painted_buffer_queues_no_commit() {
        let mut session = DetailerSession::default();
        session.begin(
            DetailerStroke::begin(
                PaintHand::Left,
                point(),
                DetailerMode::Additive,
                CellWeight::ONE,
                flat_tile(0),
            ),
            |_| Some('.'),
        );
        // Paint and then subtract back to the exact seed value: the buffer
        // ends where it began, but the stroke did move ink, so it commits.
        session.paint_at(point(), 0, 0, 255, |_| Some('.'));
        session.paint_at(point(), 0, 0, 255, |_| Some('.'));
        session.finish();
        assert_eq!(session.take_pending_commits().len(), 1);
    }

    #[test]
    fn cells_re_entered_within_one_stroke_keep_their_live_buffer() {
        let mut session = DetailerSession::default();
        session.begin(
            DetailerStroke::begin(
                PaintHand::Left,
                point(),
                DetailerMode::Additive,
                CellWeight::ONE,
                flat_tile(0),
            ),
            |_| Some('.'),
        );
        session.paint_at(point(), 0, 0, 120, |_| Some('.'));
        let half_inked = session.strokes[&point()].tile().alpha[0];

        // The drag leaves and re-crosses the same cell: its buffer keeps
        // building instead of reseeding from scratch.
        session.begin(
            DetailerStroke::begin(
                PaintHand::Left,
                point(),
                DetailerMode::Additive,
                CellWeight::ONE,
                flat_tile(255),
            ),
            |_| Some('.'),
        );
        assert_eq!(session.strokes[&point()].tile().alpha[0], half_inked);
        session.paint_at(point(), 0, 0, 135, |_| Some('.'));
        assert_eq!(session.strokes[&point()].tile().alpha[0], 255);
    }

    #[test]
    fn one_stroke_holds_one_buffer_per_crossed_cell_and_commits_each_changed_one() {
        let mut session = DetailerSession::default();
        let here = point();
        let there = CellPoint { x: 5, y: 5, z: 0 };
        session.begin(
            DetailerStroke::begin(
                PaintHand::Left,
                here,
                DetailerMode::Additive,
                CellWeight::ONE,
                flat_tile(0),
            ),
            |_| Some('.'),
        );
        session.paint_at_cell_local(here, 0.99, 0.99, 1, 255, |stroke| {
            Some(if stroke.changed() { '#' } else { '.' })
        });
        // The drag crosses into a second cell and changes it too.
        session.begin(
            DetailerStroke::begin(
                PaintHand::Left,
                there,
                DetailerMode::Additive,
                CellWeight::ONE,
                flat_tile(0),
            ),
            |_| Some('.'),
        );
        session.paint_at_cell_local(there, 0.5, 0.5, 1, 255, |stroke| {
            Some(if stroke.changed() { '#' } else { '.' })
        });
        assert_eq!(session.pending_previews().len(), 2);

        session.finish();
        assert_eq!(
            session.take_pending_commits(),
            vec![
                DetailerCommit {
                    hand: PaintHand::Left,
                    position: here,
                    graphic: '#',
                },
                DetailerCommit {
                    hand: PaintHand::Left,
                    position: there,
                    graphic: '#',
                },
            ],
            "commits come back in cell order, one per changed buffer"
        );
    }

    #[test]
    fn the_brush_footprint_scales_down_into_the_subpixel_buffer() {
        let mut stroke = DetailerStroke::begin(
            PaintHand::Left,
            point(),
            DetailerMode::Additive,
            CellWeight::ONE,
            flat_tile(0),
        );
        // Size 1 touches exactly one subcell pixel.
        stroke.paint_footprint(1, 1, 1, 255);
        let mut expected = flat_tile(0);
        expected.alpha[1 * BUFFER_WIDTH + 1] = 255;
        assert_eq!(stroke.tile(), expected);

        // Size 3 paints the centered 3x3 block.
        let mut stroke = DetailerStroke::begin(
            PaintHand::Left,
            point(),
            DetailerMode::Additive,
            CellWeight::ONE,
            flat_tile(0),
        );
        stroke.paint_footprint(1, 1, 3, 255);
        let mut expected = flat_tile(0);
        for y in 0..3 {
            for x in 0..3 {
                expected.alpha[y * BUFFER_WIDTH + x] = 255;
            }
        }
        assert_eq!(stroke.tile(), expected);

        // The footprint clips at the buffer's edge instead of panicking.
        let mut stroke = DetailerStroke::begin(
            PaintHand::Left,
            point(),
            DetailerMode::Additive,
            CellWeight::ONE,
            flat_tile(0),
        );
        stroke.paint_footprint(0, 0, 5, 255);
        assert!(stroke.changed());
    }

    #[test]
    fn cell_local_canvas_coordinates_map_to_the_typeface_tile() {
        let mut session = DetailerSession::default();
        session.begin(
            DetailerStroke::begin(
                PaintHand::Left,
                point(),
                DetailerMode::Additive,
                CellWeight::ONE,
                flat_tile(0),
            ),
            |_| Some('.'),
        );
        session.paint_at_cell_local(point(), 0.99, 0.99, 1, 255, |stroke| {
            (stroke.tile().alpha[BUFFER_PIXELS - 1] == 255).then_some('#')
        });
        assert_eq!(session.pending_graphic_at(&point()), Some('#'));
    }

    #[test]
    fn other_hand_cannot_finish_the_active_canvas_stroke() {
        let mut session = DetailerSession::default();
        session.begin(
            DetailerStroke::begin(
                PaintHand::Left,
                point(),
                DetailerMode::Additive,
                CellWeight::ONE,
                flat_tile(0),
            ),
            |_| Some('.'),
        );
        session.finish_for_hand(PaintHand::Right);
        assert!(session.is_active());
        assert!(session.take_pending_commits().is_empty());
    }

    #[test]
    fn session_without_a_resolved_glyph_never_queues_a_document_write() {
        let mut session = DetailerSession::default();
        session.begin(
            DetailerStroke::begin(
                PaintHand::Right,
                point(),
                DetailerMode::Subtractive,
                CellWeight::TWO,
                flat_tile(0),
            ),
            |_| None,
        );
        session.finish();
        assert!(session.take_pending_commits().is_empty());
    }
}
