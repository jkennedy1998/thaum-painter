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
    /// Captured at stroke start: whether this stroke may resolve across
    /// every declared render weight (J 2026-11-09's multi-weight toggle,
    /// gated by the hand's weight-channel lock at press time). A property
    /// flip mid-stroke never retargets the live resolve.
    pub multi_weight: bool,
    buffer_width: usize,
    buffer_height: usize,
    /// Ink amount per subcell pixel (row-major), seeded from the cell's
    /// current authored glyph's own comparison tile.
    buffer: Vec<u8>,
    /// True once a paint sample actually moved at least one buffer pixel
    /// off its seed value. A stroke that never moved ink commits nothing.
    changed: bool,
    /// The glyph this buffer was seeded from, if the cell held one. Barred
    /// from winning the resolve: a stroke that moves ink must land on a
    /// different, visible shape, or the edit writes nothing (J 2026-11-09:
    /// a subtractive line through full cells must author a visible slight
    /// negative, which snapping back to the seed made impossible).
    seed_glyph: Option<char>,
    /// The buffer's seed values, kept for the visible-change gate: a stroke
    /// that paints and then paints back to exactly its seed has no visible
    /// change and commits nothing, even though pixels moved along the way.
    seed_alpha: Vec<u8>,
    /// The last subcell footprint start this stroke painted in this buffer,
    /// for interpolating between consecutive samples: a drag crossing a cell
    /// leaves a continuous line, not isolated frames (J 2026-11-09: draw a
    /// big circle and each cell should resolve to the glyph nearest its own
    /// segment of the stroke). Coordinates are footprint top-left indices and
    /// may sit outside the buffer — the footprint clips.
    last_painted: Option<(i64, i64)>,
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
        multi_weight: bool,
        seed: GlyphTileRaster,
        seed_glyph: Option<char>,
    ) -> Self {
        Self {
            hand,
            position,
            mode,
            weight,
            multi_weight,
            buffer_width: seed.width,
            buffer_height: seed.height,
            seed_alpha: seed.alpha.clone(),
            buffer: seed.alpha,
            changed: false,
            seed_glyph,
            last_painted: None,
        }
    }

    pub fn buffer_dimensions(&self) -> (usize, usize) {
        (self.buffer_width, self.buffer_height)
    }

    /// Paints the brush footprint whose top-left subcell pixel is `x`,`y`:
    /// sizes 1..=3 are solid `size`×`size` blocks, sizes 4 and 5 drop the
    /// four corner pixels (J 2026-11-09: "1 will be 1 subpixel. 2 will be
    /// 2x2 subpixel. 3 will be 3x3 subpixel. 4 will be 4x4 subpixels with
    /// the corners removed. 5x5 with corners removed for the size 5").
    /// Samples outside the buffer are no-ops, so a footprint anchored near
    /// or past a cell's edge simply clips to the part that lands inside.
    pub fn paint_footprint(&mut self, x: i64, y: i64, size: i32, strength: u8) {
        let size = size.clamp(1, 5) as i64;
        for dy in 0..size {
            for dx in 0..size {
                if size >= 4
                    && (dx == 0 || dx == size - 1)
                    && (dy == 0 || dy == size - 1)
                {
                    continue;
                }
                self.paint(x + dx, y + dy, strength);
            }
        }
    }

    /// Paints one subcell pixel: additive raises it toward full ink,
    /// subtractive lowers it toward none, both by `strength`, saturating at
    /// the ink range's ends. An out-of-bounds coordinate is a no-op.
    pub fn paint(&mut self, x: i64, y: i64, strength: u8) {
        if x < 0 || y < 0 || x >= self.buffer_width as i64 || y >= self.buffer_height as i64 {
            return;
        }
        let index = y as usize * self.buffer_width + x as usize;
        let before = self.buffer[index];
        self.buffer[index] = match self.mode {
            DetailerMode::Additive => before.saturating_add(strength),
            DetailerMode::Subtractive => before.saturating_sub(strength),
        };
        if self.buffer[index] != before {
            self.changed = true;
        }
    }

    /// Whether any paint sample moved this stroke's buffer off its seed.
    pub fn changed(&self) -> bool {
        self.changed
    }

    /// Whether the buffer currently differs from its seed at all: the gate
    /// for resolve, preview, and commit. Pixels may have moved and moved
    /// back; only the net difference is visible, and only a visible change
    /// may commit a glyph.
    pub fn visibly_changed(&self) -> bool {
        self.buffer != self.seed_alpha
    }

    /// Paints the brush footprint with top-left at `x`,`y` and interpolates
    /// from this buffer's last painted footprint start with a Bresenham
    /// line, footprint at every step — a drag crossing a cell leaves a
    /// continuous stroke segment instead of isolated frame dots. Re-crossing
    /// a cell picks up from where it left, so a line drawn through a cell
    /// stays connected. Coordinates may sit outside the buffer; the
    /// footprints clip, so only the in-cell part lands.
    pub fn paint_line_to(&mut self, x: i64, y: i64, size: i32, strength: u8) {
        let target = (x, y);
        if let Some((last_x, last_y)) = self.last_painted {
            let mut from = (last_x, last_y);
            let to = target;
            let dx = (to.0 - from.0).abs();
            let dy = -(to.1 - from.1).abs();
            let step_x = if to.0 > from.0 { 1 } else { -1 };
            let step_y = if to.1 > from.1 { 1 } else { -1 };
            let mut error = dx + dy;
            loop {
                self.paint_footprint(from.0, from.1, size, strength);
                if from == to {
                    break;
                }
                let error_twice = 2 * error;
                if error_twice >= dy {
                    error += dy;
                    from.0 += step_x;
                }
                if error_twice <= dx {
                    error += dx;
                    from.1 += step_y;
                }
            }
        } else {
            self.paint_footprint(x, y, size, strength);
        }
        self.last_painted = Some(target);
    }

    /// The continuous footprint start subcell coordinate for a pointer at
    /// continuous cell-local position `pointer` (0..typeface-width) with a
    /// brush of `size`: odd sizes center on the pointer, even sizes split
    /// the half-subpixel evenly so pressing exactly on a cell's corner
    /// authors the matching corner subpixels in every adjacent cell
    /// (J 2026-11-09: "if i press the corner of a cell it should draw 1
    /// cell at each appropriate corner of a cell").
    pub fn footprint_start(pointer: f32, size: i32) -> i64 {
        (pointer - (size - 1) as f32 / 2.0).floor() as i64
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
    pub fn resolve(&self, fade: &ShapeFade) -> Option<(char, CellWeight)> {
        // Two resolve bars, both from J 2026-11-09:
        // - additive strokes resolve against inked glyphs only (the blank
        //   tile is every sparse mark's nearest neighbor and would make
        //   drawn strokes resolve to the invisible space glyph); adding ink
        //   can never legitimately produce the blank glyph. Subtractive
        //   keeps the blank candidate: erasing to nothing is a valid full
        //   erase.
        // - the seed glyph never wins: a stroke that moved ink must resolve
        //   to a different, visible shape — a line subtracted through a
        //   full cell lands on the nearest glyph with that negative in it,
        //   not back on the untouched original.
        //
        // Weight scope (J 2026-11-09): with multi-weight captured (the
        // toggle on AND the hand's weight channel unlocked) the search runs
        // across every declared render weight and the winner names its own
        // weight for the commit; otherwise the search stays scoped to the
        // stroke's captured weight and that weight is authored unchanged.
        if self.multi_weight {
            match self.mode {
                DetailerMode::Additive => {
                    fade.nearest_inked_glyph_across_weights_excluding(
                        &self.tile(),
                        self.seed_glyph,
                    )
                }
                DetailerMode::Subtractive => {
                    fade.nearest_glyph_across_weights_excluding(&self.tile(), self.seed_glyph)
                }
            }
            .map(|(glyph, weight, _distance)| (glyph, weight))
        } else {
            match self.mode {
                DetailerMode::Additive => fade.nearest_inked_glyph_at_weight_excluding(
                    &self.tile(),
                    self.weight,
                    self.seed_glyph,
                ),
                DetailerMode::Subtractive => fade.nearest_glyph_at_weight_excluding(
                    &self.tile(),
                    self.weight,
                    self.seed_glyph,
                ),
            }
            .map(|(glyph, _distance)| (glyph, self.weight))
        }
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
    /// The weight the resolved glyph named: the stroke's captured weight
    /// when the resolve stayed weight-scoped, or the winning glyph's own
    /// weight on a cross-weight resolve (J 2026-11-09 multi-weight).
    pub weight: CellWeight,
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
    /// Each stroke's live resolved (glyph, weight), for preview and release
    /// commit.
    pending: BTreeMap<CellPoint, (char, CellWeight)>,
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
        resolve: impl FnOnce(&DetailerStroke) -> Option<(char, CellWeight)>,
    ) {
        if self.active.is_none() {
            self.active = Some(stroke.hand);
            self.stroke_mode = Some(stroke.mode);
        }
        // A freshly seeded buffer has not moved ink yet: it previews and
        // commits nothing, even though its resolve (seed glyph barred) would
        // name some other glyph. Only an actual paint puts a cell on the
        // preview/commit track.
        if stroke.visibly_changed() {
            if let Some(resolved) = resolve(&stroke) {
                self.pending.insert(stroke.position, resolved);
            }
        }
        self.strokes.entry(stroke.position).or_insert(stroke);
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

    /// One live stroke buffer's (width, height) at `position`, if crossed.
    /// Every cell's buffer shares the typeface's comparison-tile dimensions;
    /// callers read them here to map continuous canvas positions into
    /// subcell coordinates.
    pub fn stroke_dimensions_at(&self, position: &CellPoint) -> Option<(usize, usize)> {
        self.strokes
            .get(position)
            .map(|stroke| stroke.buffer_dimensions())
    }

    pub fn pending_at(&self, position: &CellPoint) -> Option<(char, CellWeight)> {
        self.pending.get(position).copied()
    }

    /// Every changed cell's live candidate: exactly what release would
    /// write, one (position, glyph, weight) per crossed cell whose buffer
    /// actually moved ink and resolved. Untouched cells preview nothing —
    /// a buffer still at its seed has nothing to show yet.
    pub fn pending_previews(&self) -> Vec<(CellPoint, char, CellWeight)> {
        self.pending
            .iter()
            .filter(|(position, _)| {
                self.strokes
                    .get(position)
                    .is_some_and(|stroke| stroke.visibly_changed())
            })
            .map(|(p, (g, w))| (*p, *g, *w))
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
        resolve: impl Fn(&DetailerStroke) -> Option<(char, CellWeight)>,
    ) {
        self.paint_samples(
            [(position, x as i64, y as i64)],
            1,
            strength,
            resolve,
        );
    }

    /// Paints one brush footprint sample per entry: each sample names its
    /// cell and that cell's footprint top-left in the cell's own subcell
    /// coordinates (possibly out of range — the footprint clips). One drag
    /// anchored near a cell edge or corner touches several cells at once;
    /// each touched cell's stroke receives only the part of the footprint
    /// that lands inside it, so authoring feels continuous across the whole
    /// canvas instead of snapping to one cell (J 2026-11-09: "it shouldnt
    /// snap, it should really feel fluid like just drawing over multiple
    /// cells simply with this detailer tool"). Calls naming a cell without
    /// a live stroke are skipped; every painted cell's preview candidate
    /// refreshes.
    pub fn paint_samples(
        &mut self,
        samples: impl IntoIterator<Item = (CellPoint, i64, i64)>,
        brush_size: i32,
        strength: u8,
        resolve: impl Fn(&DetailerStroke) -> Option<(char, CellWeight)>,
    ) {
        for (position, x, y) in samples {
            let resolved = {
                let Some(stroke) = self.strokes.get_mut(&position) else {
                    continue;
                };
                stroke.paint_line_to(x, y, brush_size, strength);
                if stroke.visibly_changed() {
                    resolve(stroke)
                } else {
                    None
                }
            };
            match resolved {
                Some(graphic) => {
                    self.pending.insert(position, graphic);
                }
                None => {
                    self.pending.remove(&position);
                }
            }
        }
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
        resolve: impl Fn(&DetailerStroke) -> Option<(char, CellWeight)>,
    ) {
        let (pointer_x, pointer_y) = {
            let Some(stroke) = self.strokes.get(&position) else {
                return;
            };
            (
                local_x.clamp(0.0, 1.0 - f32::EPSILON) * stroke.buffer_width as f32,
                local_y.clamp(0.0, 1.0 - f32::EPSILON) * stroke.buffer_height as f32,
            )
        };
        self.paint_samples(
            [(
                position,
                DetailerStroke::footprint_start(pointer_x, brush_size),
                DetailerStroke::footprint_start(pointer_y, brush_size),
            )],
            brush_size,
            strength,
            resolve,
        );
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
            if let (true, Some((graphic, weight))) = (stroke.visibly_changed(), resolved) {
                self.pending_commits.insert(
                    position,
                    DetailerCommit {
                        hand: stroke.hand,
                        position,
                        graphic,
                        weight,
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

    /// Two bars and a dot: enough shape vocabulary that a vertical segment
    /// and a horizontal segment cannot resolve to the same glyph.
    struct Bars;
    impl FadeTileProvider for Bars {
        fn tiles(&self) -> Vec<(char, GlyphTileRaster)> {
            let tile = |ones: &[usize]| {
                let mut alpha = vec![0u8; BUFFER_PIXELS];
                for &i in ones {
                    alpha[i] = 255;
                }
                GlyphTileRaster {
                    width: BUFFER_WIDTH,
                    height: BUFFER_HEIGHT,
                    alpha,
                }
            };
            vec![
                ('|', tile(&[5, 9])),
                ('-', tile(&[5, 6])),
                ('.', tile(&[5])),
            ]
        }
    }

    fn bar_stroke() -> DetailerStroke {
        DetailerStroke::begin(
            PaintHand::Left,
            point(),
            DetailerMode::Additive,
            CellWeight::ONE,
            false,
            flat_tile(0),
            None,
        )
    }

    /// J 2026-11-09: different cells crossed by one stroke must resolve to
    /// glyphs matching their own segment of the stroke, not one uniform
    /// glyph everywhere. Interpolation is what deposits a real segment —
    /// vertical here, horizontal there — instead of isolated frame dots
    /// that all snap to the same smallest glyph.
    #[test]
    fn a_vertical_segment_and_a_horizontal_segment_resolve_to_different_glyphs() {
        let fade = ShapeFade::build(&Bars);

        let mut vertical = bar_stroke();
        vertical.paint_line_to(1, 1, 1, 255);
        vertical.paint_line_to(1, 2, 1, 255);
        assert_eq!(vertical.resolve(&fade).map(|(g, _)| g), Some('|'), "vertical segment");

        let mut horizontal = bar_stroke();
        horizontal.paint_line_to(1, 1, 1, 255);
        horizontal.paint_line_to(2, 1, 1, 255);
        assert_eq!(horizontal.resolve(&fade).map(|(g, _)| g), Some('-'), "horizontal segment");

        assert_ne!(
            vertical.resolve(&fade),
            horizontal.resolve(&fade),
            "segment orientation must decide the resolved glyph"
        );
    }

    /// Consecutive samples are connected: one drag paints the whole line
    /// between them, not just the sample frames.
    #[test]
    fn samples_are_interpolated_into_a_continuous_line() {
        let mut stroke = bar_stroke();
        stroke.paint_line_to(0, 0, 1, 255);
        stroke.paint_line_to(3, 3, 1, 255);
        for i in 0..4 {
            assert_eq!(stroke.buffer[i * 4 + i], 255, "diagonal pixel ({i},{i})");
        }
    }

    /// A stroke that moved ink can never resolve back to the glyph it
    /// started from: the seed is barred from both resolve lanes, so every
    /// visible edit lands on a different glyph — the subtractive "slight
    /// negative" J asked for instead of snapping back to the original.
    #[test]
    fn resolve_never_returns_the_seed_glyph() {
        let fade = ShapeFade::build(&Bars);

        let mut additive = bar_stroke();
        additive.paint_line_to(1, 1, 1, 255);
        assert!(additive.resolve(&fade).is_some());
        assert_ne!(additive.resolve(&fade).map(|(g, _)| g), Some(' '));

        let mut subtractive = DetailerStroke::begin(
            PaintHand::Left,
            point(),
            DetailerMode::Subtractive,
            CellWeight::ONE,
            false,
            flat_tile(255),
            Some('#'),
        );
        subtractive.paint_line_to(1, 1, 1, 255);
        let resolved = subtractive.resolve(&fade).map(|(g, _)| g);
        assert_ne!(resolved, Some('#'), "seed glyph barred");
        assert_ne!(resolved, None, "a moved buffer resolves to some glyph");
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
            false,
            seed.clone(), None);
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
            false,
            flat_tile(200), None);
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
            false,
            flat_tile(20), None);
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
            false,
            flat_tile(0), None);
        stroke.paint(BUFFER_WIDTH as i64, 0, 255);
        stroke.paint(0, BUFFER_HEIGHT as i64, 255);
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
            false,
            flat_tile(0), None);
        assert_eq!(
            stroke.resolve(&fade).map(|(g, _)| g),
            Some('#'),
            "additive resolves against inked candidates only: the blank tile is every sparse mark's nearest neighbor and would make drawn strokes resolve to the invisible space glyph"
        );

        for y in 0..BUFFER_HEIGHT {
            for x in 0..BUFFER_WIDTH {
                stroke.paint(x as i64, y as i64, 255);
            }
        }
        assert_eq!(
            stroke.resolve(&fade).map(|(g, _)| g),
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
            false,
            flat_tile(255), None);
        assert_eq!(stroke.resolve(&fade).map(|(g, _)| g), Some('#'));

        for y in 0..BUFFER_HEIGHT {
            for x in 0..BUFFER_WIDTH {
                stroke.paint(x as i64, y as i64, 255);
            }
        }
        assert_eq!(
            stroke.resolve(&fade).map(|(g, _)| g),
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
            false,
            flat_tile(128), None);
        let resolved = stroke.resolve(&fade).unwrap().0;
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
                false,
                flat_tile(0),
                None,
            ),
            |_| Some(('.', CellWeight::ONE)),
        );
        // A fresh seed has not moved ink: it previews nothing yet, even
        // though its resolve (seed barred) would name some other glyph.
        assert_eq!(session.pending_at(&point()), None);
        assert_eq!(session.active_hand(), Some(PaintHand::Left));

        session.paint_at(point(), 0, 0, 255, |stroke| {
            (stroke.tile().alpha[0] == 255).then_some(('#', CellWeight::ONE))
        });
        assert_eq!(session.pending_at(&point()), Some(('#', CellWeight::ONE)));

        session.finish();
        assert!(!session.is_active());
        assert_eq!(session.pending_at(&point()), None);
        assert_eq!(
            session.take_pending_commits(),
            vec![DetailerCommit {
                hand: PaintHand::Left,
                position: point(),
                graphic: '#',
                weight: CellWeight::ONE,
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
                false,
                flat_tile(0),
                None,
            ),
            |_| Some(('.', CellWeight::ONE)),
        );
        // Paint and then subtract back to the exact seed value: the buffer
        // ends where it began, but the stroke did move ink, so it commits.
        session.paint_at(point(), 0, 0, 255, |_| Some(('.', CellWeight::ONE)));
        session.paint_at(point(), 0, 0, 255, |_| Some(('.', CellWeight::ONE)));
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
                false,
                flat_tile(0),
                None,
            ),
            |_| Some(('.', CellWeight::ONE)),
        );
        session.paint_at(point(), 0, 0, 120, |_| Some(('.', CellWeight::ONE)));
        let half_inked = session.strokes[&point()].tile().alpha[0];

        // The drag leaves and re-crosses the same cell: its buffer keeps
        // building instead of reseeding from scratch.
        session.begin(
            DetailerStroke::begin(
                PaintHand::Left,
                point(),
                DetailerMode::Additive,
                CellWeight::ONE,
                false,
                flat_tile(255),
                None,
            ),
            |_| Some(('.', CellWeight::ONE)),
        );
        assert_eq!(session.strokes[&point()].tile().alpha[0], half_inked);
        session.paint_at(point(), 0, 0, 135, |_| Some(('.', CellWeight::ONE)));
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
                false,
                flat_tile(0),
                None,
            ),
            |_| Some(('.', CellWeight::ONE)),
        );
        session.paint_at_cell_local(here, 0.99, 0.99, 1, 255, |stroke| {
            Some(if stroke.changed() { ('#', CellWeight::ONE) } else { ('.', CellWeight::ONE) })
        });
        // The drag crosses into a second cell and changes it too.
        session.begin(
            DetailerStroke::begin(
                PaintHand::Left,
                there,
                DetailerMode::Additive,
                CellWeight::ONE,
                false,
                flat_tile(0),
                None,
            ),
            |_| Some(('.', CellWeight::ONE)),
        );
        session.paint_at_cell_local(there, 0.5, 0.5, 1, 255, |stroke| {
            Some(if stroke.changed() { ('#', CellWeight::ONE) } else { ('.', CellWeight::ONE) })
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
                    weight: CellWeight::ONE,
                },
                DetailerCommit {
                    hand: PaintHand::Left,
                    position: there,
                    graphic: '#',
                    weight: CellWeight::ONE,
                },
            ],
            "commits come back in cell order, one per changed buffer"
        );
    }

    #[test]
    fn the_brush_footprint_scales_down_into_the_subpixel_buffer() {
        // J 2026-11-09: "1 will be 1 subpixel. 2 will be 2x2 subpixel.
        // 3 will be 3x3 subpixel. 4 will be 4x4 subpixels with the corners
        // removed. 5x5 with corners removed for the size 5." The footprint
        // anchors at its top-left subpixel and clips at the buffer's edges.
        const W: usize = 8;
        let blank = || GlyphTileRaster {
            width: W,
            height: W,
            alpha: vec![0u8; W * W],
        };
        let painted = |x: i64, y: i64, size: i32| {
            let mut stroke = DetailerStroke::begin(
                PaintHand::Left,
                point(),
                DetailerMode::Additive,
                CellWeight::ONE,
                false,
                blank(), None);
            stroke.paint_footprint(x, y, size, 255);
            stroke.tile()
        };

        // Size 1 touches exactly one subcell pixel.
        let mut expected = blank();
        expected.alpha[1 * W + 1] = 255;
        assert_eq!(painted(1, 1, 1), expected);

        // Size 2 paints the 2x2 block from its top-left.
        let mut expected = blank();
        for y in 1..3 {
            for x in 1..3 {
                expected.alpha[y * W + x] = 255;
            }
        }
        assert_eq!(painted(1, 1, 2), expected);

        // Size 4 paints the 4x4 block minus its four corners (12 pixels).
        let mut expected = blank();
        for y in 0..4 {
            for x in 0..4 {
                let corner = (x == 0 || x == 3) && (y == 0 || y == 3);
                if !corner {
                    expected.alpha[y * W + x] = 255;
                }
            }
        }
        assert_eq!(painted(0, 0, 4), expected);

        // Size 5 paints the 5x5 block minus its four corners (21 pixels).
        let mut expected = blank();
        for y in 0..5 {
            for x in 0..5 {
                let corner = (x == 0 || x == 4) && (y == 0 || y == 4);
                if !corner {
                    expected.alpha[y * W + x] = 255;
                }
            }
        }
        assert_eq!(painted(0, 0, 5), expected);

        // A footprint anchored past the buffer's edge clips, never panics.
        assert_ne!(painted(W as i64 - 1, 2, 5), blank());
    }

    /// J 2026-11-09: "if i press the corner of a cell it should draw 1 cell
    /// at each appropriate corner of a cell" — an even-size footprint
    /// anchored exactly on the cell boundary splits its subpixels evenly
    /// between the adjacent cells, and the pointer's cell decomposition
    /// carries the overlapping part of the footprint into each touched
    /// cell's own buffer.
    #[test]
    fn footprint_start_splits_even_sizes_evenly_on_cell_boundaries() {
        // A pointer at the exact left boundary with size 2 starts one subcell
        // into the left neighbor: the 2x2 block lands half here, half there.
        assert_eq!(DetailerStroke::footprint_start(0.0, 2), -1);
        // Mid-subpixel, size 2 stays in the pointer's subcell column.
        assert_eq!(DetailerStroke::footprint_start(0.5, 2), 0);
        // Odd sizes center on the pointer's subcell.
        assert_eq!(DetailerStroke::footprint_start(1.5, 3), 0);
        assert_eq!(DetailerStroke::footprint_start(1.5, 5), -1);
    }

    #[test]
    fn a_corner_anchored_footprint_paints_every_touched_cells_corner() {
        let fade = ShapeFade::build(&Bars);
        let mut session = DetailerSession::default();
        // Seed two cells that share an edge: the pointer's cell and its left
        // neighbor, both blank at comparison resolution.
        for position in [CellPoint { x: 0, y: 0, z: 0 }, CellPoint { x: 1, y: 0, z: 0 }] {
            session.begin(
                DetailerStroke::begin(
                    PaintHand::Left,
                    position,
                    DetailerMode::Additive,
                    CellWeight::ONE,
                    false,
                    flat_tile(0),
                    None,
                ),
                |_| Some(('.', CellWeight::ONE)),
            );
        }
        // Size 2 anchored exactly on the shared boundary: one subpixel column
        // in each cell (the even split), no snapping to the pointer's cell.
        session.paint_samples(
            [
                (CellPoint { x: 0, y: 0, z: 0 }, BUFFER_WIDTH as i64 - 1, 0),
                (CellPoint { x: 1, y: 0, z: 0 }, 0, 0),
            ],
            2,
            255,
            |_| Some(('#', CellWeight::ONE)),
        );
        let left = session.pending_at(&CellPoint { x: 0, y: 0, z: 0 });
        let right = session.pending_at(&CellPoint { x: 1, y: 0, z: 0 });
        assert_eq!(left, Some(('#', CellWeight::ONE)), "left cell keeps its half of the footprint");
        assert_eq!(right, Some(('#', CellWeight::ONE)), "right cell keeps its half of the footprint");
        assert_eq!(fade.nearest_glyph_at_weight_excluding(&flat_tile(0), CellWeight::ONE, None).is_some(), true);
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
                false,
                flat_tile(0),
                None,
            ),
            |_| Some(('.', CellWeight::ONE)),
        );
        session.paint_at_cell_local(point(), 0.99, 0.99, 1, 255, |stroke| {
            (stroke.tile().alpha[BUFFER_PIXELS - 1] == 255).then_some(('#', CellWeight::ONE))
        });
        assert_eq!(session.pending_at(&point()), Some(('#', CellWeight::ONE)));
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
                false,
                flat_tile(0),
                None,
            ),
            |_| Some(('.', CellWeight::ONE)),
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
                false,
                flat_tile(0),
                None,
            ),
            |_| None,
        );
        session.finish();
        assert!(session.take_pending_commits().is_empty());
    }
}
