//! Pure lasso (freehand polygon) rasterization for painter authoring.
//! A lasso drag records one closed path of cells; [`lasso_points`] returns
//! every cell inside or on that path so tools can fill the enclosed region.
//! Rasterization runs on the camera's view plane (the drag cells lie on the
//! active focus plane), so the lasso is a 2D tool relative to whatever angle
//! the camera is looking from. Pure data transform: no session, selection,
//! or history awareness.

use std::collections::BTreeSet;

use thaum_renderer_domain::{
    project_world_relative_to_view, unproject_view_relative_to_world, CameraViewOrientation,
    CellPoint, ViewRelativePoint, WorldPoint,
};

/// Rasterizes one closed lasso path into the cells it encloses (interior
/// plus the path itself) on the view plane through the path's first cell.
/// The path is treated as implicitly closed (last point back to the first).
/// A degenerate path (fewer than two distinct points) encloses just its own
/// cells, so a lasso click acts on exactly one cell.
pub fn lasso_points(path: &[CellPoint], orientation: CameraViewOrientation) -> Vec<CellPoint> {
    let mut vertices: Vec<CellPoint> = path.to_vec();
    vertices.dedup();
    if vertices.len() < 2 {
        return vertices;
    }
    let anchor = vertices[0];
    let anchor_world = WorldPoint {
        x: anchor.x,
        y: anchor.y,
        z: anchor.z,
    };

    // Every drag cell lies on the active focus plane, so in view-relative
    // coordinates the bound is a flat (right, up) polygon at the anchor's
    // depth. Rasterizing in that view space keeps the lasso a 2D tool at any
    // camera angle; mapping back with the anchor's depth lands every filled
    // cell on the same focus plane.
    let view_vertices: Vec<(i32, i32)> = vertices
        .iter()
        .map(|vertex| {
            let relative = project_world_relative_to_view(
                orientation,
                anchor_world,
                WorldPoint {
                    x: vertex.x,
                    y: vertex.y,
                    z: vertex.z,
                },
            );
            (relative.right, relative.up)
        })
        .collect();

    rasterize_closed_polygon(&view_vertices)
        .into_iter()
        .map(|(right, up)| {
            let world = unproject_view_relative_to_world(
                orientation,
                anchor_world,
                ViewRelativePoint { right, up, depth: 0 },
            );
            CellPoint {
                x: world.x,
                y: world.y,
                z: world.z,
            }
        })
        .collect()
}

/// Even-odd-free rasterization of one implicitly closed polygon in (right,
/// up) view space: the drawn path acts as a watertight boundary wall and the
/// enclosed region is found by flood-filling from outside the bound. Unlike
/// a parity test, self-crossing freehand paths (pointer jitter, backtrack
/// near the closure) cannot cancel regions — every cell inside the drawn
/// loop fills.
fn rasterize_closed_polygon(vertices: &[(i32, i32)]) -> BTreeSet<(i32, i32)> {
    let mut cells: BTreeSet<(i32, i32)> = BTreeSet::new();

    // The bound itself is always included: every edge, including the
    // implicit closing edge, contributes its interpolated cells.
    for index in 0..vertices.len() {
        let a = &vertices[index];
        let b = &vertices[(index + 1) % vertices.len()];
        for point in edge_points(*a, *b) {
            cells.insert(point);
        }
    }

    let min_x = vertices.iter().map(|p| p.0).min().unwrap();
    let max_x = vertices.iter().map(|p| p.0).max().unwrap();
    let min_y = vertices.iter().map(|p| p.1).min().unwrap();
    let max_y = vertices.iter().map(|p| p.1).max().unwrap();

    // Flood-fill the bounding box from its outer ring; the 4-connected
    // boundary wall keeps the outside from leaking in, so every unreached
    // non-boundary cell is enclosed by the drawn loop.
    let mut outside: BTreeSet<(i32, i32)> = BTreeSet::new();
    let mut frontier: Vec<(i32, i32)> = Vec::new();
    for y in (min_y - 1)..=(max_y + 1) {
        for x in (min_x - 1)..=(max_x + 1) {
            let on_ring = x == min_x - 1 || x == max_x + 1 || y == min_y - 1 || y == max_y + 1;
            if on_ring && !cells.contains(&(x, y)) {
                outside.insert((x, y));
                frontier.push((x, y));
            }
        }
    }
    while let Some((x, y)) = frontier.pop() {
        for neighbor in [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)] {
            if neighbor.0 < min_x - 1
                || neighbor.0 > max_x + 1
                || neighbor.1 < min_y - 1
                || neighbor.1 > max_y + 1
            {
                continue;
            }
            if !cells.contains(&neighbor) && outside.insert(neighbor) {
                frontier.push(neighbor);
            }
        }
    }
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            if !outside.contains(&(x, y)) {
                cells.insert((x, y));
            }
        }
    }

    cells
}

/// Interpolates one edge in 4-connected steps (x and y never advance in the
/// same step), so the drawn boundary is watertight — diagonal-only steps
/// would let the outside flood fill leak through corner gaps.
fn edge_points(a: (i32, i32), b: (i32, i32)) -> Vec<(i32, i32)> {
    let steps = (b.0 - a.0).abs().max((b.1 - a.1).abs()).max(1);
    let mut points = Vec::with_capacity(steps as usize * 2 + 1);
    let mut current = a;
    points.push(current);
    for step in 1..=steps {
        let t = step as f32 / steps as f32;
        let next = (
            (a.0 as f32 + t * (b.0 - a.0) as f32).round() as i32,
            (a.1 as f32 + t * (b.1 - a.1) as f32).round() as i32,
        );
        // Elbow cell keeps each step 4-connected.
        if next.0 != current.0 && next.1 != current.1 {
            current = (next.0, current.1);
            points.push(current);
        }
        current = next;
        points.push(current);
    }
    points
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x: i32, y: i32) -> CellPoint {
        CellPoint { x, y, z: 0 }
    }

    fn set(points: &[CellPoint]) -> BTreeSet<(i32, i32)> {
        points.iter().map(|p| (p.x, p.y)).collect()
    }

    fn flat_view() -> CameraViewOrientation {
        camera_view_orientation_for_camera(CameraSwing::PosZ, CameraRoll::Deg0)
    }

    fn posx_view() -> CameraViewOrientation {
        camera_view_orientation_for_camera(CameraSwing::PosX, CameraRoll::Deg0)
    }

    use thaum_renderer_domain::{camera_view_orientation_for_camera, CameraRoll, CameraSwing};

    #[test]
    fn a_closed_square_encloses_its_interior_and_border() {
        let path = [point(0, 0), point(2, 0), point(2, 2), point(0, 2)];
        let points = lasso_points(&path, flat_view());
        let expected: BTreeSet<(i32, i32)> = (0..=2)
            .flat_map(|y| (0..=2).map(move |x| (x, y)))
            .collect();
        assert_eq!(set(&points), expected);
    }

    #[test]
    fn a_concave_path_excludes_the_notch() {
        // An L of cells minus the notch cell (2, 1).
        let path = [
            point(0, 0),
            point(3, 0),
            point(3, 1),
            point(2, 1),
            point(2, 2),
            point(0, 2),
        ];
        let points = lasso_points(&path, flat_view());
        let cells = set(&points);
        for cell in [(0, 0), (1, 0), (2, 0), (0, 1), (1, 1)] {
            assert!(cells.contains(&cell), "missing L cell {cell:?}");
        }
        assert!(cells.contains(&(2, 1)), "bound cells stay included");
        assert!(!cells.contains(&(3, 2)), "outside cells stay excluded");
        assert!(!cells.contains(&(2, 3)), "outside cells stay excluded");
    }

    #[test]
    fn a_single_point_path_encloses_exactly_that_cell() {
        let path = [CellPoint { x: 4, y: 7, z: 3 }];
        assert_eq!(lasso_points(&path, flat_view()), vec![CellPoint { x: 4, y: 7, z: 3 }]);
    }

    #[test]
    fn an_empty_path_encloses_nothing() {
        assert!(lasso_points(&[], flat_view()).is_empty());
    }

    #[test]
    fn a_two_point_path_encloses_the_line_between_them() {
        let path = [point(0, 0), point(2, 0)];
        assert_eq!(
            set(lasso_points(&path, flat_view()).as_slice()),
            set(&[point(0, 0), point(1, 0), point(2, 0)])
        );
    }

    #[test]
    fn the_rasterized_plane_comes_from_the_first_vertex() {
        let path = [
            CellPoint { x: 0, y: 0, z: 5 },
            CellPoint { x: 2, y: 2, z: 5 },
        ];
        assert!(lasso_points(&path, flat_view()).iter().all(|p| p.z == 5));
    }

    #[test]
    fn a_side_camera_lasso_fills_the_view_plane_not_a_z_plane() {
        // At PosX the view plane is z/y with x constant; a square drawn on
        // the focus plane x=3 must fill z/y cells on that plane, never
        // spread along x the way a PosZ-style rasterization would.
        let path = [
            CellPoint { x: 3, y: 0, z: 0 },
            CellPoint { x: 3, y: 0, z: 2 },
            CellPoint { x: 3, y: 2, z: 2 },
            CellPoint { x: 3, y: 2, z: 0 },
        ];
        let filled = lasso_points(&path, posx_view());
        assert!(filled.iter().all(|p| p.x == 3), "fill stays on the x=3 view plane");
        for cell in [
            CellPoint { x: 3, y: 0, z: 1 },
            CellPoint { x: 3, y: 1, z: 1 },
            CellPoint { x: 3, y: 2, z: 2 },
        ] {
            assert!(filled.contains(&cell), "missing view-plane cell {cell:?}");
        }
    }

    #[test]
    fn a_self_crossing_freehand_circle_still_fills_every_enclosed_cell() {
        // Simulates a real freehand drag: round path with pointer jitter and
        // a backtrack near the closure. Even-odd parity would cancel cells
        // wherever the path crosses itself; the boundary/flood-fill fill
        // must cover every cell inside the drawn loop.
        let mut path = Vec::new();
        let (cx, cy, r) = (20.0f32, 15.0f32, 12.0f32);
        for step in 0..=80 {
            let angle = step as f32 / 80.0 * std::f32::consts::TAU;
            let jitter = if step % 7 == 0 { -1.5 } else { 0.0 };
            path.push(point(
                (cx + (r + jitter) * angle.cos()).round() as i32,
                (cy + (r + jitter) * angle.sin()).round() as i32,
            ));
        }
        // Backtrack segment near the end (pointer hesitates backward).
        for step in (70..=80).rev() {
            let angle = step as f32 / 80.0 * std::f32::consts::TAU;
            path.push(point(
                (cx + r * angle.cos()).round() as i32,
                (cy + r * angle.sin()).round() as i32,
            ));
        }
        let cells: BTreeSet<(i32, i32)> = lasso_points(&path, flat_view())
            .iter()
            .map(|p| (p.x, p.y))
            .collect();
        // Every cell comfortably inside the ideal circle must be filled —
        // jitter dents the boundary but cannot punch holes in the fill.
        for y in 0..30 {
            for x in 0..35 {
                let dx = x as f32 - cx;
                let dy = y as f32 - cy;
                if dx * dx + dy * dy < (r - 2.0) * (r - 2.0) {
                    assert!(
                        cells.contains(&(x, y)),
                        "interior cell ({x}, {y}) of a self-crossing lasso stayed unfilled"
                    );
                }
            }
        }
    }

    #[test]
    fn an_open_loose_end_still_closes_through_the_implicit_edge() {
        // A C-shaped drag whose endpoints are connected by the implicit
        // closing edge encloses its interior.
        let path = [
            point(0, 2),
            point(0, 0),
            point(3, 0),
            point(3, 2),
        ];
        let cells: BTreeSet<(i32, i32)> = lasso_points(&path, flat_view())
            .iter()
            .map(|p| (p.x, p.y))
            .collect();
        assert!(cells.contains(&(1, 1)) && cells.contains(&(2, 1)));
        assert!(!cells.contains(&(0, 3)) && !cells.contains(&(4, 4)));
    }

    #[test]
    fn side_camera_lasso_matches_the_flat_camera_result_rotated() {
        // The same square viewed flat fills x/y at z=0; viewed from PosX the
        // equivalent bound fills z/y at x=0 — one 2D tool, camera-relative.
        let flat_path = [point(0, 0), point(2, 0), point(2, 2), point(0, 2)];
        let flat = lasso_points(&flat_path, flat_view());

        let side_path = [
            CellPoint { x: 0, y: 0, z: 0 },
            CellPoint { x: 0, y: 0, z: 2 },
            CellPoint { x: 0, y: 2, z: 2 },
            CellPoint { x: 0, y: 2, z: 0 },
        ];
        let side = lasso_points(&side_path, posx_view());

        assert_eq!(flat.len(), side.len());
        assert!(flat.iter().all(|p| p.z == 0));
        assert!(side.iter().all(|p| p.x == 0));
    }
}
