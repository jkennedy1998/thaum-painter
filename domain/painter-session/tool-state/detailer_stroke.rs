//! Live detailer stroke accumulation and resolution (project 6kbosu52).
//! A detailer drag paints ink into a subcell comparison-resolution buffer,
//! seeded from the cell's currently authored glyph; release commits
//! whatever glyph the buffer currently resolves to as the cell's new
//! graphic. Color and weight are never touched by a stroke — out of scope
//! for the interaction entirely, not merely lock-gated (J 2026-11-08).

use thaum_renderer_domain::shape_fade::fade::ShapeFade;
use thaum_renderer_domain::{CellPoint, CellWeight, GlyphTileRaster};

use crate::tool_state::PaintHand;

/// Additive paints ink in; subtractive erases ink out. Two variants of the
/// same subcell drawing interaction — a stroke's mode never changes mid-drag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailerMode {
    Additive,
    Subtractive,
}

/// One in-progress detailer stroke: the acting hand, the cell being
/// detailed, the render weight the buffer resolves against (captured at
/// stroke start so a weight change elsewhere never retargets the
/// comparison graph mid-stroke), and the live subcell ink buffer itself.
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
        self.buffer[index] = match self.mode {
            DetailerMode::Additive => self.buffer[index].saturating_add(strength),
            DetailerMode::Subtractive => self.buffer[index].saturating_sub(strength),
        };
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
        fade.nearest_glyph_at_weight(&self.tile(), self.weight)
            .map(|(glyph, _distance)| glyph)
    }
}

/// The one graphic-only write requested when an active detail stroke ends.
/// The canvas-pointer seam consumes it in phase 6; the detail grid never
/// owns a document or performs this write itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DetailerCommit {
    pub position: CellPoint,
    pub graphic: char,
}

/// Shared live state between the canvas target selector, detail grid, and
/// canvas preview. The grid changes only the stroke buffer through this type;
/// it cannot know or mutate document appearance beyond the pending glyph.
#[derive(Debug, Default)]
pub struct DetailerSession {
    stroke: Option<DetailerStroke>,
    pending_graphic: Option<char>,
    pending_commit: Option<DetailerCommit>,
}

impl DetailerSession {
    /// Begins one selected target and clears an unconsumed prior release.
    /// `resolve` is supplied by the caller because the live typeface fade is
    /// orchestration-owned and can change when the file changes its face.
    pub fn begin(
        &mut self,
        stroke: DetailerStroke,
        resolve: impl FnOnce(&DetailerStroke) -> Option<char>,
    ) {
        self.pending_graphic = resolve(&stroke);
        self.stroke = Some(stroke);
        self.pending_commit = None;
    }

    pub fn is_active(&self) -> bool {
        self.stroke.is_some()
    }

    pub fn active_hand(&self) -> Option<PaintHand> {
        self.stroke.as_ref().map(|stroke| stroke.hand)
    }

    pub fn active_tile(&self) -> Option<GlyphTileRaster> {
        self.stroke.as_ref().map(DetailerStroke::tile)
    }

    pub fn pending_graphic(&self) -> Option<char> {
        self.pending_graphic
    }

    /// Paints one comparison pixel and immediately refreshes the canvas
    /// preview candidate. Calls made without an active target are a no-op.
    pub fn paint(
        &mut self,
        x: usize,
        y: usize,
        strength: u8,
        resolve: impl FnOnce(&DetailerStroke) -> Option<char>,
    ) {
        let Some(stroke) = self.stroke.as_mut() else {
            return;
        };
        stroke.paint(x, y, strength);
        self.pending_graphic = resolve(stroke);
    }

    /// Ends the active grid drag. There is at most one queued commit, and it
    /// is graphic-only by construction. No resolved glyph means no write.
    pub fn finish(&mut self) {
        let Some(stroke) = self.stroke.take() else {
            return;
        };
        self.pending_commit = self.pending_graphic.map(|graphic| DetailerCommit {
            position: stroke.position,
            graphic,
        });
        self.pending_graphic = None;
    }

    /// Cancels the active stroke and any previously queued, unconsumed write.
    pub fn cancel(&mut self) {
        self.stroke = None;
        self.pending_graphic = None;
        self.pending_commit = None;
    }

    /// Transfers the one release request to the document-owning consumer.
    pub fn take_pending_commit(&mut self) -> Option<DetailerCommit> {
        self.pending_commit.take()
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
    fn resolve_follows_the_live_buffer_as_it_is_painted() {
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
            Some(' '),
            "starts at the seeded empty glyph"
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
        assert_eq!(session.pending_graphic(), Some('.'));
        assert_eq!(session.active_hand(), Some(PaintHand::Left));

        session.paint(0, 0, 255, |stroke| {
            (stroke.tile().alpha[0] == 255).then_some('#')
        });
        assert_eq!(session.pending_graphic(), Some('#'));

        session.finish();
        assert!(!session.is_active());
        assert_eq!(session.pending_graphic(), None);
        assert_eq!(
            session.take_pending_commit(),
            Some(DetailerCommit {
                position: point(),
                graphic: '#',
            })
        );
        assert_eq!(session.take_pending_commit(), None, "release queues once");
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
        assert_eq!(session.take_pending_commit(), None);
    }
}
