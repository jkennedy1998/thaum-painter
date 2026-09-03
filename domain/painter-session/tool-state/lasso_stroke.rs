//! Live lasso stroke accumulation and its overlay preview cell group.
//! A lasso drag records the freehand bound; release rasterizes it through
//! the pure `lasso` operation and applies it through the hand's tool state.

use thaum_renderer_domain::{Cell, CellColor, CellGraphic, CellGroup, CellPoint, CellWeight, WorldPoint};

use crate::tool_state::PaintHand;

/// One in-progress lasso bound: the acting hand plus the freehand path
/// recorded so far. Release turns the path into enclosed cells; the stroke
/// itself never paints.
#[derive(Debug, Clone)]
pub struct LassoStroke {
    pub hand: PaintHand,
    pub path: Vec<CellPoint>,
}

impl LassoStroke {
    pub fn new(hand: PaintHand, position: CellPoint) -> Self {
        Self {
            hand,
            path: vec![position],
        }
    }

    pub fn extend(&mut self, points: &[CellPoint]) {
        self.path.extend_from_slice(points);
    }
}

/// Overlay preview for an in-progress lasso: draws the bound path itself,
/// never the interior — the interior only exists once the stroke releases.
pub fn build_lasso_path_cell_group(stroke: &LassoStroke) -> CellGroup {
    let mut group = CellGroup::new(WorldPoint { x: 0, y: 0, z: 0 });
    for position in &stroke.path {
        group.insert(Cell {
            position: *position,
            graphic: CellGraphic::Glyph('◌'),
            color: CellColor::Flat([0.55, 0.75, 1.0, 1.0]),
            weight: CellWeight::from_index_clamped(3),
            ..Cell::default()
        });
    }
    group
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x: i32, y: i32) -> CellPoint {
        CellPoint { x, y, z: 0 }
    }

    #[test]
    fn a_lasso_stroke_starts_at_its_press_cell() {
        let stroke = LassoStroke::new(PaintHand::Left, point(2, 3));
        assert_eq!(stroke.hand, PaintHand::Left);
        assert_eq!(stroke.path, vec![point(2, 3)]);
    }

    #[test]
    fn extending_appends_the_interpolated_drag_points() {
        let mut stroke = LassoStroke::new(PaintHand::Right, point(0, 0));
        stroke.extend(&[point(1, 0), point(2, 0)]);
        assert_eq!(stroke.path, vec![point(0, 0), point(1, 0), point(2, 0)]);
    }
}
