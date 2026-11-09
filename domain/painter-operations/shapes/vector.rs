//! Pure vector-point/subpath geometry for painter authoring — the data model
//! behind the vector tool's construction phase and its stroke/fill commit.
//! A point is an anchor cell plus optional forward/backward handle offsets
//! (whole-cell deltas off the anchor, J 2026-11-09: vector geometry always
//! snaps to the existing cell grid); a subpath is an ordered point list that
//! is either open or closed (closed = connecting its own first and last
//! point). Handles author as cubic-bezier control points: each segment
//! between adjacent anchors uses the tail's forward handle and the head's
//! backward handle, and a hard corner (no handle) samples as a straight
//! segment. Pure data transform: no session, selection, or history awareness
//! — the construction session owns which subpaths exist and which point is
//! selected.

use std::collections::BTreeSet;

use thaum_renderer_domain::CellPoint;

/// One authored vector point: an anchor cell plus optional handle offsets.
/// Offsets are whole-cell deltas stored point-relative (J 2026-11-09:
/// "yes they are at whole cell coordinates ... just always on by default bc
/// it should just use the cell grid we already have"), so moving the anchor
/// and re-deriving absolute handle positions preserves handle shapes — the
/// rigid-move guarantee lives in the translate helper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VectorPoint {
    pub anchor: CellPoint,
    pub forward_offset: Option<CellPoint>,
    pub backward_offset: Option<CellPoint>,
}

impl VectorPoint {
    /// A hard-corner point: no handles, straight segments on both sides.
    pub fn hard_corner(anchor: CellPoint) -> Self {
        Self {
            anchor,
            forward_offset: None,
            backward_offset: None,
        }
    }

    /// A handled point with mirrored handles: the backward handle is the
    /// exact opposite of the forward one at authoring time only — later
    /// single-handle edits are independent (J 2026-11-09 press+drag = a
    /// handled point with mirrored forward/backward handles).
    pub fn mirrored(anchor: CellPoint, forward_offset: CellPoint) -> Self {
        Self {
            anchor,
            forward_offset: Some(forward_offset),
            backward_offset: Some(negate(forward_offset)),
        }
    }

    pub fn has_handles(&self) -> bool {
        self.forward_offset.is_some() || self.backward_offset.is_some()
    }

    /// The forward handle's absolute cell, if the point carries one.
    pub fn forward_handle(&self) -> Option<CellPoint> {
        self.forward_offset.map(|offset| add(self.anchor, offset))
    }

    /// The backward handle's absolute cell, if the point carries one.
    pub fn backward_handle(&self) -> Option<CellPoint> {
        self.backward_offset.map(|offset| add(self.anchor, offset))
    }

    /// Rigid move: translates the anchor and both handle offsets by the same
    /// delta, so the point's shape never rotates or scales.
    pub fn translate(&mut self, delta: CellPoint) {
        self.anchor = add(self.anchor, delta);
    }

    /// Collapses both handles to zero magnitude — the double-click gesture's
    /// "back to a hard corner" state.
    pub fn collapse_handles(&mut self) {
        self.forward_offset = None;
        self.backward_offset = None;
    }

    /// Expands both handles to magnitude 1 along `direction` — mirrored, one
    /// cell away and grabbable. The caller picks the direction: the neighbor
    /// tangent when the subpath has an adjacent point, or a fixed axis when
    /// it does not (J 2026-11-09: "1 sure").
    pub fn expand_handles(&mut self, direction: CellPoint) {
        let direction = unit_cell(direction);
        self.forward_offset = Some(direction);
        self.backward_offset = Some(negate(direction));
    }
}

/// One authored subpath: an ordered point list, open or closed. Closed means
/// the subpath connects its own first and last point (a segment runs from
/// the last point back to the first); a closed subpath's points all have two
/// connections, an open subpath's endpoints have one (a single-point subpath
/// has none).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VectorSubpath {
    pub points: Vec<VectorPoint>,
    pub closed: bool,
}

impl VectorSubpath {
    pub fn new(points: Vec<VectorPoint>, closed: bool) -> Self {
        Self { points, closed }
    }

    /// How many subpath edges touch the point at `index`: two when interior
    /// or in a closed subpath, one when an open subpath's end, zero for a
    /// single-point open subpath. The construction session reads this to
    /// decide continue-vs-branch on an empty-cell press (J 2026-11-09).
    pub fn connection_count(&self, index: usize) -> usize {
        if self.points.len() < 2 {
            return 0;
        }
        if self.closed {
            return 2;
        }
        if index == 0 || index + 1 == self.points.len() {
            return 1;
        }
        2
    }

    /// Marks the subpath closed: its own last point connects back to its
    /// first. Idempotent.
    pub fn close(&mut self) {
        self.closed = true;
    }

    /// Marks a closed subpath open again — deleting a closed subpath's tail
    /// point reopens it for a redo (J 2026-11-09: "Then someone could
    /// backspace it and redo that last point").
    pub fn reopen(&mut self) {
        self.closed = false;
    }

    /// Every cell the subpath's curve passes through, in draw order: each
    /// segment between adjacent anchors samples as a straight 3D line when
    /// both ends are hard corners, or as a cubic bezier through the handles
    /// (tail's forward, head's backward). A closed subpath includes the
    /// closing segment. These are the fill's boundary cells and the center
    /// of the stroke band.
    pub fn sample(&self) -> Vec<CellPoint> {
        let mut cells: Vec<CellPoint> = Vec::new();
        let mut seen: BTreeSet<CellPoint> = BTreeSet::new();
        if self.points.len() == 1 {
            return vec![self.points[0].anchor];
        }
        let mut push = |cell: CellPoint, cells: &mut Vec<CellPoint>, seen: &mut BTreeSet<CellPoint>| {
            if seen.insert(cell) {
                cells.push(cell);
            }
        };
        for pair in self.point_pairs() {
            let (from, to) = pair;
            let control_a = from
                .forward_offset
                .map(|offset| add(from.anchor, offset))
                .unwrap_or(from.anchor);
            let control_b = to
                .backward_offset
                .map(|offset| add(to.anchor, offset))
                .unwrap_or(to.anchor);
            if control_a == from.anchor && control_b == to.anchor {
                let segment = sample_line(from.anchor, to.anchor);
                for cell in segment {
                    push(cell, &mut cells, &mut seen);
                }
            } else {
                let segment = sample_bezier(from.anchor, control_a, control_b, to.anchor);
                for cell in segment {
                    push(cell, &mut cells, &mut seen);
                }
            }
        }
        cells
    }

    /// The fill's cells: the closed subpath's boundary only — interiors are
    /// never painted (J 2026-11-09: "fill does the boundary only"). An open
    /// subpath fills nothing.
    pub fn boundary_cells(&self) -> Vec<CellPoint> {
        if !self.closed {
            return Vec::new();
        }
        self.sample()
    }

    /// The stroke's cells: a band of `size` cells wide centered on the
    /// sampled line (J 2026-11-09: "Stroke should be on / centered on the
    /// vector line"), same sizes the cell brush draws at 1..=5. Size 1 is
    /// the line itself.
    pub fn stroke_cells(&self, size: i32) -> Vec<CellPoint> {
        let size = size.clamp(1, 5);
        let left_radius = (size - 1) / 2;
        let right_radius = size - 1 - left_radius;
        let mut cells: BTreeSet<CellPoint> = BTreeSet::new();
        for center in self.sample() {
            for dy in -(left_radius as i64)..=(right_radius as i64) {
                for dx in -(left_radius as i64)..=(right_radius as i64) {
                    if size >= 4 && (dx == -(left_radius as i64) || dx == right_radius as i64)
                        && (dy == -(left_radius as i64) || dy == right_radius as i64)
                    {
                        continue;
                    }
                    cells.insert(CellPoint {
                        x: center.x + dx as i32,
                        y: center.y + dy as i32,
                        z: center.z,
                    });
                }
            }
        }
        cells.into_iter().collect()
    }

    /// Adjacent anchor pairs in draw order, including the closing pair of a
    /// closed subpath.
    fn point_pairs(&self) -> Vec<(&VectorPoint, &VectorPoint)> {
        let mut pairs = Vec::new();
        if self.points.is_empty() {
            return pairs;
        }
        for window in self.points.windows(2) {
            pairs.push((&window[0], &window[1]));
        }
        if self.closed && self.points.len() > 1 {
            let last = self.points.len() - 1;
            pairs.push((&self.points[last], &self.points[0]));
        }
        pairs
    }
}

fn add(point: CellPoint, delta: CellPoint) -> CellPoint {
    CellPoint {
        x: point.x + delta.x,
        y: point.y + delta.y,
        z: point.z + delta.z,
    }
}

fn negate(point: CellPoint) -> CellPoint {
    CellPoint {
        x: -point.x,
        y: -point.y,
        z: -point.z,
    }
}

/// Reduces a delta to its per-axis signs — a magnitude-1 step along the
/// tangent's line, diagonals included.
fn unit_cell(delta: CellPoint) -> CellPoint {
    CellPoint {
        x: delta.x.signum(),
        y: delta.y.signum(),
        z: delta.z.signum(),
    }
}

/// Rasterizes a straight 3D line between two cells (every axis moves at most
/// one step per cell, never skipping a plane) — the hard-corner segment.
fn sample_line(from: CellPoint, to: CellPoint) -> Vec<CellPoint> {
    let from = (from.x as i64, from.y as i64, from.z as i64);
    let to = (to.x as i64, to.y as i64, to.z as i64);
    let dx = (to.0 - from.0).abs();
    let dy = (to.1 - from.1).abs();
    let dz = (to.2 - from.2).abs();
    let steps = dx.max(dy).max(dz);
    let mut cells = Vec::with_capacity(steps as usize + 1);
    for step in 0..=steps {
        let t = if steps == 0 {
            0.0
        } else {
            step as f64 / steps as f64
        };
        cells.push(CellPoint {
            x: (from.0 as f64 + (to.0 - from.0) as f64 * t).round() as i32,
            y: (from.1 as f64 + (to.1 - from.1) as f64 * t).round() as i32,
            z: (from.2 as f64 + (to.2 - from.2) as f64 * t).round() as i32,
        });
    }
    cells
}

/// Samples a cubic bezier between two anchors through two handle cells,
/// dense enough that consecutive samples never skip a cell: the step count
/// follows the control polygon's length.
fn sample_bezier(
    anchor_a: CellPoint,
    control_a: CellPoint,
    control_b: CellPoint,
    anchor_b: CellPoint,
) -> Vec<CellPoint> {
    let at = |t: f64| -> CellPoint {
        let mt = 1.0 - t;
        let b0 = mt * mt * mt;
        let b1 = 3.0 * mt * mt * t;
        let b2 = 3.0 * mt * t * t;
        let b3 = t * t * t;
        let lerp = |a: i32, b: i32, c: i32, d: i32| -> i32 {
            (a as f64 * b0 + b as f64 * b1 + c as f64 * b2 + d as f64 * b3).round() as i32
        };
        CellPoint {
            x: lerp(anchor_a.x, control_a.x, control_b.x, anchor_b.x),
            y: lerp(anchor_a.y, control_a.y, control_b.y, anchor_b.y),
            z: lerp(anchor_a.z, control_a.z, control_b.z, anchor_b.z),
        }
    };
    let polygon_length = |a: CellPoint, b: CellPoint| -> f64 {
        (a.x - b.x).abs() as f64 + (a.y - b.y).abs() as f64 + (a.z - b.z).abs() as f64
    };
    let steps = (polygon_length(anchor_a, control_a)
        + polygon_length(control_a, control_b)
        + polygon_length(control_b, anchor_b))
    .ceil() as i64;
    let steps = steps.clamp(1, 4096);
    (0..=steps)
        .map(|step| at(step as f64 / steps as f64))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(x: i32, y: i32) -> CellPoint {
        CellPoint { x, y, z: 0 }
    }

    #[test]
    fn mirrored_handle_construction_mirrors_at_authoring_time_only() {
        let point = VectorPoint::mirrored(cell(5, 5), cell(2, 1));
        assert_eq!(point.forward_handle(), Some(cell(7, 6)));
        assert_eq!(point.backward_handle(), Some(cell(3, 4)));

        // Later single-handle edits are independent: moving the forward
        // handle does not move the backward one.
        let mut point = point;
        point.forward_offset = Some(cell(3, 0));
        assert_eq!(point.backward_handle(), Some(cell(3, 4)));
    }

    #[test]
    fn rigid_translation_moves_the_anchor_and_both_handles_together() {
        let mut point = VectorPoint::mirrored(cell(5, 5), cell(2, 1));
        let forward_before = point.forward_handle().unwrap();
        let backward_before = point.backward_handle().unwrap();
        point.translate(cell(1, -1));
        assert_eq!(point.anchor, cell(6, 4));
        assert_eq!(
            point.forward_handle().unwrap(),
            add(forward_before, cell(1, -1))
        );
        assert_eq!(
            point.backward_handle().unwrap(),
            add(backward_before, cell(1, -1))
        );
    }

    #[test]
    fn hard_corners_sample_as_straight_segments() {
        let subpath = VectorSubpath::new(
            vec![VectorPoint::hard_corner(cell(0, 0)), VectorPoint::hard_corner(cell(3, 0))],
            false,
        );
        assert_eq!(
            subpath.sample(),
            vec![cell(0, 0), cell(1, 0), cell(2, 0), cell(3, 0)]
        );
    }

    #[test]
    fn handles_bend_the_segment_through_the_control_cells() {
        // Forward handle pulls the segment up before arriving at the next
        // hard-corner anchor: the sampled curve passes near the control.
        let subpath = VectorSubpath::new(
            vec![
                VectorPoint::mirrored(cell(0, 0), cell(4, 4)),
                VectorPoint::hard_corner(cell(8, 0)),
            ],
            false,
        );
        let cells = subpath.sample();
        assert!(cells.contains(&cell(0, 0)));
        assert!(cells.contains(&cell(8, 0)));
        // The curve leaves the first anchor along its handle direction, so
        // it gains height a straight line would never have.
        assert!(
            cells.iter().any(|c| c.y > 0),
            "the forward handle bends the segment off the straight line"
        );
    }

    #[test]
    fn a_degenerate_single_point_path_samples_only_itself() {
        let subpath = VectorSubpath::new(vec![VectorPoint::hard_corner(cell(2, 3))], false);
        assert_eq!(subpath.sample(), vec![cell(2, 3)]);
        assert_eq!(subpath.connection_count(0), 0);
        assert!(subpath.boundary_cells().is_empty(), "open fills nothing");
        assert_eq!(subpath.stroke_cells(1), vec![cell(2, 3)]);
    }

    #[test]
    fn connection_counts_follow_open_and_closed_shapes() {
        let mut subpath = VectorSubpath::new(
            vec![
                VectorPoint::hard_corner(cell(0, 0)),
                VectorPoint::hard_corner(cell(2, 0)),
                VectorPoint::hard_corner(cell(4, 0)),
            ],
            false,
        );
        assert_eq!(subpath.connection_count(0), 1);
        assert_eq!(subpath.connection_count(1), 2);
        assert_eq!(subpath.connection_count(2), 1);
        subpath.close();
        assert_eq!(subpath.connection_count(0), 2);
        assert_eq!(subpath.connection_count(2), 2);
        subpath.reopen();
        assert_eq!(subpath.connection_count(2), 1);
    }

    #[test]
    fn a_closed_subpath_includes_its_closing_segment() {
        let subpath = VectorSubpath::new(
            vec![VectorPoint::hard_corner(cell(0, 0)), VectorPoint::hard_corner(cell(4, 0))],
            true,
        );
        let cells = subpath.sample();
        assert!(cells.contains(&cell(0, 0)));
        assert!(cells.contains(&cell(2, 0)));
        assert!(cells.contains(&cell(4, 0)));
        assert_eq!(subpath.boundary_cells(), cells, "fill is the boundary only");
    }

    #[test]
    fn an_open_subpath_never_fills() {
        let subpath = VectorSubpath::new(
            vec![VectorPoint::hard_corner(cell(0, 0)), VectorPoint::hard_corner(cell(4, 0))],
            false,
        );
        assert!(subpath.boundary_cells().is_empty());
        assert!(!subpath.stroke_cells(1).is_empty(), "stroke still draws");
    }

    #[test]
    fn handle_collapse_and_expand_toggle_the_grabbable_state() {
        let mut point = VectorPoint::hard_corner(cell(2, 2));
        assert!(!point.has_handles());
        point.expand_handles(cell(3, 3));
        assert!(point.has_handles());
        assert_eq!(point.forward_handle(), Some(cell(3, 3)));
        assert_eq!(point.backward_handle(), Some(cell(1, 1)));
        point.collapse_handles();
        assert!(!point.has_handles());
        assert_eq!(point.forward_handle(), None);
        assert_eq!(point.backward_handle(), None);
    }

    #[test]
    fn stroke_band_width_follows_the_size_and_stays_centered() {
        let subpath = VectorSubpath::new(
            vec![VectorPoint::hard_corner(cell(0, 0)), VectorPoint::hard_corner(cell(4, 0))],
            false,
        );
        // Size 1 is exactly the line.
        assert_eq!(
            subpath.stroke_cells(1),
            vec![cell(0, 0), cell(1, 0), cell(2, 0), cell(3, 0), cell(4, 0)]
        );
        // Size 3 is the line plus one cell on each side, still centered.
        let band: BTreeSet<CellPoint> = subpath.stroke_cells(3).into_iter().collect();
        assert!(band.contains(&cell(2, 1)));
        assert!(band.contains(&cell(2, 0)));
        assert!(band.contains(&cell(2, -1)));
        assert_eq!(band.len(), 21, "the centered band runs 7 columns x 3 wide");
        // Size 4 drops the corners but is strictly wider than size 3.
        let band4: BTreeSet<CellPoint> = subpath.stroke_cells(4).into_iter().collect();
        assert!(band4.len() > band.len(), "size 4 covers more than size 3");
    }
}
