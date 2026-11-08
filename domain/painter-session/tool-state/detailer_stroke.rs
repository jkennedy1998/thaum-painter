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
        assert_eq!(stroke.resolve(&fade), Some(' '), "starts at the seeded empty glyph");

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
}
