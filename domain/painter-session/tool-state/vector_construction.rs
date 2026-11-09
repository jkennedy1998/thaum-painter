//! Live vector construction session (project 6kbosu52). A vector drag
//! begins a session that stays open across many presses (J 2026-11-09: a
//! press begins a vector construction session that stays open across many
//! presses; Enter, Escape, or switching tools commits the accumulated
//! shape). The session holds one or more subpaths plus exactly one
//! selection pointer — only the selected point's handles are ever visible
//! or pressable, and selection defines which subpath is active. Committing
//! runs the stroke/fill authoring sources per cell and writes every
//! subpath's cells as one action.

use std::collections::BTreeSet;

use thaum_renderer_domain::{CellGraphic, CellPoint};

use crate::brush::{self, Canvas, PaintedCell};
use crate::tool_state::{PaintHand, ToolState};
use crate::vector::{VectorPoint, VectorSubpath};

/// Which end of the selected point's handles is grabbed. A handle is only a
/// distinct press target when its point is selected and its magnitude is
/// nonzero (J 2026-11-09's precedence chart).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandleEnd {
    Forward,
    Backward,
}

/// The one selection pointer across the whole session: which subpath, which
/// point on it, and optionally which of that point's handles is grabbed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VectorSelection {
    pub subpath: usize,
    pub point: usize,
    pub handle: Option<HandleEnd>,
}

/// One open vector construction session across one or more subpaths.
#[derive(Debug, Clone)]
pub struct VectorConstruction {
    pub hand: PaintHand,
    subpaths: Vec<VectorSubpath>,
    selection: Option<VectorSelection>,
}

impl VectorConstruction {
    pub fn new(hand: PaintHand) -> Self {
        Self {
            hand,
            subpaths: Vec::new(),
            selection: None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.subpaths.is_empty()
    }

    pub fn subpaths(&self) -> &[VectorSubpath] {
        &self.subpaths
    }

    pub fn selection(&self) -> Option<VectorSelection> {
        self.selection
    }

    /// The selection's subpath index — selection defines the active subpath
    /// (J 2026-11-09 clarification pass).
    pub fn active_subpath(&self) -> Option<usize> {
        self.selection.map(|selection| selection.subpath)
    }

    /// Selects one point; its handles (if any) become visible and grabbable
    /// and its subpath becomes active.
    pub fn select_point(&mut self, subpath: usize, point: usize) {
        if subpath < self.subpaths.len() && point < self.subpaths[subpath].points.len() {
            self.selection = Some(VectorSelection {
                subpath,
                point,
                handle: None,
            });
        }
    }

    /// Selects one handle of an existing point for a follow-up drag. The
    /// handle only exists as a target when its point carries nonzero
    /// magnitude handles.
    pub fn select_handle(&mut self, subpath: usize, point: usize, handle: HandleEnd) {
        if subpath < self.subpaths.len() && point < self.subpaths[subpath].points.len() {
            self.selection = Some(VectorSelection {
                subpath,
                point,
                handle: Some(handle),
            });
        }
    }

    /// The selected point, if any.
    pub fn selected_point(&self) -> Option<&VectorPoint> {
        let selection = self.selection?;
        self.subpaths
            .get(selection.subpath)?
            .points
            .get(selection.point)
    }

    /// The selected point's connection count in its own subpath — the
    /// empty-cell press reads this to decide continue-vs-branch
    /// (J 2026-11-09).
    pub fn selected_connection_count(&self) -> Option<usize> {
        let selection = self.selection?;
        Some(
            self.subpaths
                .get(selection.subpath)?
                .connection_count(selection.point),
        )
    }

    /// The selected point's handle-drag expansion direction: the line to
    /// the adjacent point on its subpath, or a fixed horizontal axis when
    /// the point has no neighbor (single-point subpath).
    pub fn selected_handle_expansion_direction(&self) -> Option<CellPoint> {
        let selection = self.selection?;
        let subpath = self.subpaths.get(selection.subpath)?;
        let anchor = subpath.points.get(selection.point)?.anchor;
        let neighbor = subpath
            .points
            .get(selection.point + 1)
            .or_else(|| selection.point.checked_sub(1).and_then(|i| subpath.points.get(i)))?
            .anchor;
        Some(CellPoint {
            x: neighbor.x - anchor.x,
            y: neighbor.y - anchor.y,
            z: neighbor.z - anchor.z,
        })
    }

    /// Places the first point of the very first subpath (a hard corner), or
    /// moves an empty-press through the selected point's connection count:
    /// one connection continues that subpath from the tail, two connections
    /// branch a brand-new subpath rooted at the selected point's shared
    /// cell — the new subpath starts with a hard-corner duplicate at that
    /// cell so the branch connects there, then the press itself lands as
    /// its first authored point (J 2026-11-09's chart). `handled` mirrors
    /// the new point's handles around `forward_offset` when the press
    /// dragged. Returns the (subpath, point) indices of the new point,
    /// which becomes selected.
    pub fn place_from_selection(
        &mut self,
        anchor: CellPoint,
        handled: Option<CellPoint>,
    ) -> (usize, usize) {
        let point = match handled {
            Some(forward_offset) => VectorPoint::mirrored(anchor, forward_offset),
            None => VectorPoint::hard_corner(anchor),
        };
        match (self.selection, self.selected_connection_count()) {
            (Some(selection), Some(1)) => {
                // Continue the selected open tail's subpath: appending at the
                // last point, or prepending when the selected tail is the
                // subpath's first point.
                let subpath = &mut self.subpaths[selection.subpath];
                let at_head = selection.point == 0 && subpath.points.len() > 1;
                let index = if at_head {
                    subpath.points.insert(0, point);
                    0
                } else {
                    subpath.points.push(point);
                    subpath.points.len() - 1
                };
                self.selection = Some(VectorSelection {
                    subpath: selection.subpath,
                    point: index,
                    handle: None,
                });
                (selection.subpath, index)
            }
            (Some(selection), Some(2)) => {
                // Fully interior selected point: branch a brand-new subpath
                // rooted at the shared cell — a hard-corner duplicate there
                // connects the branch, and the pressed point follows it.
                let root = self.subpaths[selection.subpath].points[selection.point].anchor;
                let subpath_index = self.subpaths.len();
                self.subpaths.push(VectorSubpath::new(
                    vec![VectorPoint::hard_corner(root), point],
                    false,
                ));
                self.selection = Some(VectorSelection {
                    subpath: subpath_index,
                    point: 1,
                    handle: None,
                });
                (subpath_index, 1)
            }
            (Some(selection), Some(0)) => {
                // A single-point subpath's only point is selected: the press
                // continues that subpath from it.
                let subpath = &mut self.subpaths[selection.subpath];
                subpath.points.push(point);
                let index = subpath.points.len() - 1;
                self.selection = Some(VectorSelection {
                    subpath: selection.subpath,
                    point: index,
                    handle: None,
                });
                (selection.subpath, index)
            }
            _ => {
                // No selection (or a stale one): start a fresh subpath.
                let subpath_index = self.subpaths.len();
                self.subpaths.push(VectorSubpath::new(vec![point], false));
                self.selection = Some(VectorSelection {
                    subpath: subpath_index,
                    point: 0,
                    handle: None,
                });
                (subpath_index, 0)
            }
        }
    }

    /// Starts a brand-new, fully independent subpath at `anchor` — the
    /// double-click-on-empty-space action (J 2026-11-09). The new point
    /// becomes selected.
    pub fn start_independent_subpath(&mut self, anchor: CellPoint) -> (usize, usize) {
        let subpath_index = self.subpaths.len();
        self.subpaths
            .push(VectorSubpath::new(vec![VectorPoint::hard_corner(anchor)], false));
        self.selection = Some(VectorSelection {
            subpath: subpath_index,
            point: 0,
            handle: None,
        });
        (subpath_index, 0)
    }

    /// Rigid-moves the selected point (its handles translate with it) by
    /// `delta`, live focus depth included in the delta.
    pub fn move_selected_point(&mut self, delta: CellPoint) {
        let Some(selection) = self.selection else {
            return;
        };
        if let Some(point) = self
            .subpaths
            .get_mut(selection.subpath)
            .and_then(|subpath| subpath.points.get_mut(selection.point))
        {
            point.translate(delta);
        }
    }

    /// Moves only the selected handle — asymmetric, the sibling handle is
    /// unaffected (J 2026-11-09).
    pub fn move_selected_handle(&mut self, delta: CellPoint) {
        let Some(selection) = self.selection else {
            return;
        };
        let Some(handle) = selection.handle else {
            return;
        };
        if let Some(point) = self
            .subpaths
            .get_mut(selection.subpath)
            .and_then(|subpath| subpath.points.get_mut(selection.point))
        {
            let offset = match handle {
                HandleEnd::Forward => point.forward_offset.as_mut(),
                HandleEnd::Backward => point.backward_offset.as_mut(),
            };
            if let Some(offset) = offset {
                offset.x += delta.x;
                offset.y += delta.y;
                offset.z += delta.z;
            }
        }
    }

    /// Closes the named subpath when the restricted close case holds: the
    /// subpath is open and the currently selected point has exactly one
    /// connection, and the press target is that subpath's own first point's
    /// cell (J 2026-11-09: modeled on Adobe's pen-tool close). Returns
    /// whether the close fired.
    pub fn try_close_subpath(&mut self, subpath: usize, target: CellPoint) -> bool {
        let Some(sub) = self.subpaths.get(subpath) else {
            return false;
        };
        let first_is_target = sub.points.first().is_some_and(|point| point.anchor == target);
        let selected_is_open_tail = self
            .selection
            .is_some_and(|selection| {
                selection.subpath == subpath
                    && sub.connection_count(selection.point) == 1
            });
        if first_is_target && selected_is_open_tail {
            self.subpaths[subpath].close();
            // The last placed point (the former tail, not the first point it
            // connected to) becomes selected (J 2026-11-09).
            let last = self.subpaths[subpath].points.len() - 1;
            self.selection = Some(VectorSelection {
                subpath,
                point: last,
                handle: None,
            });
            true
        } else {
            false
        }
    }

    /// Toggles the selected point's handles (the double-click gesture):
    /// nonzero magnitude collapses to a hard corner, zero magnitude expands
    /// to magnitude 1 along the neighbor tangent — or a fixed horizontal
    /// axis when the point has no neighbor (J 2026-11-09).
    pub fn toggle_selected_point_handles(&mut self) {
        let Some(selection) = self.selection else {
            return;
        };
        let Some(subpath) = self.subpaths.get(selection.subpath) else {
            return;
        };
        let Some(point) = subpath.points.get(selection.point) else {
            return;
        };
        if point.has_handles() {
            self.subpaths[selection.subpath].points[selection.point].collapse_handles();
            return;
        }
        let direction = subpath
            .points
            .get(selection.point + 1)
            .or_else(|| selection.point.checked_sub(1).and_then(|i| subpath.points.get(i)))
            .map(|neighbor| CellPoint {
                x: neighbor.anchor.x - point.anchor.x,
                y: neighbor.anchor.y - point.anchor.y,
                z: neighbor.anchor.z - point.anchor.z,
            })
            .unwrap_or(CellPoint { x: 1, y: 0, z: 0 });
        self.subpaths[selection.subpath].points[selection.point]
            .expand_handles(direction);
    }

    /// Deletes the selected point and clears the selection — the user then
    /// selects any remaining point (J 2026-11-09). Deleting a closed
    /// subpath's last point reopens that subpath (the tail redo case); a
    /// subpath emptied by the delete disappears. Returns whether the whole
    /// session is now empty — the caller's signal to cancel and exit
    /// authoring mode without committing.
    pub fn delete_selected_point(&mut self) -> bool {
        let Some(selection) = self.selection else {
            return self.subpaths.is_empty();
        };
        let was_closed = self.subpaths[selection.subpath].closed;
        let was_last = selection.point + 1 == self.subpaths[selection.subpath].points.len();
        self.subpaths[selection.subpath]
            .points
            .remove(selection.point);
        if was_closed && was_last {
            // The tail redo case: removing the closing segment reopens.
            self.subpaths[selection.subpath].reopen();
        }
        if self.subpaths[selection.subpath].points.is_empty() {
            self.subpaths.remove(selection.subpath);
        }
        self.selection = None;
        self.subpaths.is_empty()
    }

    /// Every cell the whole session's committed geometry covers: stroke
    /// bands and fill boundaries from every subpath, unioned.
    pub fn covered_cells(&self, stroke_size: i32) -> BTreeSet<CellPoint> {
        let mut cells: BTreeSet<CellPoint> = BTreeSet::new();
        for subpath in &self.subpaths {
            for cell in subpath.stroke_cells(stroke_size) {
                cells.insert(cell);
            }
            for cell in subpath.boundary_cells() {
                cells.insert(cell);
            }
        }
        cells
    }
}

/// Commits one vector construction session for its hand: every subpath's
/// stroke band paints with the acting hand's `stroke_source` content (under
/// it), every closed subpath's boundary paints with `fill_source` content —
/// and stroke wins the centerline: stroke cells paint after fill cells
/// (J 2026-11-09: "stroke wins"). A `None` source skips its channel
/// entirely; a locked edit mask authors nothing. Returns the touched cells
/// so the caller can stage the same canvas through its document seam.
pub fn commit_vector_construction(
    tool_state: &ToolState,
    canvas: &mut Canvas,
    construction: &VectorConstruction,
) -> Vec<CellPoint> {
    let mut touched: Vec<CellPoint> = Vec::new();
    let mut seen: BTreeSet<CellPoint> = BTreeSet::new();
    let mut write = |canvas: &mut Canvas,
                     cell: CellPoint,
                     source: PaintHand,
                     touched: &mut Vec<CellPoint>,
                     seen: &mut BTreeSet<CellPoint>| {
        let painted = tool_state.resolved_painted_cell(canvas.get(&cell), source);
        brush::write_cell(canvas, cell, painted);
        if seen.insert(cell) {
            touched.push(cell);
        }
    };
    let stroke_size = tool_state.hand_state(construction.hand).brush_size;
    // Fill first, stroke second: stroke wins the centerline (J 2026-11-09).
    for subpath in construction.subpaths() {
        let Some(source) = tool_state.hand_state(construction.hand).fill_source else {
            continue;
        };
        if !tool_state.hand_state(source).edit_channels.any_enabled() {
            continue;
        }
        for cell in subpath.boundary_cells() {
            write(canvas, cell, source, &mut touched, &mut seen);
        }
    }
    for subpath in construction.subpaths() {
        let Some(source) = tool_state.hand_state(construction.hand).stroke_source else {
            continue;
        };
        if !tool_state.hand_state(source).edit_channels.any_enabled() {
            continue;
        }
        for cell in subpath.stroke_cells(stroke_size) {
            write(canvas, cell, source, &mut touched, &mut seen);
        }
    }
    touched
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_state::PaintTool;

    fn cell(x: i32, y: i32) -> CellPoint {
        CellPoint { x, y, z: 0 }
    }

    fn painted_canvas() -> Canvas {
        Canvas::default()
    }

    #[test]
    fn the_first_press_starts_the_first_subpath_and_selects_its_point() {
        let mut construction = VectorConstruction::new(PaintHand::Left);
        let (subpath, point) = construction.place_from_selection(cell(3, 3), None);
        assert_eq!((subpath, point), (0, 0));
        assert_eq!(construction.selected_point().unwrap().anchor, cell(3, 3));
        assert_eq!(construction.selected_connection_count(), Some(0));
    }

    #[test]
    fn a_dragged_press_mirrors_handles_and_a_click_stays_a_hard_corner() {
        let mut construction = VectorConstruction::new(PaintHand::Left);
        construction.place_from_selection(cell(0, 0), Some(cell(2, 1)));
        let point = construction.selected_point().unwrap();
        assert_eq!(point.forward_handle(), Some(cell(2, 1)));
        assert_eq!(point.backward_handle(), Some(cell(-2, -1)));

        let mut construction = VectorConstruction::new(PaintHand::Left);
        construction.place_from_selection(cell(0, 0), None);
        assert!(!construction.selected_point().unwrap().has_handles());
    }

    #[test]
    fn an_open_tail_continues_and_an_interior_point_branches() {
        let mut construction = VectorConstruction::new(PaintHand::Left);
        construction.place_from_selection(cell(0, 0), None);
        let (subpath, point) = construction.place_from_selection(cell(3, 0), None);
        assert_eq!((subpath, point), (0, 1), "one connection continues");

        // Selecting the head tail of a two-point open subpath continues by
        // prepending (the new point connects to the selected tail).
        construction.select_point(0, 0);
        assert_eq!(construction.selected_connection_count(), Some(1));
        let (subpath, point) = construction.place_from_selection(cell(0, 3), None);
        assert_eq!((subpath, point), (0, 0), "the head tail prepends");
        assert_eq!(construction.subpaths()[0].points[0].anchor, cell(0, 3));
        assert_eq!(construction.subpaths()[0].points[1].anchor, cell(0, 0));

        // Selecting a fully interior point and pressing empty space branches.
        construction.select_point(0, 1);
        assert_eq!(construction.selected_connection_count(), Some(2));
        let (subpath, point) = construction.place_from_selection(cell(3, 3), None);
        assert_eq!((subpath, point), (1, 1), "the pressed point is selected");
        assert_eq!(construction.subpaths().len(), 2);
        assert_eq!(
            construction.subpaths()[1].points[0].anchor,
            cell(0, 0),
            "the branch roots at the shared cell"
        );
        assert_eq!(
            construction.subpaths()[1].points[1].anchor,
            cell(3, 3),
            "the press lands as the branch's first authored point"
        );
    }

    #[test]
    fn closing_requires_the_first_point_and_an_open_tail_selection() {
        let mut construction = VectorConstruction::new(PaintHand::Left);
        construction.place_from_selection(cell(0, 0), None);
        construction.place_from_selection(cell(4, 0), None);
        // Pressing the first point while the tail is selected closes.
        assert!(construction.try_close_subpath(0, cell(0, 0)));
        assert!(construction.subpaths()[0].closed);
        // The last placed point is now selected, not the first.
        assert_eq!(construction.selection().unwrap().point, 1);
        // A second close attempt does nothing (already closed).
        assert!(!construction.try_close_subpath(0, cell(0, 0)));
    }

    #[test]
    fn closing_refuses_without_the_open_tail_selection() {
        let mut construction = VectorConstruction::new(PaintHand::Left);
        construction.place_from_selection(cell(0, 0), None);
        construction.place_from_selection(cell(4, 0), None);
        construction.place_from_selection(cell(8, 0), None);
        // Select the middle interior point; its press on the first cell must
        // not close.
        construction.select_point(0, 1);
        assert!(!construction.try_close_subpath(0, cell(0, 0)));
        assert!(!construction.subpaths()[0].closed);
    }

    #[test]
    fn double_click_toggles_handles_along_the_neighbor_tangent() {
        let mut construction = VectorConstruction::new(PaintHand::Left);
        construction.place_from_selection(cell(0, 0), None);
        construction.place_from_selection(cell(4, 4), None);
        // The tail (index 1) selected: its neighbor tangent points back to
        // (0,0) — expansion goes one cell along that line.
        construction.toggle_selected_point_handles();
        let point = construction.selected_point().unwrap();
        assert_eq!(point.forward_offset, Some(cell(-1, -1)));
        assert_eq!(point.backward_offset, Some(cell(1, 1)));
        // Collapse again.
        construction.toggle_selected_point_handles();
        assert!(!construction.selected_point().unwrap().has_handles());
    }

    #[test]
    fn a_single_point_expands_on_the_fixed_axis() {
        let mut construction = VectorConstruction::new(PaintHand::Left);
        construction.place_from_selection(cell(2, 2), None);
        construction.toggle_selected_point_handles();
        let point = construction.selected_point().unwrap();
        assert_eq!(point.forward_offset, Some(cell(1, 0)));
        assert_eq!(point.backward_offset, Some(cell(-1, 0)));
    }

    #[test]
    fn point_move_is_rigid_and_handle_move_is_asymmetric() {
        let mut construction = VectorConstruction::new(PaintHand::Left);
        construction.place_from_selection(cell(0, 0), Some(cell(2, 0)));
        construction.move_selected_point(cell(1, 1));
        let point = construction.selected_point().unwrap();
        assert_eq!(point.anchor, cell(1, 1));
        assert_eq!(point.forward_handle(), Some(cell(3, 1)), "rigid");
        assert_eq!(point.backward_handle(), Some(cell(-1, 1)), "rigid");

        construction.select_handle(0, 0, HandleEnd::Forward);
        construction.move_selected_handle(cell(0, -1));
        let point = construction.selected_point().unwrap();
        assert_eq!(point.forward_handle(), Some(cell(3, 0)));
        assert_eq!(point.backward_handle(), Some(cell(-1, 1)), "sibling unaffected");
    }

    #[test]
    fn independent_subpaths_start_unconnected() {
        let mut construction = VectorConstruction::new(PaintHand::Left);
        construction.place_from_selection(cell(0, 0), None);
        let (subpath, point) = construction.start_independent_subpath(cell(10, 10));
        assert_eq!((subpath, point), (1, 0));
        assert_eq!(construction.subpaths().len(), 2);
        assert_eq!(construction.subpaths()[1].points.len(), 1);
    }

    #[test]
    fn backspace_deletes_the_selection_and_the_last_delete_cancels() {
        let mut construction = VectorConstruction::new(PaintHand::Left);
        construction.place_from_selection(cell(0, 0), None);
        construction.place_from_selection(cell(4, 0), None);
        // Delete the tail: the subpath keeps its earlier point.
        assert!(!construction.delete_selected_point());
        assert_eq!(construction.subpaths()[0].points.len(), 1);
        assert_eq!(construction.selection(), None, "selection clears");
        // Select the remaining point, delete it: the session empties.
        construction.select_point(0, 0);
        assert!(construction.delete_selected_point());
        assert!(construction.is_empty(), "the caller cancels the session");
    }

    #[test]
    fn deleting_a_closed_tail_reopens_the_subpath() {
        let mut construction = VectorConstruction::new(PaintHand::Left);
        construction.place_from_selection(cell(0, 0), None);
        construction.place_from_selection(cell(4, 0), None);
        construction.place_from_selection(cell(4, 4), None);
        assert!(construction.try_close_subpath(0, cell(0, 0)));
        // The closed tail is selected; backspacing it deletes the tail and
        // reopens for a redo (J 2026-11-09).
        assert!(!construction.delete_selected_point());
        assert!(!construction.subpaths()[0].closed);
        assert_eq!(construction.subpaths()[0].points.len(), 2);
    }

    #[test]
    fn commit_paints_fill_then_stroke_and_stroke_wins_the_centerline() {
        let mut tool_state = ToolState::default();
        tool_state.set_tool_for_hand(PaintHand::Left, PaintTool::Vector);
        tool_state.set_graphic_for_hand(PaintHand::Left, CellGraphic::Glyph('S'));
        tool_state.set_graphic_for_hand(PaintHand::Right, CellGraphic::Glyph('F'));
        tool_state.set_stroke_source_for_hand(PaintHand::Left, Some(PaintHand::Left));
        tool_state.set_fill_source_for_hand(PaintHand::Left, Some(PaintHand::Right));
        tool_state.set_brush_size_for_hand(PaintHand::Left, 3);

        let mut construction = VectorConstruction::new(PaintHand::Left);
        construction.place_from_selection(cell(0, 0), None);
        construction.place_from_selection(cell(4, 0), None);
        assert!(construction.try_close_subpath(0, cell(0, 0)));

        let mut canvas = painted_canvas();
        let touched = commit_vector_construction(&tool_state, &mut canvas, &construction);
        // The centerline is stroke's content (stroke wins over fill's
        // boundary ring), and the band runs one cell each side.
        for x in 0..=4 {
            let painted = canvas.get(&cell(x, 0)).unwrap();
            assert_eq!(painted.graphic, CellGraphic::Glyph('S'), "centerline at {x}");
            let above = canvas.get(&cell(x, 1)).unwrap();
            assert_eq!(above.graphic, CellGraphic::Glyph('S'), "band at {x}");
        }
        // Fill ran under the stroke but every fill cell lies inside the
        // stroke band, so no 'F' survives anywhere.
        assert!(touched.iter().all(|cell| {
            canvas.get(cell).unwrap().graphic == CellGraphic::Glyph('S')
        }));
    }

    #[test]
    fn a_none_source_skips_its_channel_and_empty_cells_author_fresh() {
        let mut tool_state = ToolState::default();
        tool_state.set_tool_for_hand(PaintHand::Left, PaintTool::Vector);
        tool_state.set_graphic_for_hand(PaintHand::Left, CellGraphic::Glyph('S'));
        tool_state.set_stroke_source_for_hand(PaintHand::Left, Some(PaintHand::Left));
        tool_state.set_fill_source_for_hand(PaintHand::Left, None);

        let mut construction = VectorConstruction::new(PaintHand::Left);
        construction.place_from_selection(cell(0, 0), None);
        construction.place_from_selection(cell(4, 0), None);
        assert!(construction.try_close_subpath(0, cell(0, 0)));

        let mut canvas = painted_canvas();
        commit_vector_construction(&tool_state, &mut canvas, &construction);
        // Stroke painted the closed outline; nothing else was authored.
        assert!(canvas.get(&cell(2, 0)).is_some());
        assert!(canvas.get(&cell(2, 1)).is_none(), "fill skipped, interior empty");
    }
}
