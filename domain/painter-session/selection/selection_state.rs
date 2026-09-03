use std::collections::{BTreeSet, VecDeque};

use thaum_renderer_domain::CellPoint;

use crate::{
    brush::{Canvas, PaintedCell},
    fill::CanvasBounds,
    tool_state::ChannelMask,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SelectionMode {
    #[default]
    Replace,
    Additive,
    Subtract,
    Intersect,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaneSelection {
    /// The active interaction plane. Bounds gate where strokes/select-all/invert
    /// land, but the cell set itself is 3D: cells selected on other planes or at
    /// other depths stay selected and keep rendering across camera moves.
    bounds: CanvasBounds,
    cells: BTreeSet<CellPoint>,
}

impl PlaneSelection {
    pub fn new(bounds: CanvasBounds) -> Self {
        Self {
            bounds,
            cells: BTreeSet::new(),
        }
    }

    pub fn bounds(&self) -> CanvasBounds {
        self.bounds
    }

    /// Retargets the active interaction plane without touching selected cells —
    /// selections are 3D and survive camera plane changes. New strokes and
    /// plane-scoped commands land on the new bounds instead.
    pub fn set_bounds(&mut self, bounds: CanvasBounds) {
        self.bounds = bounds;
    }

    pub fn clear(&mut self) {
        self.cells.clear();
    }

    /// Selects the whole active interaction plane, keeping any cells already
    /// selected outside it (other depths/planes).
    pub fn select_all(&mut self) {
        self.cells.extend(self.bounds.iter_points());
    }

    /// Flips membership inside the active interaction plane only; cells selected
    /// outside it (other depths/planes) are untouched.
    pub fn invert(&mut self) {
        let previous = self.cells.clone();
        for point in self.bounds.iter_points() {
            if previous.contains(&point) {
                self.cells.remove(&point);
            } else {
                self.cells.insert(point);
            }
        }
    }

    pub fn has_selection(&self) -> bool {
        !self.cells.is_empty()
    }

    pub fn contains(&self, point: CellPoint) -> bool {
        self.cells.contains(&point)
    }

    pub fn allows_edit(&self, point: CellPoint) -> bool {
        !self.has_selection() || self.contains(point)
    }

    pub fn filter_edit_points<I>(&self, points: I) -> Vec<CellPoint>
    where
        I: IntoIterator<Item = CellPoint>,
    {
        points
            .into_iter()
            .filter(|point| self.bounds.contains(*point))
            .filter(|point| self.allows_edit(*point))
            .collect()
    }

    pub fn set_selected(&mut self, point: CellPoint, selected: bool) {
        if !self.bounds.contains(point) {
            return;
        }
        if selected {
            self.cells.insert(point);
        } else {
            self.cells.remove(&point);
        }
    }

    pub fn apply_points<I>(&mut self, points: I, mode: SelectionMode)
    where
        I: IntoIterator<Item = CellPoint>,
    {
        let incoming: BTreeSet<CellPoint> = points
            .into_iter()
            .filter(|point| self.bounds.contains(*point))
            .collect();
        // Every mode is plane-scoped: cells selected outside the active
        // interaction plane (other depths/planes) always survive.
        let outside = self
            .cells
            .iter()
            .filter(|point| !self.bounds.contains(**point))
            .copied()
            .collect::<BTreeSet<CellPoint>>();

        match mode {
            SelectionMode::Replace => {
                self.cells = outside;
                self.cells.extend(incoming);
            }
            SelectionMode::Additive => self.cells.extend(incoming),
            SelectionMode::Subtract => {
                for point in incoming {
                    self.cells.remove(&point);
                }
            }
            SelectionMode::Intersect => {
                self.cells = outside
                    .union(
                        &self
                            .cells
                            .intersection(&incoming)
                            .copied()
                            .collect::<BTreeSet<CellPoint>>(),
                    )
                    .copied()
                    .collect();
            }
        }
    }

    /// Inserts points without plane filtering. Used by boot-time restore from the
    /// document's selection channel, which legitimately holds cells outside the
    /// current interaction plane.
    pub fn restore_points<I>(&mut self, points: I)
    where
        I: IntoIterator<Item = CellPoint>,
    {
        self.cells.extend(points);
    }

    pub fn iter(&self) -> impl Iterator<Item = CellPoint> + '_ {
        self.cells.iter().copied()
    }

    pub fn is_border(&self, point: CellPoint) -> bool {
        if !self.contains(point) {
            return false;
        }
        plane_neighbors(self.bounds, point)
            .into_iter()
            .any(|neighbor| !self.bounds.contains(neighbor) || !self.contains(neighbor))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorldSelection {
    cells: BTreeSet<CellPoint>,
}

impl WorldSelection {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.cells.clear();
    }

    pub fn has_selection(&self) -> bool {
        !self.cells.is_empty()
    }

    pub fn contains(&self, point: CellPoint) -> bool {
        self.cells.contains(&point)
    }

    pub fn set_selected(&mut self, point: CellPoint, selected: bool) {
        if selected {
            self.cells.insert(point);
        } else {
            self.cells.remove(&point);
        }
    }

    pub fn apply_points<I>(&mut self, points: I, mode: SelectionMode)
    where
        I: IntoIterator<Item = CellPoint>,
    {
        let incoming: BTreeSet<CellPoint> = points.into_iter().collect();
        match mode {
            SelectionMode::Replace => self.cells = incoming,
            SelectionMode::Additive => self.cells.extend(incoming),
            SelectionMode::Subtract => {
                for point in incoming {
                    self.cells.remove(&point);
                }
            }
            SelectionMode::Intersect => {
                self.cells = self.cells.intersection(&incoming).copied().collect();
            }
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = CellPoint> + '_ {
        self.cells.iter().copied()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PainterSelection {
    plane: PlaneSelection,
    world: WorldSelection,
    mode: SelectionMode,
}

impl PainterSelection {
    pub fn new(bounds: CanvasBounds) -> Self {
        Self {
            plane: PlaneSelection::new(bounds),
            world: WorldSelection::new(),
            mode: SelectionMode::Replace,
        }
    }

    pub fn plane(&self) -> &PlaneSelection {
        &self.plane
    }

    pub fn world(&self) -> &WorldSelection {
        &self.world
    }

    pub fn mode(&self) -> SelectionMode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: SelectionMode) {
        self.mode = mode;
    }

    pub fn set_plane_bounds(&mut self, bounds: CanvasBounds) {
        self.plane.set_bounds(bounds);
    }

    pub fn clear_plane(&mut self) {
        self.plane.clear();
    }

    pub fn select_all_plane(&mut self) {
        self.plane.select_all();
    }

    pub fn invert_plane(&mut self) {
        self.plane.invert();
    }

    pub fn apply_plane_points<I>(&mut self, points: I)
    where
        I: IntoIterator<Item = CellPoint>,
    {
        self.plane.apply_points(points, self.mode);
    }

    pub fn apply_plane_points_with_mode<I>(&mut self, points: I, mode: SelectionMode)
    where
        I: IntoIterator<Item = CellPoint>,
    {
        self.plane.apply_points(points, mode);
    }

    pub fn preview_plane_with_mode<I>(&self, points: I, mode: SelectionMode) -> PlaneSelection
    where
        I: IntoIterator<Item = CellPoint>,
    {
        let mut preview = self.plane.clone();
        preview.apply_points(points, mode);
        preview
    }

    pub fn allows_plane_edit(&self, point: CellPoint) -> bool {
        self.plane.allows_edit(point)
    }

    pub fn filter_plane_edit_points<I>(&self, points: I) -> Vec<CellPoint>
    where
        I: IntoIterator<Item = CellPoint>,
    {
        self.plane.filter_edit_points(points)
    }

    pub fn apply_world_points<I>(&mut self, points: I)
    where
        I: IntoIterator<Item = CellPoint>,
    {
        self.world.apply_points(points, self.mode);
    }

    pub fn apply_world_points_with_mode<I>(&mut self, points: I, mode: SelectionMode)
    where
        I: IntoIterator<Item = CellPoint>,
    {
        self.world.apply_points(points, mode);
    }

    /// Restores selection cells from the document's selection channel at boot.
    /// Not plane-filtered: the channel is 3D and legitimately holds cells outside
    /// the current interaction plane.
    pub fn restore_points<I>(&mut self, points: I)
    where
        I: IntoIterator<Item = CellPoint>,
    {
        self.plane.restore_points(points);
    }

    /// Replaces the whole mirrored selection cache with `points`. Used to resync
    /// the interaction surface from the document-owned channel after the runtime
    /// reloads another writer's version of the document.
    pub fn replace_points<I>(&mut self, points: I)
    where
        I: IntoIterator<Item = CellPoint>,
    {
        self.clear_plane();
        self.restore_points(points);
    }
}

fn plane_neighbors(bounds: CanvasBounds, point: CellPoint) -> Vec<CellPoint> {
    vec![
        bounds.offset_in_plane(point, -1, 0),
        bounds.offset_in_plane(point, 1, 0),
        bounds.offset_in_plane(point, 0, -1),
        bounds.offset_in_plane(point, 0, 1),
    ]
}

fn cells_match_on_mask(
    left: Option<PaintedCell>,
    right: Option<PaintedCell>,
    channels: ChannelMask,
) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => {
            (!channels.graphic || left.graphic == right.graphic)
                && (!channels.color || left.color == right.color)
                && (!channels.weight || left.weight_index == right.weight_index)
        }
        _ => false,
    }
}

pub fn flood_select_points(
    canvas: &Canvas,
    start: CellPoint,
    bounds: CanvasBounds,
    channels: ChannelMask,
) -> Vec<CellPoint> {
    if !bounds.contains(start) || !channels.any_enabled() {
        return Vec::new();
    }

    let target = canvas.get(&start).cloned();
    let mut queue = VecDeque::from([start]);
    let mut seen = BTreeSet::new();
    let mut points = Vec::new();

    while let Some(point) = queue.pop_front() {
        if !seen.insert(point) || !bounds.contains(point) {
            continue;
        }
        if !cells_match_on_mask(canvas.get(&point).cloned(), target.clone(), channels) {
            continue;
        }

        points.push(point);
        queue.extend(plane_neighbors(bounds, point));
    }

    points
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brush::apply_brush;

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
            plane_axis: crate::fill::CanvasPlaneAxis::Z,
        }
    }

    fn cell(glyph: char, rgb: (u8, u8, u8), weight_index: i64) -> PaintedCell {
        PaintedCell {
            graphic: thaum_renderer_domain::CellGraphic::Glyph(glyph),
            color: crate::paint_color::PaintColor::flat_rgb(rgb.0, rgb.1, rgb.2),
            weight_index,
        }
    }

    #[test]
    fn additive_and_subtractive_application_update_the_live_selection() {
        let mut selection = PlaneSelection::new(bounds());
        selection.apply_points([point(1, 1), point(2, 1)], SelectionMode::Additive);
        selection.apply_points([point(2, 1)], SelectionMode::Subtract);

        assert!(selection.contains(point(1, 1)));
        assert!(!selection.contains(point(2, 1)));
    }

    #[test]
    fn select_all_and_invert_cover_the_whole_plane_bounds() {
        let mut selection = PlaneSelection::new(bounds());
        selection.apply_points([point(1, 1)], SelectionMode::Replace);
        selection.invert();

        assert!(!selection.contains(point(1, 1)));
        assert!(selection.contains(point(0, 0)));
        assert!(selection.contains(point(4, 4)));

        selection.clear();
        selection.select_all();
        assert!(selection.contains(point(0, 0)));
        assert!(selection.contains(point(4, 4)));
    }

    #[test]
    fn painter_selection_applies_the_current_mode_to_plane_updates() {
        let mut selection = PainterSelection::new(bounds());
        selection.set_mode(SelectionMode::Additive);
        selection.apply_plane_points([point(1, 1)]);
        selection.apply_plane_points([point(2, 1)]);
        assert!(selection.plane().contains(point(1, 1)));
        assert!(selection.plane().contains(point(2, 1)));

        selection.set_mode(SelectionMode::Intersect);
        selection.apply_plane_points([point(2, 1), point(3, 1)]);
        assert!(!selection.plane().contains(point(1, 1)));
        assert!(selection.plane().contains(point(2, 1)));
        assert!(!selection.plane().contains(point(3, 1)));
    }

    #[test]
    fn painter_selection_keeps_world_selection_separate_from_plane_selection() {
        let mut selection = PainterSelection::new(bounds());
        selection.apply_plane_points([point(1, 1)]);
        selection.set_mode(SelectionMode::Additive);
        selection.apply_world_points([
            CellPoint { x: 9, y: 9, z: 2 },
            CellPoint { x: 10, y: 9, z: 2 },
        ]);

        assert!(selection.plane().contains(point(1, 1)));
        assert!(selection.world().contains(CellPoint { x: 9, y: 9, z: 2 }));
        assert!(selection.world().contains(CellPoint { x: 10, y: 9, z: 2 }));
    }

    #[test]
    fn plane_edit_filter_allows_everything_when_no_selection_exists() {
        let selection = PainterSelection::new(bounds());

        assert_eq!(
            selection.filter_plane_edit_points([point(0, 0), point(1, 0)]),
            vec![point(0, 0), point(1, 0)]
        );
    }

    #[test]
    fn set_plane_bounds_retargets_the_interaction_plane_without_losing_selections() {
        let mut selection = PainterSelection::new(bounds());
        selection.apply_plane_points_with_mode([point(1, 1), point(4, 4)], SelectionMode::Replace);

        selection.set_plane_bounds(CanvasBounds {
            x0: 0,
            y0: 0,
            x1: 2,
            y1: 2,
            z: 0,
            plane_axis: crate::fill::CanvasPlaneAxis::Z,
        });

        // The cell set is 3D: retargeting the interaction plane never prunes it.
        assert!(selection.plane().contains(point(1, 1)));
        assert!(selection.plane().contains(point(4, 4)));
    }

    #[test]
    fn selections_persist_across_depth_changes_and_render_from_any_plane() {
        let mut selection = PainterSelection::new(bounds());
        selection.apply_plane_points_with_mode([point(1, 1)], SelectionMode::Replace);

        // Simulate the camera moving to a different depth and plane axis: the
        // selection set must survive untouched.
        selection.set_plane_bounds(CanvasBounds {
            x0: -10,
            y0: -10,
            x1: 10,
            y1: 10,
            z: 3,
            plane_axis: crate::fill::CanvasPlaneAxis::Y,
        });

        assert!(selection.plane().contains(point(1, 1)));

        // A stroke on the new plane (axis Y, fixed y = 3) coexists with the old
        // depth's cells.
        let depth_point = CellPoint { x: 0, y: 3, z: 5 };
        selection.apply_plane_points_with_mode([depth_point], SelectionMode::Additive);
        assert!(selection.plane().contains(point(1, 1)));
        assert!(selection.plane().contains(depth_point));
    }

    #[test]
    fn replace_strokes_only_rewrite_the_active_plane_slice() {
        let mut selection = PainterSelection::new(bounds());
        selection.apply_plane_points_with_mode([point(1, 1)], SelectionMode::Additive);
        let deep = CellPoint { x: 8, y: 8, z: 7 };
        selection.restore_points([deep]);

        // A replace stroke on the active plane clears that plane's slice but must
        // never touch cells selected at other depths/planes.
        selection.apply_plane_points_with_mode([point(2, 2)], SelectionMode::Replace);

        assert!(!selection.plane().contains(point(1, 1)));
        assert!(selection.plane().contains(point(2, 2)));
        assert!(selection.plane().contains(deep));
    }

    #[test]
    fn select_all_and_invert_are_scoped_to_the_active_plane_slice() {
        let mut selection = PainterSelection::new(bounds());
        let deep = CellPoint { x: 8, y: 8, z: 7 };
        selection.restore_points([deep]);

        selection.invert_plane();
        // Invert flips the slice around the empty selection, deep cell untouched.
        assert!(selection.plane().contains(point(0, 0)));
        assert!(selection.plane().contains(deep));

        selection.select_all_plane();
        assert!(selection.plane().contains(point(4, 4)));
        assert!(selection.plane().contains(deep));

        selection.clear_plane();
        assert!(!selection.plane().contains(deep));
    }

    #[test]
    fn restore_points_seeds_cells_outside_the_current_plane() {
        let mut selection = PainterSelection::new(bounds());
        selection.restore_points([
            point(1, 1),
            CellPoint { x: -3, y: -3, z: -2 },
        ]);

        assert!(selection.plane().contains(point(1, 1)));
        assert!(selection
            .plane()
            .contains(CellPoint { x: -3, y: -3, z: -2 }));
    }

    #[test]
    fn plane_edit_filter_limits_edits_to_the_selected_cells() {
        let mut selection = PainterSelection::new(bounds());
        selection.apply_plane_points([point(1, 1), point(2, 1)]);

        assert_eq!(
            selection.filter_plane_edit_points([point(0, 0), point(1, 1), point(2, 1)]),
            vec![point(1, 1), point(2, 1)]
        );
    }

    #[test]
    fn border_detection_marks_edge_cells() {
        let mut selection = PlaneSelection::new(bounds());
        selection.apply_points(
            [
                point(1, 1),
                point(2, 1),
                point(3, 1),
                point(1, 2),
                point(2, 2),
                point(3, 2),
                point(1, 3),
                point(2, 3),
                point(3, 3),
            ],
            SelectionMode::Replace,
        );

        assert!(selection.is_border(point(1, 1)));
        assert!(!selection.is_border(point(2, 2)));
    }

    #[test]
    fn flood_select_respects_the_enabled_channels() {
        let mut canvas = Canvas::new();
        apply_brush(&mut canvas, point(0, 0), cell('A', (1, 1, 1), 0));
        apply_brush(&mut canvas, point(1, 0), cell('B', (1, 1, 1), 0));
        apply_brush(&mut canvas, point(2, 0), cell('B', (9, 9, 9), 0));

        let glyph_only = flood_select_points(
            &canvas,
            point(1, 0),
            bounds(),
            ChannelMask {
                graphic: true,
                color: false,
                weight: false,
            },
        );
        let glyph_and_color = flood_select_points(
            &canvas,
            point(1, 0),
            bounds(),
            ChannelMask {
                graphic: true,
                color: true,
                weight: false,
            },
        );

        assert_eq!(glyph_only, vec![point(1, 0), point(2, 0)]);
        assert_eq!(glyph_and_color, vec![point(1, 0)]);
    }
}
