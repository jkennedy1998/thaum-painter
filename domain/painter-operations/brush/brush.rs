use std::collections::{BTreeMap, BTreeSet};

use thaum_renderer_domain::{CameraViewOrientation, CellGraphic, CellPoint};

use crate::paint_color::PaintColor;

/// One brush-painted cell's appearance: what `domain/painter-operations/brush`
/// owns about a stroke, independent of any renderer handoff shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaintedCell {
    pub graphic: CellGraphic,
    pub color: PaintColor,
    pub weight_index: i64,
    /// Ordered, portable renderer shader asset filenames. The order is part of
    /// authored appearance because later shaders override only the channels
    /// they write.
    pub shader_stack: Vec<String>,
}

/// A live, in-memory field of painted cells. Not a saved document: this is the
/// headless data `painter-session` mutates during a live editing session.
pub type Canvas = BTreeMap<CellPoint, PaintedCell>;

/// Returns the square footprint for a brush tip of `size` centered on `position`,
/// lying flat in the camera's active view plane: offsets grow along the view
/// orientation's right/up axes and the view's depth axis never moves, so the tip
/// always faces the camera at every swing and roll. The default PosZ swing
/// (right = East, up = Top) reproduces the original x/y-expansion behavior.
pub fn brush_points(
    position: CellPoint,
    size: i32,
    orientation: CameraViewOrientation,
) -> Vec<CellPoint> {
    let size = size.clamp(1, 5);
    let left_radius = (size - 1) / 2;
    let right_radius = size - 1 - left_radius;
    let right = orientation.right.unit_vector();
    let up = orientation.up.unit_vector();
    let mut points = BTreeSet::new();

    for dr in -left_radius..=right_radius {
        for du in -left_radius..=right_radius {
            points.insert(CellPoint {
                x: position.x + right[0] * dr + up[0] * du,
                y: position.y + right[1] * dr + up[1] * du,
                z: position.z + right[2] * dr + up[2] * du,
            });
        }
    }

    points.into_iter().collect()
}

/// Draws (or overwrites) one cell at `position`.
pub fn apply_brush(canvas: &mut Canvas, position: CellPoint, cell: PaintedCell) {
    canvas.insert(position, cell);
}

/// True when a painted cell is an authored blank: a space glyph renders
/// nothing, so under the unified empty-cell rule it carries no color or
/// weight and counts as an empty canvas cell.
pub fn is_blank_cell(cell: &PaintedCell) -> bool {
    matches!(cell.graphic, CellGraphic::Glyph(' '))
}

/// The unified empty-cell read seam: a canvas cell is empty when it is
/// absent OR an authored blank (space glyph). Stored blanks are legacy
/// artifacts; reads must treat them like `None`.
pub fn effective_cell(existing: Option<&PaintedCell>) -> Option<&PaintedCell> {
    existing.filter(|cell| !is_blank_cell(cell))
}

/// The unified empty-cell write seam: non-blank cells insert, blank cells
/// remove. Masked tools that resolve to an authored blank (a space glyph
/// carries no color or weight) leave the cell truly empty instead of
/// storing an invisible colored space.
pub fn write_cell(canvas: &mut Canvas, position: CellPoint, cell: PaintedCell) {
    if is_blank_cell(&cell) {
        canvas.remove(&position);
    } else {
        canvas.insert(position, cell);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x: i32, y: i32) -> CellPoint {
        CellPoint { x, y, z: 0 }
    }

    fn default_view() -> CameraViewOrientation {
        camera_view_orientation_for_camera(CameraSwing::PosZ, CameraRoll::Deg0)
    }

    use thaum_renderer_domain::{camera_view_orientation_for_camera, CameraRoll, CameraSwing};

    fn cell(glyph: char) -> PaintedCell {
        PaintedCell {
            graphic: CellGraphic::Glyph(glyph),
            color: PaintColor::flat_rgb(255, 255, 255),
            weight_index: 1,
            shader_stack: Vec::new(),
        }
    }

    #[test]
    fn brush_points_expand_to_the_requested_square_size_in_the_default_view() {
        let orientation = default_view();
        assert_eq!(brush_points(point(2, 2), 1, orientation), vec![point(2, 2)]);
        assert_eq!(brush_points(point(2, 2), 2, orientation).len(), 4);
        assert!(brush_points(point(2, 2), 3, orientation).contains(&point(1, 2)));
        assert!(brush_points(point(2, 2), 3, orientation).contains(&point(3, 2)));
    }

    #[test]
    fn brush_points_lie_flat_in_the_active_view_plane_at_every_swing() {
        // Looking down the x axis: the footprint must spread across z (screen
        // right) and y (screen up) while x — the view's depth — stays fixed.
        let orientation = camera_view_orientation_for_camera(CameraSwing::PosX, CameraRoll::Deg0);
        let center = CellPoint { x: 4, y: 2, z: 2 };
        let points = brush_points(center, 3, orientation);
        assert!(points.iter().all(|p| p.x == 4));
        assert!(points.contains(&CellPoint { x: 4, y: 2, z: 1 }));
        assert!(points.contains(&CellPoint { x: 4, y: 2, z: 3 }));
        assert!(points.contains(&CellPoint { x: 4, y: 3, z: 2 }));
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
    fn writing_the_clear_character_removes_only_that_cell() {
        let mut canvas = Canvas::new();
        apply_brush(&mut canvas, point(0, 0), cell('#'));
        apply_brush(&mut canvas, point(1, 0), cell('@'));
        write_cell(&mut canvas, point(0, 0), cell(' '));
        write_cell(&mut canvas, point(9, 9), cell(' '));

        assert!(canvas.get(&point(0, 0)).is_none());
        assert_eq!(canvas.get(&point(1, 0)), Some(&cell('@')));
        assert_eq!(canvas.len(), 1);
    }
}
