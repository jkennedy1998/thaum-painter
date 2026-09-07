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
use thaum_renderer_domain::{
    project_world_relative_to_view, unproject_view_relative_to_world, CameraViewOrientation,
    CellColor, CellGraphic, CellGroup, CellPoint, ViewRelativePoint, WorldPoint,
};

use crate::{
    brush::{self, Canvas, PaintedCell},
    clipboard::WorldCopyData,
    fill::CanvasBounds,
    lasso_stroke::{
        build_lasso_path_cell_group, build_lasso_preview_cell_groups, LassoPreviewCell, LassoStroke,
    },
    paint_color::PaintColor,
    painter_tools::shared::{drag_behavior, selection_behavior, DragBehavior},
    selection_state::{PainterSelection, SelectionMode},
    selection_stroke::{build_plane_selection_cell_groups, interpolate_cell_path, SelectionStroke},
    session_document::{
        commit_move_offset, commit_selection_channel, commit_staged_paint_stroke,
        stage_image_edit_chunk, stage_painted_cells_chunk,
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

/// Accumulated view-space drag displacement that rebases when a drag frame
/// reports a changed camera orientation — the shared drag machinery behind
/// both move behaviors. With an unchanged orientation each frame's world
/// delta is exact cursor motion, depth included, so a focus-depth scroll
/// mid-drag carries the landing plane with it. When the orientation changed,
/// the offset rebases into the new view basis (the displacement the user saw
/// survives, rotated) and the anchor resets, because the world point under a
/// still cursor jumps on re-basing — that jump is noise, not motion.
#[derive(Debug, Clone)]
pub struct DragOffset {
    anchor: CellPoint,
    orientation: CameraViewOrientation,
    offset: ViewRelativePoint,
}

impl DragOffset {
    pub fn new(origin: CellPoint, orientation: CameraViewOrientation) -> Self {
        Self {
            anchor: origin,
            orientation,
            offset: ViewRelativePoint {
                right: 0,
                up: 0,
                depth: 0,
            },
        }
    }

    pub fn drag_to(&mut self, position: CellPoint, orientation: CameraViewOrientation) {
        if orientation != self.orientation {
            let world = unproject_view_relative_to_world(
                self.orientation,
                WorldPoint::origin(),
                self.offset,
            );
            self.offset = project_world_relative_to_view(orientation, WorldPoint::origin(), world);
            self.anchor = position;
        } else if position != self.anchor {
            let delta = project_world_relative_to_view(
                orientation,
                WorldPoint::origin(),
                WorldPoint {
                    x: position.x - self.anchor.x,
                    y: position.y - self.anchor.y,
                    z: position.z - self.anchor.z,
                },
            );
            self.offset.right += delta.right;
            self.offset.up += delta.up;
            self.offset.depth += delta.depth;
            self.anchor = position;
        }
        self.orientation = orientation;
    }

    /// The commit's 3D world displacement: the accumulated view-space
    /// offset unprojected through the orientation it accumulated in.
    pub fn world_offset(&self) -> (i32, i32, i32) {
        let world =
            unproject_view_relative_to_world(self.orientation, WorldPoint::origin(), self.offset);
        (world.x, world.y, world.z)
    }
}

/// One in-progress selection move: the acting hand, the press cell, the
/// plane-selection points captured at press, and the non-blank content
/// under them. The drag offset accumulates in view space and rebases
/// whenever a drag frame reports a changed camera orientation, so a
/// mid-drag swing/roll re-aims the displacement with the view and a
/// mid-drag focus-depth scroll lands the content at the new depth.
/// Release commits the whole move — clear the origin area, paste the
/// content at its translated position, re-anchor the selection there — as
/// one bounded undoable op.
#[derive(Debug, Clone)]
pub struct MoveStroke {
    pub hand: PaintHand,
    pub origin: CellPoint,
    pub origin_points: Vec<CellPoint>,
    pub content: Vec<(CellPoint, PaintedCell)>,
    offset: DragOffset,
}

impl MoveStroke {
    pub fn new(
        hand: PaintHand,
        origin: CellPoint,
        origin_points: Vec<CellPoint>,
        content: Vec<(CellPoint, PaintedCell)>,
        orientation: CameraViewOrientation,
    ) -> Self {
        Self {
            hand,
            origin,
            origin_points,
            content,
            offset: DragOffset::new(origin, orientation),
        }
    }

    pub fn drag_to(&mut self, position: CellPoint, orientation: CameraViewOrientation) {
        self.offset.drag_to(position, orientation);
    }

    pub fn world_offset(&self) -> (i32, i32, i32) {
        self.offset.world_offset()
    }
}

/// One in-progress vector move: a no-selection move drag that offsets
/// the active layer's render position (its `move` property block)
/// without touching raster data. Same drag machinery as the raster
/// selection move; release commits the whole delta as one move-offset
/// edit.
#[derive(Debug, Clone)]
pub struct VectorMoveStroke {
    pub hand: PaintHand,
    offset: DragOffset,
}

impl VectorMoveStroke {
    pub fn new(hand: PaintHand, origin: CellPoint, orientation: CameraViewOrientation) -> Self {
        Self {
            hand,
            offset: DragOffset::new(origin, orientation),
        }
    }

    pub fn drag_to(&mut self, position: CellPoint, orientation: CameraViewOrientation) {
        self.offset.drag_to(position, orientation);
    }

    pub fn world_offset(&self) -> (i32, i32, i32) {
        self.offset.world_offset()
    }
}

impl MoveStroke {
    /// The release commit's cell changes: clear every captured origin point,
    /// then paste the captured content at its translated position. Clearing
    /// runs first so a move that overlaps its own origin lands correctly.
    pub fn commit_changes(&self) -> Vec<(CellPoint, Option<PaintedCell>)> {
        let (dx, dy, dz) = self.world_offset();
        let mut changes: Vec<_> = self
            .origin_points
            .iter()
            .map(|point| (*point, None))
            .collect();
        changes.extend(self.content.iter().map(|(point, cell)| {
            (
                CellPoint {
                    x: point.x + dx,
                    y: point.y + dy,
                    z: point.z + dz,
                },
                Some(cell.clone()),
            )
        }));
        changes
    }

    /// Per-cell preview data for the in-flight move: the origin area
    /// flashes its current content against the cleared state while the
    /// destination flashes what is under it against the incoming content —
    /// the same two-phase pair the stamp paste preview shows.
    pub fn preview_cells(&self, canvas: &Canvas) -> Vec<LassoPreviewCell> {
        let (dx, dy, dz) = self.world_offset();
        let cleared = PaintedCell {
            graphic: CellGraphic::Glyph(' '),
            color: PaintColor::flat_rgb(0, 0, 0),
            weight_index: 3,
        };
        let mut previews: Vec<_> = self
            .origin_points
            .iter()
            .map(|point| LassoPreviewCell {
                point: *point,
                current: canvas.get(point).cloned(),
                upcoming: cleared.clone(),
            })
            .collect();
        previews.extend(self.content.iter().map(|(point, cell)| {
            let destination = CellPoint {
                x: point.x + dx,
                y: point.y + dy,
                z: point.z + dz,
            };
            LassoPreviewCell {
                point: destination,
                current: canvas.get(&destination).cloned(),
                upcoming: cell.clone(),
            }
        }));
        previews
    }
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
    move_stroke: Option<MoveStroke>,
    vector_move: Option<VectorMoveStroke>,
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
            move_stroke: None,
            vector_move: None,
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
        self.move_stroke = None;
        self.vector_move = None;
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
        if drag_behavior(pressed_tool_id) == DragBehavior::MoveSelection {
            // The move tool never begins a typing session or a selection
            // stroke. Without an active plane selection it is a deliberate
            // no-op stub for the future layer-offset behavior.
            if ctx.tool_state.borrow().hand_state(hand).target == PaintTarget::Image {
                let origin_points: Vec<CellPoint> = {
                    let selection = ctx.selection.borrow();
                    if selection.plane().has_selection() {
                        selection.plane().iter().collect()
                    } else {
                        Vec::new()
                    }
                };
                if !origin_points.is_empty() {
                    let content: Vec<(CellPoint, PaintedCell)> = origin_points
                        .iter()
                        .filter_map(|point| {
                            let cell = ctx.canvas.get(point).cloned()?;
                            if brush::is_blank_cell(&cell) {
                                return None;
                            }
                            Some((*point, cell))
                        })
                        .collect();
                    self.move_stroke = Some(MoveStroke::new(
                        hand,
                        position,
                        origin_points,
                        content,
                        orientation,
                    ));
                } else {
                    // No selection: the vector move. The drag offsets the
                    // active layer's render position through its move
                    // property block — raster data stays untouched.
                    self.vector_move = Some(VectorMoveStroke::new(hand, position, orientation));
                }
            }
            return None;
        }
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
                            tool_state.stamp_changes_for_hand(
                                ctx.canvas, &selection, &data, position, hand,
                            )
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
                    let mode = selection_behavior(ctx.tool_state.borrow().tool_for_hand(hand).id())
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

    /// Folds the release frame into an in-progress move stroke: a focus-depth
    /// scroll or view rotation after the last drag frame must still land. The
    /// entrypoint gates the position with its usual canvas eligibility and
    /// passes only on-canvas release points.
    pub fn fold_move_release(
        &mut self,
        hand: PaintHand,
        position: CellPoint,
        orientation: CameraViewOrientation,
    ) {
        if let Some(stroke) = self.move_stroke.as_mut() {
            if stroke.hand == hand {
                stroke.drag_to(position, orientation);
                return;
            }
        }
        if let Some(stroke) = self.vector_move.as_mut() {
            if stroke.hand == hand {
                stroke.drag_to(position, orientation);
            }
        }
    }

    /// The pending render shift of the in-flight vector move(s): the live
    /// WYSIWYG preview the entrypoint folds into the layer render path. The
    /// raster selection move previews through its own flash overlay instead.
    pub fn pending_move_offset(&self) -> Option<WorldPoint> {
        let mut total = WorldPoint::origin();
        let mut any = false;
        for stroke in self.vector_move.iter() {
            let (dx, dy, dz) = stroke.world_offset();
            total = WorldPoint {
                x: total.x + dx,
                y: total.y + dy,
                z: total.z + dz,
            };
            any = true;
        }
        any.then_some(total)
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
        if drag_behavior(ctx.tool_state.borrow().tool_for_hand(hand).id())
            == DragBehavior::MoveSelection
        {
            // The move strokes fold each frame into their view-space
            // offsets; the raster overlay previews the placement while the
            // vector one previews through the live render shift, and release
            // commits the whole move.
            if let Some(stroke) = self.move_stroke.as_mut() {
                if stroke.hand == hand {
                    stroke.drag_to(position, orientation);
                }
            } else if let Some(stroke) = self.vector_move.as_mut() {
                if stroke.hand == hand {
                    stroke.drag_to(position, orientation);
                }
            }
            return;
        }
        if drag_behavior(ctx.tool_state.borrow().tool_for_hand(hand).id())
            == DragBehavior::ClickOnly
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
        if let Some(stroke) = self.vector_move.take() {
            let (dx, dy, dz) = stroke.world_offset();
            if let Err(error) = commit_move_offset(
                ctx.document,
                ctx.document_paths,
                ctx.active_layer_id,
                WorldPoint {
                    x: dx,
                    y: dy,
                    z: dz,
                },
                current_breath,
            ) {
                errors.push(error);
            }
        }
        if let Some(stroke) = self.move_stroke.take() {
            if let Some(block_id) = ctx
                .document
                .active_raster_block_id(ctx.active_layer_id, current_breath)
            {
                let start = (ctx.canvas.clone(), block_id.clone());
                stage_painted_cells_chunk(
                    ctx.document,
                    ctx.canvas,
                    stroke.commit_changes(),
                    ctx.active_layer_id,
                    &block_id,
                );
                // One move is one undo step: the staged commit loop below
                // diffs the press snapshot against the moved canvas.
                self.set_image_start(stroke.hand, start);
                // Re-anchor the selection at the moved location as an exact
                // 3D replacement: a mid-drag view rotation can legitimately
                // leave the moved set spanning depths, so this path never
                // plane-filters. The channel mirror is the shared truth, the
                // plane set the interaction surface — the same split a
                // selection stroke commits through.
                let (dx, dy, dz) = stroke.world_offset();
                let translated = stroke.origin_points.iter().map(|point| CellPoint {
                    x: point.x + dx,
                    y: point.y + dy,
                    z: point.z + dz,
                });
                ctx.selection
                    .borrow_mut()
                    .replace_plane_points_exact(translated);
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
                    let mode =
                        selection_behavior(ctx.tool_state.borrow().tool_for_hand(stroke.hand).id())
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
        if let Some(stroke) = &self.move_stroke {
            // Two-phase move preview: the origin area flashes its content
            // against the cleared state while the destination flashes what
            // release will place there.
            let previews = stroke.preview_cells(ctx.canvas);
            groups.extend(build_lasso_preview_cell_groups(&previews, vivid));
        }
        groups
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        brush::{apply_brush, PaintedCell},
        document_locations::new_unsaved_document,
        layers_runtime::resolved_active_layer_id,
        paint_color::PaintColor,
        session_document::sync_canvas_from_active_layer,
        tool_state::PaintTool,
    };
    use thaum_renderer_domain::{
        camera_view_orientation_for_camera, CameraRoll, CameraSwing, CellColor, CellGraphic,
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

    fn point_at(x: i32, y: i32, z: i32) -> CellPoint {
        CellPoint { x, y, z }
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
                "canvas-pointer-test-{}-{}",
                std::process::id(),
                {
                    use std::sync::atomic::{AtomicUsize, Ordering};
                    static SESSION_SEQ: AtomicUsize = AtomicUsize::new(0);
                    SESSION_SEQ.fetch_add(1, Ordering::Relaxed)
                }
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
        strokes.begin_press(
            &mut session.ctx(),
            PaintHand::Left,
            point(0, 0),
            bounds,
            flat_view(),
            0,
        );
        strokes.continue_drag(
            &mut session.ctx(),
            PaintHand::Left,
            point(2, 0),
            bounds,
            flat_view(),
        );
        strokes.continue_drag(
            &mut session.ctx(),
            PaintHand::Left,
            point(2, 2),
            bounds,
            flat_view(),
        );
        strokes.continue_drag(
            &mut session.ctx(),
            PaintHand::Left,
            point(0, 2),
            bounds,
            flat_view(),
        );
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
        strokes.begin_press(
            &mut session.ctx(),
            PaintHand::Left,
            point(0, 0),
            bounds,
            flat_view(),
            0,
        );
        let path_before = strokes.lasso().unwrap().path.clone();
        strokes.continue_drag(
            &mut session.ctx(),
            PaintHand::Right,
            point(5, 5),
            bounds,
            flat_view(),
        );
        assert_eq!(strokes.lasso().unwrap().path, path_before);

        strokes.continue_drag(
            &mut session.ctx(),
            PaintHand::Left,
            point(2, 0),
            bounds,
            flat_view(),
        );
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
        strokes.begin_press(
            &mut session.ctx(),
            PaintHand::Left,
            point(0, 0),
            bounds,
            flat_view(),
            0,
        );
        assert!(strokes.selection().is_some());
        strokes.continue_drag(
            &mut session.ctx(),
            PaintHand::Left,
            point(1, 0),
            bounds,
            flat_view(),
        );
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
        strokes.begin_press(
            &mut session.ctx(),
            PaintHand::Left,
            point(0, 0),
            bounds,
            flat_view(),
            0,
        );
        strokes.continue_drag(
            &mut session.ctx(),
            PaintHand::Left,
            point(2, 2),
            bounds,
            flat_view(),
        );

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
        strokes.begin_press(
            &mut session.ctx(),
            PaintHand::Left,
            point(5, 5),
            bounds,
            flat_view(),
            0,
        );
        // The stamp lands with the copied cells' own appearance, staged
        // not committed yet.
        let painted = session.canvas.get(&point(5, 5)).unwrap();
        assert_eq!(painted.graphic, CellGraphic::Glyph('a'));
        assert!(session.canvas.get(&point(6, 5)).is_some());
        assert_eq!(session.action_counter, 0);

        // Drags never stamp: a click-only tool places exactly once.
        strokes.continue_drag(
            &mut session.ctx(),
            PaintHand::Left,
            point(8, 8),
            bounds,
            flat_view(),
        );
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
        strokes.begin_press(
            &mut session.ctx(),
            PaintHand::Left,
            point(5, 5),
            bounds,
            flat_view(),
            0,
        );
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
        strokes.begin_press(
            &mut session.ctx(),
            PaintHand::Right,
            point(5, 5),
            bounds,
            flat_view(),
            0,
        );
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
        let groups = strokes.overlay_cell_groups(
            &mut session.ctx(),
            flat_view(),
            CellColor::Flat([0.5, 1.0, 0.75, 1.0]),
        );
        // Two flash phases over the two copied cells.
        let overlay_cells: Vec<_> = groups.iter().flat_map(|group| group.iter_cells()).collect();
        assert_eq!(overlay_cells.len(), 4);
    }

    fn brush_cell(graphic: char) -> PaintedCell {
        PaintedCell {
            graphic: CellGraphic::Glyph(graphic),
            color: PaintColor::FlatRgb(255, 255, 255),
            weight_index: 1,
        }
    }

    #[test]
    fn move_release_folds_a_depth_scroll_that_followed_the_last_drag_frame() {
        let mut session = Session::new();
        session
            .tool_state
            .get_mut()
            .set_tool_for_hand(PaintHand::Left, PaintTool::Move);
        apply_brush(&mut session.canvas, point(1, 1), brush_cell('#'));
        session
            .selection
            .borrow_mut()
            .apply_plane_points_with_mode([point(1, 1)], SelectionMode::Replace);

        let mut strokes = CanvasPointerStrokes::new();
        let bounds = canvas_bounds();
        strokes.begin_press(
            &mut session.ctx(),
            PaintHand::Left,
            point(0, 0),
            bounds,
            flat_view(),
            0,
        );
        strokes.continue_drag(
            &mut session.ctx(),
            PaintHand::Left,
            point(2, 0),
            bounds,
            flat_view(),
        );
        // The user scrolls the focus depth to z=2 and releases without any
        // further pointer motion: the release frame folds the depth delta.
        strokes.fold_move_release(PaintHand::Left, point_at(2, 0, 2), flat_view());
        let errors = strokes.finish_pointer_stroke(&mut session.ctx(), flat_view(), 0);
        assert!(errors.is_empty());

        assert!(session.canvas.get(&point(1, 1)).is_none());
        assert_eq!(
            session.canvas.get(&point_at(3, 1, 2)).unwrap().graphic,
            CellGraphic::Glyph('#')
        );
    }

    #[test]
    fn move_release_fold_ignores_the_other_hand() {
        let mut session = Session::new();
        session
            .tool_state
            .get_mut()
            .set_tool_for_hand(PaintHand::Left, PaintTool::Move);
        apply_brush(&mut session.canvas, point(1, 1), brush_cell('#'));
        session
            .selection
            .borrow_mut()
            .apply_plane_points_with_mode([point(1, 1)], SelectionMode::Replace);

        let mut strokes = CanvasPointerStrokes::new();
        let bounds = canvas_bounds();
        strokes.begin_press(
            &mut session.ctx(),
            PaintHand::Left,
            point(0, 0),
            bounds,
            flat_view(),
            0,
        );
        strokes.continue_drag(
            &mut session.ctx(),
            PaintHand::Left,
            point(2, 0),
            bounds,
            flat_view(),
        );
        // A right-hand release folds nothing into the left hand's move.
        strokes.fold_move_release(PaintHand::Right, point_at(2, 0, 2), flat_view());
        let errors = strokes.finish_pointer_stroke(&mut session.ctx(), flat_view(), 0);
        assert!(errors.is_empty());

        assert_eq!(
            session.canvas.get(&point(3, 1)).unwrap().graphic,
            CellGraphic::Glyph('#')
        );
    }

    #[test]
    fn move_press_drag_release_moves_the_selected_content_as_one_commit() {
        let mut session = Session::new();
        session
            .tool_state
            .get_mut()
            .set_tool_for_hand(PaintHand::Left, PaintTool::Move);
        apply_brush(&mut session.canvas, point(1, 1), brush_cell('#'));
        apply_brush(&mut session.canvas, point(1, 2), brush_cell('a'));
        session
            .selection
            .borrow_mut()
            .apply_plane_points_with_mode([point(1, 1), point(1, 2)], SelectionMode::Replace);

        let mut strokes = CanvasPointerStrokes::new();
        let bounds = canvas_bounds();
        strokes.begin_press(
            &mut session.ctx(),
            PaintHand::Left,
            point(0, 0),
            bounds,
            flat_view(),
            0,
        );
        strokes.continue_drag(
            &mut session.ctx(),
            PaintHand::Left,
            point(3, 1),
            bounds,
            flat_view(),
        );
        let errors = strokes.finish_pointer_stroke(&mut session.ctx(), flat_view(), 0);
        assert!(errors.is_empty());

        // The origin area is cleared, the content sits at the new place:
        // the drag from (0,0) to (3,1) offsets everything by (+3,+1).
        assert!(session.canvas.get(&point(1, 1)).is_none());
        assert!(session.canvas.get(&point(1, 2)).is_none());
        assert_eq!(
            session.canvas.get(&point(4, 2)).unwrap().graphic,
            CellGraphic::Glyph('#')
        );
        assert_eq!(
            session.canvas.get(&point(4, 3)).unwrap().graphic,
            CellGraphic::Glyph('a')
        );
        // One bounded move, one undo step.
        assert_eq!(session.action_counter, 1);
        // The selection stays active, re-anchored at the new location.
        let plane: Vec<CellPoint> = session.selection.borrow().plane().iter().collect();
        assert!(plane.contains(&point(4, 2)));
        assert!(plane.contains(&point(4, 3)));
        assert!(!plane.contains(&point(1, 1)));
    }

    #[test]
    fn move_without_a_selection_offsets_the_layer_through_its_move_block() {
        let mut session = Session::new();
        session
            .tool_state
            .get_mut()
            .set_tool_for_hand(PaintHand::Left, PaintTool::Move);
        apply_brush(&mut session.canvas, point(1, 1), brush_cell('#'));

        let mut strokes = CanvasPointerStrokes::new();
        let bounds = canvas_bounds();
        strokes.begin_press(
            &mut session.ctx(),
            PaintHand::Left,
            point(1, 1),
            bounds,
            flat_view(),
            0,
        );
        strokes.continue_drag(
            &mut session.ctx(),
            PaintHand::Left,
            point(4, 1),
            bounds,
            flat_view(),
        );
        // The pending offset previews through the render path before commit.
        assert_eq!(
            strokes.pending_move_offset(),
            Some(WorldPoint { x: 3, y: 0, z: 0 })
        );
        let errors = strokes.finish_pointer_stroke(&mut session.ctx(), flat_view(), 0);
        assert!(errors.is_empty());

        // Raster data is untouched; the offset landed on the active layer's
        // move block covering the current breath.
        assert_eq!(
            session.canvas.get(&point(1, 1)).unwrap().graphic,
            CellGraphic::Glyph('#')
        );
        assert_eq!(session.action_counter, 0);
        let offset = session.document.move_offset_for_layer(&session.layer_id, 0);
        assert_eq!(offset, WorldPoint { x: 3, y: 0, z: 0 });
        // The preview clears once the offset is committed.
        assert_eq!(strokes.pending_move_offset(), None);
    }

    #[test]
    fn cancel_drops_an_in_flight_vector_move_without_committing() {
        let mut session = Session::new();
        session
            .tool_state
            .get_mut()
            .set_tool_for_hand(PaintHand::Left, PaintTool::Move);
        apply_brush(&mut session.canvas, point(1, 1), brush_cell('#'));

        let mut strokes = CanvasPointerStrokes::new();
        let bounds = canvas_bounds();
        strokes.begin_press(
            &mut session.ctx(),
            PaintHand::Left,
            point(1, 1),
            bounds,
            flat_view(),
            0,
        );
        strokes.continue_drag(
            &mut session.ctx(),
            PaintHand::Left,
            point(4, 1),
            bounds,
            flat_view(),
        );
        strokes.cancel(PaintHand::Left);
        let errors = strokes.finish_pointer_stroke(&mut session.ctx(), flat_view(), 0);
        assert!(errors.is_empty());

        assert_eq!(
            session.document.move_offset_for_layer(&session.layer_id, 0),
            WorldPoint::origin()
        );
    }

    #[test]
    fn move_drag_builds_a_two_phase_flash_preview() {
        let vivid = CellColor::Flat([1.0, 0.0, 0.0, 1.0]);
        let mut session = Session::new();
        session
            .tool_state
            .get_mut()
            .set_tool_for_hand(PaintHand::Left, PaintTool::Move);
        apply_brush(&mut session.canvas, point(1, 1), brush_cell('#'));
        session
            .selection
            .borrow_mut()
            .apply_plane_points_with_mode([point(1, 1)], SelectionMode::Replace);

        let mut strokes = CanvasPointerStrokes::new();
        let bounds = canvas_bounds();
        strokes.begin_press(
            &mut session.ctx(),
            PaintHand::Left,
            point(0, 0),
            bounds,
            flat_view(),
            0,
        );
        strokes.continue_drag(
            &mut session.ctx(),
            PaintHand::Left,
            point(2, 1),
            bounds,
            flat_view(),
        );

        // Two flash phases over the origin cell plus the destination cell.
        let groups = strokes.overlay_cell_groups(&mut session.ctx(), flat_view(), vivid);
        let overlay_cells: Vec<_> = groups.iter().flat_map(|group| group.iter_cells()).collect();
        assert!(overlay_cells.len() >= 4);

        let errors = strokes.finish_pointer_stroke(&mut session.ctx(), flat_view(), 0);
        assert!(errors.is_empty());
    }

    #[test]
    fn move_drag_follows_a_mid_drag_focus_depth_scroll_to_the_new_depth() {
        let mut session = Session::new();
        session
            .tool_state
            .get_mut()
            .set_tool_for_hand(PaintHand::Left, PaintTool::Move);
        apply_brush(&mut session.canvas, point(1, 1), brush_cell('#'));
        session
            .selection
            .borrow_mut()
            .apply_plane_points_with_mode([point(1, 1)], SelectionMode::Replace);

        let mut strokes = CanvasPointerStrokes::new();
        let bounds = canvas_bounds();
        strokes.begin_press(
            &mut session.ctx(),
            PaintHand::Left,
            point(0, 0),
            bounds,
            flat_view(),
            0,
        );
        // Drag right two cells on the z=0 plane…
        strokes.continue_drag(
            &mut session.ctx(),
            PaintHand::Left,
            point(2, 0),
            bounds,
            flat_view(),
        );
        // …then the user scrolls the focus depth to z=2: the same cursor
        // view position now maps to a world point two units deeper, and
        // that depth delta joins the accumulated offset.
        strokes.continue_drag(
            &mut session.ctx(),
            PaintHand::Left,
            point_at(2, 0, 2),
            bounds,
            flat_view(),
        );
        let errors = strokes.finish_pointer_stroke(&mut session.ctx(), flat_view(), 0);
        assert!(errors.is_empty());

        // The move lands on the new depth, not the origin's.
        assert!(session.canvas.get(&point(1, 1)).is_none());
        assert!(session.canvas.get(&point_at(3, 1, 0)).is_none());
        assert_eq!(
            session.canvas.get(&point_at(3, 1, 2)).unwrap().graphic,
            CellGraphic::Glyph('#')
        );
        let plane: Vec<CellPoint> = session.selection.borrow().plane().iter().collect();
        assert!(plane.contains(&point_at(3, 1, 2)));
    }

    #[test]
    fn move_drag_rebases_its_offset_when_the_view_rotates_mid_drag() {
        let mut session = Session::new();
        session
            .tool_state
            .get_mut()
            .set_tool_for_hand(PaintHand::Left, PaintTool::Move);
        apply_brush(&mut session.canvas, point(0, 0), brush_cell('#'));
        session
            .selection
            .borrow_mut()
            .apply_plane_points_with_mode([point(0, 0)], SelectionMode::Replace);

        // PosZ view: right is East (+x), up is Top (+y).
        let posz = flat_view();
        // PosX view: right is North (-y), up is Top (+y), depth is East (+x).
        let posx = thaum_renderer_domain::camera_view_orientation_for_camera(
            CameraSwing::PosX,
            CameraRoll::Deg0,
        );

        let mut strokes = CanvasPointerStrokes::new();
        let bounds = canvas_bounds();
        strokes.begin_press(
            &mut session.ctx(),
            PaintHand::Left,
            point(0, 0),
            bounds,
            posz,
            0,
        );
        // Drag three cells right in the PosZ view.
        strokes.continue_drag(
            &mut session.ctx(),
            PaintHand::Left,
            point(3, 0),
            bounds,
            posz,
        );
        // The user swings to the PosX view: the still cursor's world point
        // re-bases onto the new plane; that jump is noise, not motion.
        strokes.continue_drag(
            &mut session.ctx(),
            PaintHand::Left,
            point(0, -3),
            bounds,
            posx,
        );
        // Then the user drags two more cells right — now along North.
        strokes.continue_drag(
            &mut session.ctx(),
            PaintHand::Left,
            point(0, -5),
            bounds,
            posx,
        );
        let errors = strokes.finish_pointer_stroke(&mut session.ctx(), posx, 0);
        assert!(errors.is_empty());

        // The displacement followed the cursor in view space: three east
        // before the swing, two north after it.
        assert!(session.canvas.get(&point(0, 0)).is_none());
        assert_eq!(
            session.canvas.get(&point(3, -2)).unwrap().graphic,
            CellGraphic::Glyph('#')
        );
        let plane: Vec<CellPoint> = session.selection.borrow().plane().iter().collect();
        assert!(plane.contains(&point(3, -2)));
    }
}
