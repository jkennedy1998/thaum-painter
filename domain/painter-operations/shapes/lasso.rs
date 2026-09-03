//! Pure lasso (freehand polygon) rasterization for painter authoring.
//! A lasso drag records one closed path of cells; [`lasso_points`] returns
//! every cell inside or on that path so tools can fill the enclosed region.
//! Pure data transform: no session, selection, or history awareness.

use std::collections::BTreeSet;

use thaum_renderer_domain::CellPoint;

/// Rasterizes one closed lasso path into the cells it encloses (interior
/// plus the path itself). The path is treated as implicitly closed (last
/// point back to the first) and rasterized on the path's plane (the first
/// point's z). A degenerate path (fewer than two distinct points) encloses
/// just its own cells, so a lasso click acts on exactly one cell.
pub fn lasso_points(path: &[CellPoint]) -> Vec<CellPoint> {
    let mut vertices: Vec<CellPoint> = path.to_vec();
    vertices.dedup();
    if vertices.len() < 2 {
        return vertices;
    }
    let z = vertices[0].z;
    let mut cells: BTreeSet<(i32, i32)> = BTreeSet::new();

    // The bound itself is always included: every edge, including the
    // implicit closing edge, contributes its rasterized cells.
    for index in 0..vertices.len() {
        let a = &vertices[index];
        let b = &vertices[(index + 1) % vertices.len()];
        for (x, y) in edge_points(a, b) {
            cells.insert((x, y));
        }
    }

    // Interior: even-odd ray cast per cell center, so concave paths and
    // self-adjacent drags behave predictably.
    let min_x = vertices.iter().map(|p| p.x).min().unwrap();
    let max_x = vertices.iter().map(|p| p.x).max().unwrap();
    let min_y = vertices.iter().map(|p| p.y).min().unwrap();
    let max_y = vertices.iter().map(|p| p.y).max().unwrap();
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            if point_inside(x, y, &vertices) {
                cells.insert((x, y));
            }
        }
    }

    cells
        .into_iter()
        .map(|(x, y)| CellPoint { x, y, z })
        .collect()
}

fn point_inside(x: i32, y: i32, vertices: &[CellPoint]) -> bool {
    let center_x = x as f32 + 0.5;
    let center_y = y as f32 + 0.5;
    let mut inside = false;
    for index in 0..vertices.len() {
        let a = &vertices[index];
        let b = &vertices[(index + 1) % vertices.len()];
        if (a.y as f32 > center_y) != (b.y as f32 > center_y) {
            let span_y = b.y as f32 - a.y as f32;
            let edge_x = a.x as f32 + (center_y - a.y as f32) / span_y * (b.x - a.x) as f32;
            if center_x < edge_x {
                inside = !inside;
            }
        }
    }
    inside
}

fn edge_points(a: &CellPoint, b: &CellPoint) -> Vec<(i32, i32)> {
    let steps = (b.x - a.x)
        .abs()
        .max((b.y - a.y).abs())
        .max(1);
    let mut points = Vec::with_capacity(steps as usize + 1);
    for step in 0..=steps {
        let t = step as f32 / steps as f32;
        points.push((
            (a.x as f32 + t * (b.x - a.x) as f32).round() as i32,
            (a.y as f32 + t * (b.y - a.y) as f32).round() as i32,
        ));
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

    #[test]
    fn a_closed_square_encloses_its_interior_and_border() {
        let path = [point(0, 0), point(2, 0), point(2, 2), point(0, 2)];
        let points = lasso_points(&path);
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
        let points = lasso_points(&path);
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
        assert_eq!(lasso_points(&path), vec![CellPoint { x: 4, y: 7, z: 3 }]);
    }

    #[test]
    fn an_empty_path_encloses_nothing() {
        assert!(lasso_points(&[]).is_empty());
    }

    #[test]
    fn a_two_point_path_encloses_the_line_between_them() {
        let path = [point(0, 0), point(2, 0)];
        assert_eq!(set(&lasso_points(&path)), set(&[point(0, 0), point(1, 0), point(2, 0)]));
    }

    #[test]
    fn the_rasterized_plane_comes_from_the_first_vertex() {
        let path = [
            CellPoint { x: 0, y: 0, z: 5 },
            CellPoint { x: 2, y: 2, z: 5 },
        ];
        assert!(lasso_points(&path).iter().all(|p| p.z == 5));
    }
}
