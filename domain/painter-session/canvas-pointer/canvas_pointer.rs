//! Per-hand pointer stroke lifecycle on the paint canvas: press dispatch,
//! drag continuation, and release commit — written once, shared by both
//! hands.
//!
//! This encapsulation owns the in-progress stroke state (the lasso bound,
//! the selection stroke, each hand's staged image-stroke start, and each
//! hand's last drag position) and the per-tool dispatch across the three
//! pointer phases. The entrypoint keeps only what is genuinely screen-space:
//! hit testing (command bar, module captures, drawing-space gizmos) and
//! typing-session keyboard ownership, which it installs from
//! [`TypingSessionBegin`] when the text tool starts a session.
//!
//! Dispatch rules, one copy for both hands:
//! - Selection target: the lasso records its bound on press and selects the
//!   enclosed region on release; every other tool extends a selection stroke
//!   per position through `selection_points_for_hand`.
//! - Image target: the text tool begins a typing session on press; the lasso
//!   records its bound and fills on release through `apply_lasso_for_hand`;
//!   every other tool stages dabs through `stage_image_edit_chunk` and
//!   commits once on release (one undo per stroke).
//!
//! Invariants:
//! - A press begins at most one stroke kind for the pressing hand.
//! - A stroke only responds to drags from its owning hand.
//! - A release commits each hand's work exactly once; a lasso release is the
//!   only image-target edit that does not flow through the staged-patch
//!   commit (it paints directly, then the staged commit is a no-op).
//! - Cancelling (command bar or module capture) clears the in-progress
//!   strokes and the cancelling hand's drag position, never staged starts.

use std::cell::RefCell;
use std::rc::Rc;

use anyhow::Error;
use thaum_renderer_domain::{CameraViewOrientation, CellColor, CellGroup, CellPoint};

use crate::{
    brush::{Canvas, PaintedCell},
    clipboard::WorldCopyData,
    fill::CanvasBounds,
    lasso_stroke::{
        build_lasso_path_cell_group, build_lasso_preview_cell_groups, LassoStroke,
    },
    painter_tools::shared::{drag_behavior, selection_behavior, DragBehavior},
    selection_state::{PainterSelection, SelectionMode},
    selection_stroke::{
        build_plane_selection_cell_groups, interpolate_cell_path, SelectionStroke,
    },
    session_document::{
        commit_selection_channel, commit_staged_paint_stroke, stage_image_edit_chunk,
        stage_painted_cells_chunk,
    },
    storage::{SharedDocumentPaths, SharedDocumentRuntime},
    text_entry::TextEntryState,
    tool_state::{PaintHand, PaintTarget, ToolState},
};

/// The canvas block plus snapshot a stroke started against; release diffs
/// from it to produce one committed patch set.
pub type StrokeStart = (Canvas, String);

/// Live stamp hover state: the hand equipping the stamp, the canvas cell it
/// would stamp at, and that hand user's clipboard payload. The entrypoint
/// refreshes it every frame from the cursor; the overlay consumes it for the
/// two-phase paste preview while the press commits through the same payload.
#[derive(Debug, Clone)]
pub struct StampHover {
    pub hand: PaintHand,
    pub anchor: CellPoint,
    pub data: WorldCopyData,
}

/// Everything the pointer lifecycle needs from the live session. The
/// entrypoint assembles it per event; the seams here never touch screen
/// space or modules.
pub struct CanvasPointerContext<'a> {
    pub tool_state: &'a RefCell<ToolState>,
    pub selection: &'a Rc<RefCell<PainterSelection>>,
    pub canvas: &'a mut Canvas,
    pub document: &'a mut SharedDocumentRuntime,
    pub document_paths: &'a SharedDocumentPaths,
    pub action_counter: &'a mut u64,
    pub session_user_id: &'a str,
    pub active_layer_id: &'a mut String,
}

/// What a text-tool press hands back to the entrypoint: the typing session
/// to install and the stroke start its commits will diff against.
pub struct TypingSessionBegin {
    pub entry: TextEntryState,
    pub stroke_start: StrokeStart,
}

/// The in-progress pointer strokes across both hands.
pub struct CanvasPointerStrokes {
    lasso: Option<LassoStroke>,
    selection: Option<SelectionStroke>,
    left_image_start: Option<StrokeStart>,
    right_image_start: Option<StrokeStart>,
    left_drag_position: Option<CellPoint>,
    right_drag_position: Option<CellPoint>,
    stamp_hover: Option<StampHover>,
}

impl CanvasPointerStrokes {
    pub fn new() -> Self {
        Self {
            lasso: None,
            selection: None,
            left_image_start: None,
            right_image_start: None,
            left_drag_position: None,
            right_drag_position: None,
            stamp_hover: None,
        }
    }

    /// Replaces the live stamp hover (the entrypoint refreshes it every
    /// frame; `None` clears the paste preview). Never affects commits.
    pub fn set_stamp_hover(&mut self, hover: Option<StampHover>) {
        self.stamp_hover = hover;
    }

    /// The in-progress lasso bound, for overlay previews.
    pub fn lasso(&self) -> Option<&LassoStroke> {
        self.lasso.as_ref()
    }

    /// The in-progress selection stroke, for overlay previews.
    pub fn selection(&self) -> Option<&SelectionStroke> {
        self.selection.as_ref()
    }

    /// Whether any hand has an in-progress selection stroke.
    pub fn has_selection_stroke(&self) -> bool {
        self.selection.is_some()
    }

    /// Cancels in-progress work when a press or drag lands on chrome: the
    /// strokes clear regardless of owning hand, the cancelling hand's drag
    /// position clears, staged starts survive (they commit as no-ops).
    pub fn cancel(&mut self, hand: PaintHand) {
        self.lasso = None;
        self.selection = None;
        *self.drag_position_mut(hand) = None;
    }

    /// A button coming up without a release commit just drops the drag trail.
    pub fn clear_drag_position(&mut self, hand: PaintHand) {
        *self.drag_position_mut(hand) = None;
    }

    /// Drops both hands' drag trails (used when neither button is down).
    pub fn clear_drag_positions(&mut self) {
        self.clear_drag_position(PaintHand::Left);
        self.clear_drag_position(PaintHand::Right);
    }

    fn drag_position(&self, hand: PaintHand) -> Option<CellPoint> {
        match hand {
            PaintHand::Left => self.left_drag_position,
            PaintHand::Right => self.right_drag_position,
        }
    }

    fn drag_position_mut(&mut self, hand: PaintHand) -> &mut Option<CellPoint> {
        match hand {
            PaintHand::Left => &mut self.left_drag_position,
            PaintHand::Right => &mut self.right_drag_position,
        }
    }

    fn set_drag_position(&mut self, hand: PaintHand, position: CellPoint) {
        *self.drag_position_mut(hand) = Some(position);
    }

    fn image_start(&self, hand: PaintHand) -> Option<&StrokeStart> {
        match hand {
            PaintHand::Left => self.left_image_start.as_ref(),
            PaintHand::Right => self.right_image_start.as_ref(),
        }
    }

    fn set_image_start(&mut self, hand: PaintHand, start: StrokeStart) {
        match hand {
            PaintHand::Left => self.left_image_start = Some(start),
            PaintHand::Right => self.right_image_start = Some(start),
        }
    }

    /// Dispatches a canvas press for `hand`. Returns a typing session begin
    /// when the text tool started one; every other press begins (or declines
    /// to begin) its stroke internally. A press with no raster block under
    /// the playhead breath begins nothing.
    pub fn begin_press(
        &mut self,
        ctx: &mut CanvasPointerContext<'_>,
        hand: PaintHand,
        position: CellPoint,
        bounds: CanvasBounds,
        orientation: CameraViewOrientation,
        current_breath: u32,
    ) -> Option<TypingSessionBegin> {
        // The press seeds the drag trail so the first drag interpolates a
        // single-cell step from the press cell.
        self.set_drag_position(hand, position);
        // Click-only tools act once on press and never paint or select
        // through strokes; no drag behavior follows. The tool id is hoisted
        // out of the match scrutinee so the short-lived tool_state borrow is
        // gone before the arms borrow_mut (a scrutinee temporary would live
        // through every arm and panic the pick).
        let pressed_tool_id = ctx.tool_state.borrow().tool_for_hand(hand).id();
        if drag_behavior(pressed_tool_id) == DragBehavior::ClickOnly {
            match pressed_tool_id {
                // The picker samples the cell under the cursor into hand state.
                "picker" => {
                    ctx.tool_state
                        .borrow_mut()
                        .pick_at_for_hand(ctx.canvas, position, hand);
                }
                // The stamp places the hover preview's payload at the press
                // cell: changes stage once so release commits one undo step.
                "stamp" => {
                    let data = self
                        .stamp_hover
                        .as_ref()
                        .filter(|hover| hover.hand == hand)
                        .map(|hover| hover.data.clone());
                    if let Some(data) = data {
                        let block_id = ctx
                            .document
                            .active_raster_block_id(ctx.active_layer_id, current_breath)?;
                        let changes: Vec<(CellPoint, Option<PaintedCell>)> = {
                            let tool_state = ctx.tool_state.borrow();
                            let selection = ctx.selection.borrow();
                            tool_state
                                .stamp_changes_for_hand(ctx.canvas, &selection, &data, position, hand)
                        }
                        .into_iter()
                        .map(|(point, cell)| (point, Some(cell)))
                        .collect();
                        self.set_image_start(hand, (ctx.canvas.clone(), block_id.clone()));
                        stage_painted_cells_chunk(
                            ctx.document,
                            ctx.canvas,
                            changes,
                            ctx.active_layer_id,
                            &block_id,
                        );
                    }
                }
                _ => {}
            }
            return None;
        }
        let target = ctx.tool_state.borrow().hand_state(hand).target;
        match target {
            PaintTarget::Selection => {
                if drag_behavior(ctx.tool_state.borrow().tool_for_hand(hand).id())
                    == DragBehavior::ReleaseBound
                {
                    // The bound records only; the enclosed region selects on release.
                    self.lasso = Some(LassoStroke::new(hand, position));
                    None
                } else {
                    let mode = selection_behavior(
                        ctx.tool_state.borrow().tool_for_hand(hand).id(),
                    )
                    .resolve(ctx.selection.borrow().mode(), SelectionMode::Subtract);
                    let mut stroke = SelectionStroke::new(hand, mode);
                    let points = ctx.tool_state.borrow().selection_points_for_hand(
                        ctx.canvas,
                        position,
                        hand,
                        bounds,
                        orientation,
                    );
                    stroke.extend(points);
                    self.selection = Some(stroke);
                    None
                }
            }
            PaintTarget::Image => {
                if drag_behavior(ctx.tool_state.borrow().tool_for_hand(hand).id())
                    == DragBehavior::TypingSession
                {
                    // Typing-session tools: a click begins a typing session
                    // anchored at the click cell with the hand's brush
                    // captured; typing then owns the keyboard until Escape/exit.
                    let block_id = ctx
                        .document
                        .active_raster_block_id(&ctx.active_layer_id, current_breath)?;
                    let brush_cell = ctx.tool_state.borrow().text_brush_cell_for_hand(hand);
                    let (options, space_replace) = {
                        let tool_state = ctx.tool_state.borrow();
                        (tool_state.text_options, tool_state.text_space_replace)
                    };
                    return Some(TypingSessionBegin {
                        entry: TextEntryState::begin(
                            position,
                            orientation,
                            options,
                            space_replace,
                            brush_cell,
                        ),
                        stroke_start: (ctx.canvas.clone(), block_id),
                    });
                }
                let block_id = ctx
                    .document
                    .active_raster_block_id(&ctx.active_layer_id, current_breath)?;
                if drag_behavior(ctx.tool_state.borrow().tool_for_hand(hand).id())
                    == DragBehavior::ReleaseBound
                {
                    // The bound records only; the fill lands on release as one
                    // committed stroke.
                    self.set_image_start(hand, (ctx.canvas.clone(), block_id));
                    self.lasso = Some(LassoStroke::new(hand, position));
                    return None;
                }
                self.set_image_start(hand, (ctx.canvas.clone(), block_id.clone()));
                let mut selection_state = ctx.selection.borrow_mut();
                stage_image_edit_chunk(
                    ctx.document,
                    ctx.canvas,
                    &mut ctx.tool_state.borrow_mut(),
                    &mut selection_state,
                    [position],
                    hand,
                    bounds,
                    orientation,
                    &ctx.active_layer_id,
                    &block_id,
                );
                None
            }
        }
    }

    /// Continues the in-progress stroke for `hand` with a canvas drag.
    /// Strokes ignore drags from the other hand; with no stroke in progress,
    /// image-target drags stage chunks and selection-target drags apply per
    /// position. Click-only tools (picker, stamp) never follow drags.
    pub fn continue_drag(
        &mut self,
        ctx: &mut CanvasPointerContext<'_>,
        hand: PaintHand,
        position: CellPoint,
        bounds: CanvasBounds,
        orientation: CameraViewOrientation,
    ) {
        if drag_behavior(ctx.tool_state.borrow().tool_for_hand(hand).id()) == DragBehavior::ClickOnly
        {
            return;
        }
        let stroke_positions = self
            .drag_position(hand)
            .map(|last| interpolate_cell_path(last, position))
            .unwrap_or_else(|| vec![position]);
        if let Some(stroke) = self.lasso.as_mut() {
            if stroke.hand == hand {
                stroke.extend(&stroke_positions);
            }
        } else if let Some(stroke) = self.selection.as_mut() {
            if stroke.hand == hand {
                let tool_state = ctx.tool_state.borrow();
                for anchor in &stroke_positions {
                    stroke.extend(tool_state.selection_points_for_hand(
                        ctx.canvas,
                        *anchor,
                        hand,
                        bounds,
                        orientation,
                    ));
                }
            }
        } else {
            let target = ctx.tool_state.borrow().hand_state(hand).target;
            if target == PaintTarget::Image {
                if let Some((_, block_id)) = self.image_start(hand) {
                    let block_id = block_id.clone();
                    let mut selection_state = ctx.selection.borrow_mut();
                    stage_image_edit_chunk(
                        ctx.document,
                        ctx.canvas,
                        &mut ctx.tool_state.borrow_mut(),
                        &mut selection_state,
                        stroke_positions,
                        hand,
                        bounds,
                        orientation,
                        &ctx.active_layer_id,
                        &block_id,
                    );
                }
            } else {
                let mut selection_state = ctx.selection.borrow_mut();
                for anchor in stroke_positions {
                    ctx.tool_state.borrow_mut().apply_at_for_hand(
                        ctx.canvas,
                        &mut selection_state,
                        anchor,
                        hand,
                        bounds,
                        orientation,
                    );
                }
            }
        }
        self.set_drag_position(hand, position);
    }

    /// Releases a finished selection stroke: the plane points apply to the
    /// selection and mirror into the document channel as an exact
    /// replacement. No-op without a stroke in progress.
    pub fn finish_selection_stroke(
        &mut self,
        ctx: &mut CanvasPointerContext<'_>,
        current_breath: u32,
    ) {
        let Some(stroke) = self.selection.take() else {
            return;
        };
        ctx.selection
            .borrow_mut()
            .apply_plane_points_with_mode(stroke.points, stroke.mode);
        // Mirror the full 3D set into the document channel as an exact
        // replacement — the channel is the shared truth, the plane cache
        // is the interaction surface.
        let points: Vec<CellPoint> = ctx.selection.borrow().plane().iter().collect();
        commit_selection_channel(
            ctx.document,
            ctx.document_paths,
            points,
            SelectionMode::Replace,
            ctx.active_layer_id,
            current_breath,
            ctx.canvas,
            ctx.selection,
            ctx.action_counter,
        );
    }

    /// Releases everything else: a closed lasso bound fills or selects its
    /// enclosed region, and each hand's staged stroke commits once (one undo
    /// per stroke). Returns the commit errors so the entrypoint can report
    /// them without the seam owning logging policy.
    pub fn finish_pointer_stroke(
        &mut self,
        ctx: &mut CanvasPointerContext<'_>,
        orientation: CameraViewOrientation,
        current_breath: u32,
    ) -> Vec<Error> {
        let mut errors = Vec::new();
        if let Some(stroke) = self.lasso.take() {
            let target = ctx.tool_state.borrow().hand_state(stroke.hand).target;
            if target == PaintTarget::Image {
                ctx.tool_state.borrow_mut().apply_lasso_for_hand(
                    ctx.canvas,
                    &ctx.selection.borrow(),
                    &stroke.path,
                    stroke.hand,
                    orientation,
                );
            } else {
                {
                    let mut selection_state = ctx.selection.borrow_mut();
                    let mode = selection_behavior(
                        ctx.tool_state.borrow().tool_for_hand(stroke.hand).id(),
                    )
                    .resolve(selection_state.mode(), SelectionMode::Subtract);
                    let points = ctx.tool_state.borrow().lasso_selection_points(
                        &stroke.path,
                        stroke.hand,
                        orientation,
                    );
                    selection_state.apply_plane_points_with_mode(points, mode);
                }
                let points: Vec<CellPoint> = ctx.selection.borrow().plane().iter().collect();
                commit_selection_channel(
                    ctx.document,
                    ctx.document_paths,
                    points,
                    SelectionMode::Replace,
                    ctx.active_layer_id,
                    current_breath,
                    ctx.canvas,
                    ctx.selection,
                    ctx.action_counter,
                );
            }
        }
        for start in [self.left_image_start.take(), self.right_image_start.take()] {
            if let Err(error) = commit_staged_paint_stroke(
                ctx.document,
                ctx.document_paths,
                ctx.action_counter,
                ctx.session_user_id,
                &ctx.active_layer_id,
                ctx.canvas,
                start,
            ) {
                errors.push(error);
            }
        }
        errors
    }

    /// Overlay groups for in-progress strokes: the plane-selection preview
    /// (with any active selection stroke) and, when a lasso bound is open,
    /// the bound path plus its live interior preview. The image/selection
    /// target dispatch mirrors the release commit's — a preview must show
    /// exactly what release will paint. Never commits anything.
    pub fn overlay_cell_groups(
        &self,
        ctx: &mut CanvasPointerContext<'_>,
        orientation: CameraViewOrientation,
        vivid: CellColor,
    ) -> Vec<CellGroup> {
        let mut groups = build_plane_selection_cell_groups(
            &ctx.selection.borrow(),
            self.selection.as_ref(),
            ctx.canvas,
            vivid,
        );
        if let Some(stroke) = &self.lasso {
            groups.push(build_lasso_path_cell_group(stroke));
            let target = ctx.tool_state.borrow().hand_state(stroke.hand).target;
            let previews = if target == PaintTarget::Image {
                ctx.tool_state.borrow().lasso_preview_cells(
                    ctx.canvas,
                    &ctx.selection.borrow(),
                    &stroke.path,
                    stroke.hand,
                    orientation,
                )
            } else {
                // Selection-target lasso: the drawing will not change, so both
                // flash halves show the cell as currently drawn (vivid recolor
                // vs true) — the selection display behavior.
                ctx.tool_state.borrow().lasso_select_preview_cells(
                    ctx.canvas,
                    &stroke.path,
                    stroke.hand,
                    orientation,
                )
            };
            groups.extend(build_lasso_preview_cell_groups(&previews, vivid));
        }
        if let Some(hover) = &self.stamp_hover {
            // Two-phase paste preview: the cells as currently drawn flash
            // against the cells the press will place, through the same
            // resolution the press stages.
            let previews = {
                let tool_state = ctx.tool_state.borrow();
                let selection = ctx.selection.borrow();
                tool_state.stamp_preview_cells(
                    ctx.canvas,
                    &selection,
                    &hover.data,
                    hover.anchor,
                    hover.hand,
                )
            };
            groups.extend(build_lasso_preview_cell_groups(&previews, vivid));
        }
        groups
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use thaum_renderer_domain::{
        camera_view_orientation_for_camera, CameraRoll, CameraSwing, CellColor, CellGraphic,
    };
    use crate::{
        brush::{apply_brush, PaintedCell},
        document_locations::new_unsaved_document,
        layers_runtime::resolved_active_layer_id,
        paint_color::PaintColor,
        tool_state::PaintTool,
        session_document::sync_canvas_from_active_layer,
    };

    fn canvas_bounds() -> CanvasBounds {
        CanvasBounds {
            x0: -16,
            y0: -16,
            x1: 16,
            y1: 16,
            z: 0,
            plane_axis: Default::default(),
        }
    }

    fn flat_view() -> CameraViewOrientation {
        camera_view_orientation_for_camera(CameraSwing::PosZ, CameraRoll::Deg0)
    }

    fn point(x: i32, y: i32) -> CellPoint {
        CellPoint { x, y, z: 0 }
    }

    /// A minimal live session: one layer's canvas, tool state, selection, and
    /// a document runtime rooted in a temp dir, mirroring what the entrypoint
    /// wires into the seam.
    struct Session {
        tool_state: RefCell<ToolState>,
        selection: Rc<RefCell<PainterSelection>>,
        canvas: Canvas,
        document: SharedDocumentRuntime,
        document_paths: SharedDocumentPaths,
        action_counter: u64,
        user: String,
        layer_id: String,
    }

    impl Session {
        fn new() -> Self {
            let document = new_unsaved_document();
            let layer_id = resolved_active_layer_id(&document, None);
            let document_paths = SharedDocumentPaths::new(std::env::temp_dir().join(format!(
                "canvas-pointer-test-{}",
                std::process::id()
            )));
            let mut canvas = Canvas::new();
            sync_canvas_from_active_layer(&document, &layer_id, 0, &mut canvas);
            Self {
                tool_state: RefCell::new(ToolState::default()),
                selection: Rc::new(RefCell::new(PainterSelection::new(canvas_bounds()))),
                canvas,
                document,
                document_paths,
                action_counter: 0,
                user: "test-user".to_string(),
                layer_id,
            }
        }

        fn ctx(&mut self) -> CanvasPointerContext<'_> {
            CanvasPointerContext {
                tool_state: &self.tool_state,
                selection: &self.selection,
                canvas: &mut self.canvas,
                document: &mut self.document,
                document_paths: &self.document_paths,
                action_counter: &mut self.action_counter,
                session_user_id: &self.user,
                active_layer_id: &mut self.layer_id,
            }
        }
    }

    #[test]
    fn lasso_press_drag_release_fills_the_enclosed_region_as_one_commit() {
        let mut session = Session::new();
        {
            let tool_state = session.tool_state.get_mut();
            tool_state.set_tool_for_hand(PaintHand::Left, PaintTool::Lasso);
            tool_state.set_graphic_for_hand(PaintHand::Left, CellGraphic::Glyph('.'));
        }
        // Something drawn inside the bound so the fill visibly changes it.
        apply_brush(
            &mut session.canvas,
            point(1, 1),
            PaintedCell {
                graphic: CellGraphic::Glyph('#'),
                color: PaintColor::FlatRgb(255, 255, 255),
                weight_index: 1,
            },
        );

        let mut strokes = CanvasPointerStrokes::new();
        let bounds = canvas_bounds();
        strokes.begin_press(&mut session.ctx(), PaintHand::Left, point(0, 0), bounds, flat_view(), 0);
        strokes.continue_drag(&mut session.ctx(), PaintHand::Left, point(2, 0), bounds, flat_view());
        strokes.continue_drag(&mut session.ctx(), PaintHand::Left, point(2, 2), bounds, flat_view());
        strokes.continue_drag(&mut session.ctx(), PaintHand::Left, point(0, 2), bounds, flat_view());
        assert!(strokes.lasso().is_some());

        let errors = strokes.finish_pointer_stroke(&mut session.ctx(), flat_view(), 0);
        assert!(errors.is_empty());
        assert!(strokes.lasso().is_none());
        // The enclosed cell took the hand's glyph, in exactly one commit.
        let painted = session.canvas.get(&point(1, 1)).unwrap();
        assert_eq!(painted.graphic, CellGraphic::Glyph('.'));
        assert_eq!(session.action_counter, 1);
    }

    #[test]
    fn drags_from_the_other_hand_do_not_extend_the_stroke() {
        let mut session = Session::new();
        session
            .tool_state
            .get_mut()
            .set_tool_for_hand(PaintHand::Left, PaintTool::Lasso);

        let mut strokes = CanvasPointerStrokes::new();
        let bounds = canvas_bounds();
        strokes.begin_press(&mut session.ctx(), PaintHand::Left, point(0, 0), bounds, flat_view(), 0);
        let path_before = strokes.lasso().unwrap().path.clone();
        strokes.continue_drag(&mut session.ctx(), PaintHand::Right, point(5, 5), bounds, flat_view());
        assert_eq!(strokes.lasso().unwrap().path, path_before);

        strokes.continue_drag(&mut session.ctx(), PaintHand::Left, point(2, 0), bounds, flat_view());
        assert!(strokes.lasso().unwrap().path.len() > path_before.len());
    }

    #[test]
    fn selection_target_press_begins_a_stroke_and_release_commits_the_channel() {
        let mut session = Session::new();
        {
            let tool_state = session.tool_state.get_mut();
            tool_state.set_tool_for_hand(PaintHand::Left, PaintTool::Brush);
            tool_state.set_target_for_hand(PaintHand::Left, PaintTarget::Selection);
        }

        let mut strokes = CanvasPointerStrokes::new();
        let bounds = canvas_bounds();
        strokes.begin_press(&mut session.ctx(), PaintHand::Left, point(0, 0), bounds, flat_view(), 0);
        assert!(strokes.selection().is_some());
        strokes.continue_drag(&mut session.ctx(), PaintHand::Left, point(1, 0), bounds, flat_view());
        strokes.finish_selection_stroke(&mut session.ctx(), 0);

        assert!(strokes.selection().is_none());
        let plane_points: Vec<CellPoint> = session.selection.borrow().plane().iter().collect();
        assert!(!plane_points.is_empty());
        // Selection commits mirror into the document channel via snapshot, not
        // as action records — the counter must stay untouched.
        assert_eq!(session.action_counter, 0);
    }

    #[test]
    fn text_press_hands_back_a_typing_session() {
        let mut session = Session::new();
        session
            .tool_state
            .get_mut()
            .set_tool_for_hand(PaintHand::Left, PaintTool::Text);

        let mut strokes = CanvasPointerStrokes::new();
        let typing = strokes.begin_press(
            &mut session.ctx(),
            PaintHand::Left,
            point(3, 4),
            canvas_bounds(),
            flat_view(),
            0,
        );
        let typing = typing.expect("text press must begin a typing session");
        assert_eq!(typing.entry.cursor_point(), point(3, 4));
        assert!(typing.entry.pending_changes().is_empty());
        // The stroke start snapshots the canvas and its raster block so the
        // typed segment commits as one record on Escape/exit.
        assert!(!typing.stroke_start.1.is_empty());
    }

    #[test]
    fn overlay_cell_groups_follow_in_progress_strokes() {
        let vivid = CellColor::Flat([1.0, 0.0, 0.0, 1.0]);
        let mut session = Session::new();
        {
            let tool_state = session.tool_state.get_mut();
            tool_state.set_tool_for_hand(PaintHand::Left, PaintTool::Lasso);
            tool_state.set_graphic_for_hand(PaintHand::Left, CellGraphic::Glyph('.'));
        }
        let mut strokes = CanvasPointerStrokes::new();
        let bounds = canvas_bounds();
        strokes.begin_press(&mut session.ctx(), PaintHand::Left, point(0, 0), bounds, flat_view(), 0);
        strokes.continue_drag(&mut session.ctx(), PaintHand::Left, point(2, 2), bounds, flat_view());

        // Open lasso bound: the bound path plus its flash-preview groups.
        let groups = strokes.overlay_cell_groups(&mut session.ctx(), flat_view(), vivid);
        assert!(!groups.is_empty());

        let errors = strokes.finish_pointer_stroke(&mut session.ctx(), flat_view(), 0);
        assert!(errors.is_empty());
        // Nothing in progress and nothing selected: every returned overlay
        // group is an empty shell with no cells.
        let groups = strokes.overlay_cell_groups(&mut session.ctx(), flat_view(), vivid);
        assert!(groups
            .iter()
            .all(|group| group.iter_cells().next().is_none()));
    }

    #[test]
    fn cancel_clears_in_progress_strokes_and_release_stays_a_no_op() {
        let mut session = Session::new();
        session
            .tool_state
            .get_mut()
            .set_tool_for_hand(PaintHand::Left, PaintTool::Lasso);

        let mut strokes = CanvasPointerStrokes::new();
        strokes.begin_press(
            &mut session.ctx(),
            PaintHand::Left,
            point(0, 0),
            canvas_bounds(),
            flat_view(),
            0,
        );
        strokes.cancel(PaintHand::Left);
        assert!(strokes.lasso().is_none());
        assert!(strokes.selection().is_none());

        let errors = strokes.finish_pointer_stroke(&mut session.ctx(), flat_view(), 0);
        assert!(errors.is_empty());
        assert_eq!(session.action_counter, 0);
    }

    use crate::clipboard::WorldCopyData;
    use std::collections::BTreeMap;

    fn hover_for(hand: PaintHand, anchor: CellPoint) -> StampHover {
        let mut cells = BTreeMap::new();
        cells.insert(
            CellPoint { x: 0, y: 0, z: 0 },
            PaintedCell {
                graphic: CellGraphic::Glyph('a'),
                color: PaintColor::FlatRgb(255, 255, 255),
                weight_index: 1,
            },
        );
        cells.insert(
            CellPoint { x: 1, y: 0, z: 0 },
            PaintedCell {
                graphic: CellGraphic::Glyph('b'),
                color: PaintColor::FlatRgb(255, 255, 255),
                weight_index: 1,
            },
        );
        StampHover {
            hand,
            anchor,
            data: WorldCopyData {
                anchor: CellPoint { x: 0, y: 0, z: 0 },
                center: CellPoint { x: 0, y: 0, z: 0 },
                cells,
            },
        }
    }

    #[test]
    fn stamp_press_places_the_copied_cells_as_one_commit_and_ignores_drags() {
        let mut session = Session::new();
        {
            let tool_state = session.tool_state.get_mut();
            tool_state.set_tool_for_hand(PaintHand::Left, PaintTool::Stamp);
            tool_state.set_graphic_for_hand(PaintHand::Left, CellGraphic::Glyph('.'));
        }

        let mut strokes = CanvasPointerStrokes::new();
        strokes.set_stamp_hover(Some(hover_for(PaintHand::Left, point(5, 5))));
        let bounds = canvas_bounds();
        strokes.begin_press(&mut session.ctx(), PaintHand::Left, point(5, 5), bounds, flat_view(), 0);
        // The stamp lands with the copied cells' own appearance, staged
        // not committed yet.
        let painted = session.canvas.get(&point(5, 5)).unwrap();
        assert_eq!(painted.graphic, CellGraphic::Glyph('a'));
        assert!(session.canvas.get(&point(6, 5)).is_some());
        assert_eq!(session.action_counter, 0);

        // Drags never stamp: a click-only tool places exactly once.
        strokes.continue_drag(&mut session.ctx(), PaintHand::Left, point(8, 8), bounds, flat_view());
        assert!(session.canvas.get(&point(8, 8)).is_none());
        assert!(session.canvas.get(&point(9, 8)).is_none());

        let errors = strokes.finish_pointer_stroke(&mut session.ctx(), flat_view(), 0);
        assert!(errors.is_empty());
        assert_eq!(session.action_counter, 1);
    }

    #[test]
    fn stamp_press_without_a_hover_payload_is_a_no_op() {
        let mut session = Session::new();
        session
            .tool_state
            .get_mut()
            .set_tool_for_hand(PaintHand::Left, PaintTool::Stamp);

        let mut strokes = CanvasPointerStrokes::new();
        let bounds = canvas_bounds();
        strokes.begin_press(&mut session.ctx(), PaintHand::Left, point(5, 5), bounds, flat_view(), 0);
        assert!(session.canvas.get(&point(5, 5)).is_none());

        let errors = strokes.finish_pointer_stroke(&mut session.ctx(), flat_view(), 0);
        assert!(errors.is_empty());
        assert_eq!(session.action_counter, 0);
    }

    #[test]
    fn stamp_hover_for_one_hand_does_not_stamp_from_the_other() {
        let mut session = Session::new();
        {
            let tool_state = session.tool_state.get_mut();
            tool_state.set_tool_for_hand(PaintHand::Left, PaintTool::Stamp);
            tool_state.set_tool_for_hand(PaintHand::Right, PaintTool::Stamp);
        }

        let mut strokes = CanvasPointerStrokes::new();
        strokes.set_stamp_hover(Some(hover_for(PaintHand::Left, point(5, 5))));
        let bounds = canvas_bounds();
        strokes.begin_press(&mut session.ctx(), PaintHand::Right, point(5, 5), bounds, flat_view(), 0);
        assert!(session.canvas.get(&point(5, 5)).is_none());
    }

    #[test]
    fn stamp_hover_builds_a_two_phase_flash_preview() {
        let mut session = Session::new();
        {
            let tool_state = session.tool_state.get_mut();
            tool_state.set_tool_for_hand(PaintHand::Left, PaintTool::Stamp);
            tool_state.set_graphic_for_hand(PaintHand::Left, CellGraphic::Glyph('.'));
        }
        apply_brush(
            &mut session.canvas,
            point(5, 5),
            PaintedCell {
                graphic: CellGraphic::Glyph('a'),
                color: PaintColor::FlatRgb(255, 255, 255),
                weight_index: 1,
            },
        );

        let mut strokes = CanvasPointerStrokes::new();
        strokes.set_stamp_hover(Some(hover_for(PaintHand::Left, point(5, 5))));
        let groups = strokes.overlay_cell_groups(&mut session.ctx(), flat_view(), CellColor::Flat([0.5, 1.0, 0.75, 1.0]));
        // Two flash phases over the two copied cells.
        let overlay_cells: Vec<_> = groups.iter().flat_map(|group| group.iter_cells()).collect();
        assert_eq!(overlay_cells.len(), 4);
    }
}
