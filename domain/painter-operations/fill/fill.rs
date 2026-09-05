use std::collections::{BTreeSet, VecDeque};

use thaum_renderer_domain::CellPoint;

use crate::brush::{effective_cell, Canvas, PaintedCell};

/// Finite fill bounds for painter flood fill. The live canvas is logically
/// unbounded, so fill needs the caller to declare the region it is allowed to
/// traverse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanvasBounds {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
    /// The fixed plane coordinate along `plane_axis`.
    pub z: i32,
    pub plane_axis: CanvasPlaneAxis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CanvasPlaneAxis {
    X,
    Y,
    #[default]
    Z,
}

impl CanvasBounds {
    pub fn contains(self, point: CellPoint) -> bool {
        let (plane_x, plane_y, plane) = self.world_to_plane(point);
        plane == self.z
            && plane_x >= self.x0
            && plane_x <= self.x1
            && plane_y >= self.y0
            && plane_y <= self.y1
    }

    pub fn plane_point(self, plane_x: i32, plane_y: i32) -> CellPoint {
        match self.plane_axis {
            CanvasPlaneAxis::Z => CellPoint {
                x: plane_x,
                y: plane_y,
                z: self.z,
            },
            CanvasPlaneAxis::X => CellPoint {
                x: self.z,
                y: plane_x,
                z: plane_y,
            },
            CanvasPlaneAxis::Y => CellPoint {
                x: plane_x,
                y: self.z,
                z: plane_y,
            },
        }
    }

    pub fn iter_points(self) -> Vec<CellPoint> {
        let mut points = Vec::new();
        for plane_y in self.y0..=self.y1 {
            for plane_x in self.x0..=self.x1 {
                points.push(self.plane_point(plane_x, plane_y));
            }
        }
        points
    }

    pub fn offset_in_plane(self, point: CellPoint, delta_x: i32, delta_y: i32) -> CellPoint {
        match self.plane_axis {
            CanvasPlaneAxis::Z => CellPoint {
                x: point.x + delta_x,
                y: point.y + delta_y,
                z: point.z,
            },
            CanvasPlaneAxis::X => CellPoint {
                x: point.x,
                y: point.y + delta_x,
                z: point.z + delta_y,
            },
            CanvasPlaneAxis::Y => CellPoint {
                x: point.x + delta_x,
                y: point.y,
                z: point.z + delta_y,
            },
        }
    }

    fn world_to_plane(self, point: CellPoint) -> (i32, i32, i32) {
        match self.plane_axis {
            CanvasPlaneAxis::Z => (point.x, point.y, point.z),
            CanvasPlaneAxis::X => (point.y, point.z, point.x),
            CanvasPlaneAxis::Y => (point.x, point.z, point.y),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FillConnectivity {
    #[default]
    Cardinal,
    CardinalAndDiagonal,
}

fn neighbors(
    bounds: CanvasBounds,
    point: CellPoint,
    connectivity: FillConnectivity,
) -> Vec<CellPoint> {
    let mut points = vec![
        bounds.offset_in_plane(point, -1, 0),
        bounds.offset_in_plane(point, 1, 0),
        bounds.offset_in_plane(point, 0, -1),
        bounds.offset_in_plane(point, 0, 1),
    ];

    if connectivity == FillConnectivity::CardinalAndDiagonal {
        points.extend([
            bounds.offset_in_plane(point, -1, -1),
            bounds.offset_in_plane(point, 1, -1),
            bounds.offset_in_plane(point, -1, 1),
            bounds.offset_in_plane(point, 1, 1),
        ]);
    }

    points
}

/// Every contiguous point inside `bounds` whose current canvas value matches the starting
/// cell's value (`Some(PaintedCell)` or `None`). Returned in traversal order and suitable for
/// higher-level masked fill behavior that wants to resolve each point separately.
pub fn flood_fill_points(
    canvas: &Canvas,
    start: CellPoint,
    bounds: CanvasBounds,
) -> Vec<CellPoint> {
    flood_fill_points_with_connectivity(canvas, start, bounds, FillConnectivity::Cardinal)
}

pub fn flood_fill_points_with_connectivity(
    canvas: &Canvas,
    start: CellPoint,
    bounds: CanvasBounds,
    connectivity: FillConnectivity,
) -> Vec<CellPoint> {
    if !bounds.contains(start) {
        return Vec::new();
    }

    // Unified empty-cell rule: authored blanks (space glyphs) match `None`,
    // so blank cells never split a flood region from truly empty space.
    let target = effective_cell(canvas.get(&start)).cloned();
    let mut queue = VecDeque::from([start]);
    let mut seen = BTreeSet::new();
    let mut points = Vec::new();

    while let Some(point) = queue.pop_front() {
        if !seen.insert(point) || !bounds.contains(point) {
            continue;
        }
        if effective_cell(canvas.get(&point)).cloned() != target {
            continue;
        }

        points.push(point);
        queue.extend(neighbors(bounds, point, connectivity));
    }

    points
}

/// Flood-fills every contiguous cell inside `bounds` matching the starting
/// cell's current value (`Some(PaintedCell)` or `None`) with `replacement`.
pub fn flood_fill(
    canvas: &mut Canvas,
    start: CellPoint,
    bounds: CanvasBounds,
    replacement: PaintedCell,
) {
    flood_fill_with_connectivity(
        canvas,
        start,
        bounds,
        replacement,
        FillConnectivity::Cardinal,
    );
}

pub fn flood_fill_with_connectivity(
    canvas: &mut Canvas,
    start: CellPoint,
    bounds: CanvasBounds,
    replacement: PaintedCell,
    connectivity: FillConnectivity,
) {
    if canvas.get(&start).cloned() == Some(replacement.clone()) {
        return;
    }

    for point in flood_fill_points_with_connectivity(canvas, start, bounds, connectivity) {
        canvas.insert(point, replacement.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{brush::apply_brush, paint_color::PaintColor};
    use thaum_renderer_domain::CellGraphic;

    fn point(x: i32, y: i32) -> CellPoint {
        CellPoint { x, y, z: 0 }
    }

    fn bounds() -> CanvasBounds {
        CanvasBounds {
            x0: 0,
            y0: 0,
            x1: 4,
            y1: 4,
            z: 0,
            plane_axis: CanvasPlaneAxis::Z,
        }
    }

    fn cell(glyph: char) -> PaintedCell {
        PaintedCell {
            graphic: thaum_renderer_domain::CellGraphic::Glyph(glyph),
            color: PaintColor::flat_rgb(255, 255, 255),
            weight_index: 1,
        }
    }

    #[test]
    fn fills_a_contiguous_empty_region_inside_bounds() {
        let mut canvas = Canvas::new();
        apply_brush(&mut canvas, point(2, 2), cell('#'));

        flood_fill(&mut canvas, point(0, 0), bounds(), cell('.'));

        assert_eq!(canvas.get(&point(0, 0)), Some(&cell('.')));
        assert_eq!(canvas.get(&point(1, 0)), Some(&cell('.')));
        assert_eq!(canvas.get(&point(4, 4)), Some(&cell('.')));
        assert_eq!(canvas.get(&point(2, 2)), Some(&cell('#')));
    }

    #[test]
    fn fills_only_the_matching_contiguous_region() {
        let mut canvas = Canvas::new();
        apply_brush(&mut canvas, point(0, 1), cell('#'));
        apply_brush(&mut canvas, point(1, 1), cell('#'));
        apply_brush(&mut canvas, point(3, 3), cell('#'));

        flood_fill(&mut canvas, point(0, 1), bounds(), cell('@'));

        assert_eq!(canvas.get(&point(0, 1)), Some(&cell('@')));
        assert_eq!(canvas.get(&point(1, 1)), Some(&cell('@')));
        assert_eq!(canvas.get(&point(3, 3)), Some(&cell('#')));
    }

    #[test]
    fn flood_matching_treats_authored_blanks_as_empty() {
        // A stored blank (space glyph + color, e.g. from an old save) must
        // not split a flood region from truly empty space.
        let mut canvas = Canvas::new();
        canvas.insert(
            point(1, 1),
            PaintedCell {
                graphic: CellGraphic::Glyph(' '),
                color: crate::paint_color::PaintColor::flat_rgb(1, 2, 3),
                weight_index: 2,
            },
        );

        let points = flood_fill_points(&canvas, point(0, 0), bounds());

        // The blank at (1,1) floods as part of the empty region.
        assert!(points.contains(&point(1, 1)));
    }

    #[test]
    fn fill_does_nothing_outside_bounds() {
        let mut canvas = Canvas::new();

        flood_fill(
            &mut canvas,
            CellPoint { x: 9, y: 9, z: 0 },
            bounds(),
            cell('@'),
        );

        assert!(canvas.is_empty());
    }

    #[test]
    fn fill_is_a_noop_when_replacement_matches_the_target_cell() {
        let mut canvas = Canvas::new();
        apply_brush(&mut canvas, point(1, 1), cell('#'));

        flood_fill(&mut canvas, point(1, 1), bounds(), cell('#'));

        assert_eq!(canvas.len(), 1);
        assert_eq!(canvas.get(&point(1, 1)), Some(&cell('#')));
    }

    #[test]
    fn diagonal_connectivity_can_cross_corner_neighbors() {
        let mut canvas = Canvas::new();
        apply_brush(&mut canvas, point(0, 0), cell('A'));
        apply_brush(&mut canvas, point(1, 1), cell('A'));

        let cardinal = flood_fill_points_with_connectivity(
            &canvas,
            point(0, 0),
            bounds(),
            FillConnectivity::Cardinal,
        );
        let diagonal = flood_fill_points_with_connectivity(
            &canvas,
            point(0, 0),
            bounds(),
            FillConnectivity::CardinalAndDiagonal,
        );

        assert_eq!(cardinal, vec![point(0, 0)]);
        assert_eq!(diagonal, vec![point(0, 0), point(1, 1)]);
    }

    #[test]
    fn fill_points_return_only_the_target_region() {
        let mut canvas = Canvas::new();
        apply_brush(&mut canvas, point(0, 0), cell('A'));
        apply_brush(&mut canvas, point(1, 0), cell('A'));
        apply_brush(&mut canvas, point(2, 0), cell('B'));

        let points = flood_fill_points(&canvas, point(0, 0), bounds());

        assert_eq!(points, vec![point(0, 0), point(1, 0)]);
    }

    #[test]
    fn x_plane_bounds_treat_yz_as_the_edit_surface() {
        let bounds = CanvasBounds {
            x0: 2,
            y0: 3,
            x1: 4,
            y1: 5,
            z: 7,
            plane_axis: CanvasPlaneAxis::X,
        };

        assert!(bounds.contains(CellPoint { x: 7, y: 2, z: 3 }));
        assert!(bounds.contains(CellPoint { x: 7, y: 4, z: 5 }));
        assert!(!bounds.contains(CellPoint { x: 6, y: 2, z: 3 }));
        assert!(!bounds.contains(CellPoint { x: 7, y: 1, z: 3 }));
        assert_eq!(bounds.plane_point(4, 5), CellPoint { x: 7, y: 4, z: 5 });
    }
}
