use std::collections::{BTreeMap, BTreeSet};

use thaum_renderer_domain::{CellGraphic, CellPoint};

use crate::paint_color::PaintColor;

/// One brush-painted cell's appearance: what `domain/painter-operations/brush`
/// owns about a stroke, independent of any renderer handoff shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaintedCell {
    pub graphic: CellGraphic,
    pub color: PaintColor,
    pub weight_index: i64,
}

/// A live, in-memory field of painted cells. Not a saved document: this is the
/// headless data `painter-session` mutates during a live editing session.
pub type Canvas = BTreeMap<CellPoint, PaintedCell>;

/// Returns the square footprint for a brush tip of `size` centered on `position`.
pub fn brush_points(position: CellPoint, size: i32) -> Vec<CellPoint> {
    let size = size.clamp(1, 5);
    let left_radius = (size - 1) / 2;
    let right_radius = size - 1 - left_radius;
    let mut points = BTreeSet::new();

    for y in (position.y - left_radius)..=(position.y + right_radius) {
        for x in (position.x - left_radius)..=(position.x + right_radius) {
            points.insert(CellPoint {
                x,
                y,
                z: position.z,
            });
        }
    }

    points.into_iter().collect()
}

/// Draws (or overwrites) one cell at `position`.
pub fn apply_brush(canvas: &mut Canvas, position: CellPoint, cell: PaintedCell) {
    canvas.insert(position, cell);
}

/// Erases whatever cell (if any) is at `position`.
pub fn erase(canvas: &mut Canvas, position: CellPoint) {
    canvas.remove(&position);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x: i32, y: i32) -> CellPoint {
        CellPoint { x, y, z: 0 }
    }

    fn cell(glyph: char) -> PaintedCell {
        PaintedCell {
            graphic: CellGraphic::Glyph(glyph),
            color: PaintColor::flat_rgb(255, 255, 255),
            weight_index: 1,
        }
    }

    #[test]
    fn brush_points_expand_to_the_requested_square_size() {
        assert_eq!(brush_points(point(2, 2), 1), vec![point(2, 2)]);
        assert_eq!(brush_points(point(2, 2), 2).len(), 4);
        assert!(brush_points(point(2, 2), 3).contains(&point(1, 1)));
        assert!(brush_points(point(2, 2), 3).contains(&point(3, 3)));
    }

    #[test]
    fn applying_a_brush_stroke_inserts_exactly_that_cell() {
        let mut canvas = Canvas::new();
        apply_brush(&mut canvas, point(2, 3), cell('#'));

        assert_eq!(canvas.get(&point(2, 3)), Some(&cell('#')));
        assert_eq!(canvas.len(), 1);
    }

    #[test]
    fn applying_a_brush_stroke_to_an_already_painted_cell_overwrites_it() {
        let mut canvas = Canvas::new();
        apply_brush(&mut canvas, point(0, 0), cell('#'));
        apply_brush(&mut canvas, point(0, 0), cell('@'));

        assert_eq!(canvas.get(&point(0, 0)), Some(&cell('@')));
        assert_eq!(canvas.len(), 1);
    }

    #[test]
    fn erasing_a_painted_cell_removes_it() {
        let mut canvas = Canvas::new();
        apply_brush(&mut canvas, point(1, 1), cell('#'));
        erase(&mut canvas, point(1, 1));

        assert!(canvas.get(&point(1, 1)).is_none());
    }

    #[test]
    fn erasing_an_unpainted_cell_is_a_no_op() {
        let mut canvas = Canvas::new();
        apply_brush(&mut canvas, point(5, 5), cell('#'));
        erase(&mut canvas, point(9, 9));

        assert_eq!(canvas.len(), 1);
        assert_eq!(canvas.get(&point(5, 5)), Some(&cell('#')));
    }

    #[test]
    fn brush_and_erase_never_affect_other_cells() {
        let mut canvas = Canvas::new();
        apply_brush(&mut canvas, point(0, 0), cell('#'));
        apply_brush(&mut canvas, point(1, 0), cell('@'));
        erase(&mut canvas, point(0, 0));

        assert!(canvas.get(&point(0, 0)).is_none());
        assert_eq!(canvas.get(&point(1, 0)), Some(&cell('@')));
    }
}
