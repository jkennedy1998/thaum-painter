use std::collections::{BTreeSet, VecDeque};

use thaum_renderer_domain::CellPoint;

use crate::brush::{effective_cell, Canvas, PaintedCell};

/// Which cell channels a flood compares when deciding whether a neighbor
/// belongs to the same region. J 2026-09-10: the fill tool's region sensing
/// follows the hand's Select row — toggled-off channels are ignored in the
/// comparison, so a graphic difference alone no longer splits a region when
/// the graphic channel is unlocked for selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FillChannelMask {
    pub graphic: bool,
    pub color: bool,
    pub weight: bool,
}

impl FillChannelMask {
    pub fn all() -> Self {
        Self {
            graphic: true,
            color: true,
            weight: true,
        }
    }

    pub fn any_enabled(&self) -> bool {
        self.graphic || self.color || self.weight
    }
}

fn cells_match_on_mask(
    left: Option<PaintedCell>,
    right: Option<PaintedCell>,
    mask: FillChannelMask,
) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => {
            (!mask.graphic || left.graphic == right.graphic)
                && (!mask.color || left.color == right.color)
                && (!mask.weight || left.weight_index == right.weight_index)
        }
        _ => false,
    }
}

/// How far a flood may step between matching cells. J 2026-10-10: the fill
/// tool's reach is a three-way choice — `Adjacent` (cardinal neighbors),
/// `Diagonal` (every in-plane neighbor incl. corners), and `All`, which drops
/// contiguity entirely and takes every matching cell in the region.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FillConnectivity {
    #[default]
    Adjacent,
    Diagonal,
    All,
}

impl FillConnectivity {
    /// Stable persistence id (user session state, tool presets).
    pub fn id(self) -> &'static str {
        match self {
            Self::Adjacent => "adjacent",
            Self::Diagonal => "diagonal",
            Self::All => "all",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "adjacent" => Some(Self::Adjacent),
            "diagonal" => Some(Self::Diagonal),
            "all" => Some(Self::All),
            _ => None,
        }
    }
}

/// Whether a fill stays on its interaction plane or reaches through depth —
/// the world axis the camera views along (J 2026-10-10: a 3d toggle lets the
/// fill act across multiple 3d depth layers relative to the viewing angle).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FillDepthMode {
    #[default]
    Plane,
    ThroughDepth,
}

/// The finite span of depth layers a through-depth fill or lasso acts on:
/// the canvas's content extent along `depth_axis` (the camera's view axis,
/// `CanvasPlaneAxis`), always extended to include the anchor point's own
/// depth. The live canvas is logically unbounded, so content is what bounds
/// a depth reach — every 3d edit region stays finite.
pub fn depth_span_for_canvas(
    canvas: &Canvas,
    depth_axis: CanvasPlaneAxis,
    anchor: CellPoint,
) -> (i32, i32) {
    let anchor_depth = depth_coordinate(depth_axis, anchor);
    let mut span = (anchor_depth, anchor_depth);
    for point in canvas.keys() {
        let depth = depth_coordinate(depth_axis, *point);
        span.0 = span.0.min(depth);
        span.1 = span.1.max(depth);
    }
    span
}

fn depth_coordinate(depth_axis: CanvasPlaneAxis, point: CellPoint) -> i32 {
    match depth_axis {
        CanvasPlaneAxis::Z => point.z,
        CanvasPlaneAxis::X => point.x,
        CanvasPlaneAxis::Y => point.y,
    }
}

fn with_depth_coordinate(depth_axis: CanvasPlaneAxis, point: CellPoint, depth: i32) -> CellPoint {
    match depth_axis {
        CanvasPlaneAxis::Z => CellPoint { z: depth, ..point },
        CanvasPlaneAxis::X => CellPoint { x: depth, ..point },
        CanvasPlaneAxis::Y => CellPoint { y: depth, ..point },
    }
}

/// Finite fill bounds for painter flood fill. The live canvas is logically
/// unbounded, so fill needs the caller to declare the region it is allowed to
/// traverse. The plane axis doubles as the depth axis for through-depth
/// fills: it is the world axis the camera views along.
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

fn region_contains(
    bounds: CanvasBounds,
    depth: FillDepthMode,
    depth_span: (i32, i32),
    point: CellPoint,
) -> bool {
    match depth {
        FillDepthMode::Plane => bounds.contains(point),
        FillDepthMode::ThroughDepth => {
            let (plane_x, plane_y, _) = bounds.world_to_plane(point);
            plane_x >= bounds.x0
                && plane_x <= bounds.x1
                && plane_y >= bounds.y0
                && plane_y <= bounds.y1
                && depth_coordinate(bounds.plane_axis, point) >= depth_span.0
                && depth_coordinate(bounds.plane_axis, point) <= depth_span.1
        }
    }
}

fn in_plane_neighbors(bounds: CanvasBounds, point: CellPoint, diagonal: bool) -> Vec<CellPoint> {
    let mut points = vec![
        bounds.offset_in_plane(point, -1, 0),
        bounds.offset_in_plane(point, 1, 0),
        bounds.offset_in_plane(point, 0, -1),
        bounds.offset_in_plane(point, 0, 1),
    ];

    if diagonal {
        points.extend([
            bounds.offset_in_plane(point, -1, -1),
            bounds.offset_in_plane(point, 1, -1),
            bounds.offset_in_plane(point, -1, 1),
            bounds.offset_in_plane(point, 1, 1),
        ]);
    }

    points
}

fn neighbors(
    bounds: CanvasBounds,
    point: CellPoint,
    connectivity: FillConnectivity,
    depth: FillDepthMode,
) -> Vec<CellPoint> {
    match (depth, connectivity) {
        (FillDepthMode::ThroughDepth, FillConnectivity::Adjacent) => {
            let mut points = in_plane_neighbors(bounds, point, false);
            let plane_axis = bounds.plane_axis;
            points.push(with_depth_coordinate(
                plane_axis,
                point,
                depth_coordinate(plane_axis, point) - 1,
            ));
            points.push(with_depth_coordinate(
                plane_axis,
                point,
                depth_coordinate(plane_axis, point) + 1,
            ));
            points
        }
        // Full cube neighborhood: every (dx, dy, dz) in {-1,0,1}³ except the
        // point itself — the 3d analog of the plane's 8-neighbor diagonal.
        (FillDepthMode::ThroughDepth, FillConnectivity::Diagonal) => {
            let plane_axis = bounds.plane_axis;
            let base_depth = depth_coordinate(plane_axis, point);
            let mut points = Vec::with_capacity(26);
            for depth_delta in [-1, 0, 1] {
                let shifted = with_depth_coordinate(plane_axis, point, base_depth + depth_delta);
                for delta_y in [-1, 0, 1] {
                    for delta_x in [-1, 0, 1] {
                        let candidate = bounds.offset_in_plane(shifted, delta_x, delta_y);
                        if candidate != point {
                            points.push(candidate);
                        }
                    }
                }
            }
            points
        }
        (FillDepthMode::Plane, FillConnectivity::Adjacent) => {
            in_plane_neighbors(bounds, point, false)
        }
        (FillDepthMode::Plane, FillConnectivity::Diagonal) => {
            in_plane_neighbors(bounds, point, true)
        }
        // `All` is non-continuous: it never walks neighbors, it scans the
        // whole region, so this arm is unreachable.
        (_, FillConnectivity::All) => Vec::new(),
    }
}

/// Every region cell matching the starting cell's value. `All` scans the
/// whole region without contiguity; the other modes flood from `start`.
fn matching_region_points(
    canvas: &Canvas,
    start: CellPoint,
    bounds: CanvasBounds,
    connectivity: FillConnectivity,
    mask: FillChannelMask,
    depth: FillDepthMode,
    depth_span: (i32, i32),
    target: Option<PaintedCell>,
) -> Vec<CellPoint> {
    if connectivity == FillConnectivity::All {
        let mut points = Vec::new();
        let depths: Vec<i32> = match depth {
            FillDepthMode::Plane => vec![bounds.z],
            FillDepthMode::ThroughDepth => (depth_span.0..=depth_span.1).collect(),
        };
        for depth_value in depths {
            for plane_y in bounds.y0..=bounds.y1 {
                for plane_x in bounds.x0..=bounds.x1 {
                    let point = with_depth_coordinate(
                        bounds.plane_axis,
                        bounds.plane_point(plane_x, plane_y),
                        depth_value,
                    );
                    if cells_match_on_mask(effective_cell(canvas.get(&point)).cloned(), target.clone(), mask)
                    {
                        points.push(point);
                    }
                }
            }
        }
        return points;
    }

    let mut queue = VecDeque::from([start]);
    let mut seen = BTreeSet::new();
    let mut points = Vec::new();

    while let Some(point) = queue.pop_front() {
        if !seen.insert(point) || !region_contains(bounds, depth, depth_span, point) {
            continue;
        }
        if !cells_match_on_mask(
            effective_cell(canvas.get(&point)).cloned(),
            target.clone(),
            mask,
        ) {
            continue;
        }

        points.push(point);
        queue.extend(neighbors(bounds, point, connectivity, depth));
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
    flood_fill_points_with(
        canvas,
        start,
        bounds,
        FillConnectivity::Adjacent,
        FillChannelMask::all(),
        FillDepthMode::Plane,
    )
}

/// Full-control flood region sensing: connectivity (`Adjacent`/`Diagonal`/
/// `All`), channel mask, and whether the region stays on the interaction
/// plane or reaches through the camera's depth axis. Through-depth spans
/// come from the canvas's own content extent, so the region stays finite.
pub fn flood_fill_points_with(
    canvas: &Canvas,
    start: CellPoint,
    bounds: CanvasBounds,
    connectivity: FillConnectivity,
    mask: FillChannelMask,
    depth: FillDepthMode,
) -> Vec<CellPoint> {
    if !mask.any_enabled() {
        return Vec::new();
    }

    let depth_span = match depth {
        FillDepthMode::Plane => (bounds.z, bounds.z),
        FillDepthMode::ThroughDepth => {
            depth_span_for_canvas(canvas, bounds.plane_axis, start)
        }
    };
    if !region_contains(bounds, depth, depth_span, start) {
        return Vec::new();
    }

    // Unified empty-cell rule: authored blanks (space glyphs) match `None`,
    // so blank cells never split a flood region from truly empty space.
    let target = effective_cell(canvas.get(&start)).cloned();
    matching_region_points(
        canvas,
        start,
        bounds,
        connectivity,
        mask,
        depth,
        depth_span,
        target,
    )
}

/// Flood-fills every contiguous cell inside `bounds` matching the starting
/// cell's current value (`Some(PaintedCell)` or `None`) with `replacement`.
pub fn flood_fill(
    canvas: &mut Canvas,
    start: CellPoint,
    bounds: CanvasBounds,
    replacement: PaintedCell,
) {
    flood_fill_with(
        canvas,
        start,
        bounds,
        replacement,
        FillConnectivity::Adjacent,
        FillChannelMask::all(),
        FillDepthMode::Plane,
    );
}

pub fn flood_fill_with(
    canvas: &mut Canvas,
    start: CellPoint,
    bounds: CanvasBounds,
    replacement: PaintedCell,
    connectivity: FillConnectivity,
    mask: FillChannelMask,
    depth: FillDepthMode,
) {
    if canvas.get(&start).cloned() == Some(replacement.clone()) {
        return;
    }

    for point in flood_fill_points_with(canvas, start, bounds, connectivity, mask, depth) {
        canvas.insert(point, replacement.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{brush::apply_brush, paint_color::PaintColor};
    use thaum_renderer_domain::{CellFacing, CellGraphic};

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
            facing: CellFacing::PosZ,
            graphic: thaum_renderer_domain::CellGraphic::Glyph(glyph),
            color: PaintColor::flat_rgb(255, 255, 255),
            weight_index: 1,
            shader_stack: Vec::new(),
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
                facing: CellFacing::PosZ,
                graphic: CellGraphic::Glyph(' '),
                color: crate::paint_color::PaintColor::flat_rgb(1, 2, 3),
                weight_index: 2,
                shader_stack: Vec::new(),
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

        let cardinal = flood_fill_points_with(
            &canvas,
            point(0, 0),
            bounds(),
            FillConnectivity::Adjacent,
            FillChannelMask::all(),
            FillDepthMode::Plane,
        );
        let diagonal = flood_fill_points_with(
            &canvas,
            point(0, 0),
            bounds(),
            FillConnectivity::Diagonal,
            FillChannelMask::all(),
            FillDepthMode::Plane,
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
    fn all_connectivity_takes_every_matching_cell_without_contiguity() {
        // Two same-glyph islands with no connecting path: `All` reaches both,
        // while a contiguous flood from (0, 0) stays on the first island.
        let mut canvas = Canvas::new();
        apply_brush(&mut canvas, point(0, 0), cell('A'));
        apply_brush(&mut canvas, point(3, 3), cell('A'));
        apply_brush(&mut canvas, point(1, 0), cell('B'));

        let all = flood_fill_points_with(
            &canvas,
            point(0, 0),
            bounds(),
            FillConnectivity::All,
            FillChannelMask::all(),
            FillDepthMode::Plane,
        );
        let adjacent = flood_fill_points_with(
            &canvas,
            point(0, 0),
            bounds(),
            FillConnectivity::Adjacent,
            FillChannelMask::all(),
            FillDepthMode::Plane,
        );

        assert!(all.contains(&point(3, 3)), "All crosses the B wall");
        assert_eq!(all.len(), 2);
        assert_eq!(adjacent, vec![point(0, 0)]);
    }

    #[test]
    fn all_connectivity_also_matches_empties_across_the_region() {
        let mut canvas = Canvas::new();
        apply_brush(&mut canvas, point(2, 2), cell('A'));

        let points = flood_fill_points_with(
            &canvas,
            point(0, 0),
            bounds(),
            FillConnectivity::All,
            FillChannelMask::all(),
            FillDepthMode::Plane,
        );

        // Every cell of the plane rect except the 'A' at (2, 2).
        assert_eq!(points.len(), bounds().iter_points().len() - 1);
        assert!(!points.contains(&point(2, 2)));
    }

    #[test]
    fn through_depth_adjacency_steps_across_depth_layers() {
        let mut canvas = Canvas::new();
        // The same glyph stacked at z=0 and z=1 under the cursor, plus an
        // unrelated glyph one plane over at z=1.
        apply_brush(&mut canvas, point(0, 0), cell('A'));
        apply_brush(&mut canvas, CellPoint { x: 0, y: 0, z: 1 }, cell('A'));
        apply_brush(&mut canvas, CellPoint { x: 1, y: 0, z: 1 }, cell('B'));

        let plane = flood_fill_points_with(
            &canvas,
            point(0, 0),
            bounds(),
            FillConnectivity::Adjacent,
            FillChannelMask::all(),
            FillDepthMode::Plane,
        );
        let through = flood_fill_points_with(
            &canvas,
            point(0, 0),
            bounds(),
            FillConnectivity::Adjacent,
            FillChannelMask::all(),
            FillDepthMode::ThroughDepth,
        );

        assert_eq!(plane, vec![point(0, 0)]);
        assert_eq!(
            through,
            vec![point(0, 0), CellPoint { x: 0, y: 0, z: 1 }]
        );
    }

    #[test]
    fn through_depth_diagonal_reaches_the_full_cube_neighborhood() {
        let mut canvas = Canvas::new();
        // Seed cell at the start, a corner diagonal in x/y at the same
        // depth, and an orthogonal neighbor one depth plane over: the
        // 26-neighbor diagonal reaches both.
        apply_brush(&mut canvas, point(0, 0), cell('A'));
        apply_brush(&mut canvas, point(1, 1), cell('A'));
        apply_brush(&mut canvas, CellPoint { x: 0, y: 0, z: 1 }, cell('A'));

        let points = flood_fill_points_with(
            &canvas,
            point(0, 0),
            bounds(),
            FillConnectivity::Diagonal,
            FillChannelMask::all(),
            FillDepthMode::ThroughDepth,
        );

        assert!(points.contains(&point(1, 1)));
        assert!(points.contains(&CellPoint { x: 0, y: 0, z: 1 }));
        assert_eq!(points.len(), 3);
    }

    #[test]
    fn through_depth_all_scans_the_content_depth_span() {
        let mut canvas = Canvas::new();
        apply_brush(&mut canvas, point(0, 0), cell('A'));
        // Content at z=2 and z=-1 defines the span; a matching cell at z=2
        // outside the plane rect stays out, one inside comes in.
        apply_brush(&mut canvas, CellPoint { x: 5, y: 5, z: 2 }, cell('C'));
        apply_brush(&mut canvas, CellPoint { x: 0, y: 0, z: 2 }, cell('A'));
        apply_brush(&mut canvas, CellPoint { x: 1, y: 1, z: -1 }, cell('A'));

        let points = flood_fill_points_with(
            &canvas,
            point(0, 0),
            bounds(),
            FillConnectivity::All,
            FillChannelMask::all(),
            FillDepthMode::ThroughDepth,
        );

        assert!(points.contains(&CellPoint { x: 0, y: 0, z: 2 }));
        assert!(points.contains(&CellPoint { x: 1, y: 1, z: -1 }));
        assert_eq!(points.len(), 3);
    }

    #[test]
    fn depth_span_tracks_the_canvas_content_extent_along_the_depth_axis() {
        let mut canvas = Canvas::new();
        apply_brush(&mut canvas, CellPoint { x: 3, y: 3, z: -2 }, cell('A'));
        apply_brush(&mut canvas, CellPoint { x: 1, y: 1, z: 4 }, cell('A'));

        assert_eq!(
            depth_span_for_canvas(&canvas, CanvasPlaneAxis::Z, point(0, 0)),
            (-2, 4)
        );
        // An empty canvas spans exactly the anchor's depth.
        assert_eq!(
            depth_span_for_canvas(&Canvas::new(), CanvasPlaneAxis::Z, point(0, 0)),
            (0, 0)
        );
        // The x axis is the depth axis for an x-plane camera view.
        let mut side_canvas = Canvas::new();
        apply_brush(&mut side_canvas, CellPoint { x: -3, y: 0, z: 0 }, cell('A'));
        assert_eq!(
            depth_span_for_canvas(&side_canvas, CanvasPlaneAxis::X, point(0, 0)),
            (-3, 0)
        );
    }

    #[test]
    fn fill_connectivity_ids_round_trip() {
        for connectivity in [
            FillConnectivity::Adjacent,
            FillConnectivity::Diagonal,
            FillConnectivity::All,
        ] {
            assert_eq!(
                FillConnectivity::from_id(connectivity.id()),
                Some(connectivity)
            );
        }
        assert_eq!(FillConnectivity::from_id("bogus"), None);
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
