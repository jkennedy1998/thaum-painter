# domain/painter-session/canvas-pointer

## purpose
session-owned, per-hand pointer stroke lifecycle on the paint canvas: press dispatch, drag continuation, and release commit — written once and shared by both hands instead of duplicated per hand in the entrypoint.

## owns
- the in-progress pointer stroke state across both hands: the lasso bound, the selection stroke, each hand's staged image-stroke start, and each hand's last drag position
- per-tool dispatch across the three pointer phases (press begins, drag continues, release commits) for both selection-target and image-target hands
- the text tool's press-to-typing-session handoff (`TypingSessionBegin`); typing-session keyboard ownership stays with the entrypoint
- the release commit invariants: a selection stroke mirrors into the document channel as an exact replacement, a closed lasso fills or selects its enclosed region, and each hand's staged stroke commits once per release (one undo per stroke)
- cancel semantics for chrome hits: in-progress strokes clear regardless of owning hand; the cancelling hand's drag position clears; staged starts survive

## does not own
- screen-space hit testing (command bar, module captures, drawing-space gizmos) — the entrypoint guards events before calling the seams
- typing-session keyboard ownership, key translation, or text commit routing (`text-entry` owns the session, the entrypoint owns the wiring)
- tool state truth (`tool-state`), selection truth (`selection/selection_state`), or document persistence (`file/storage`, `painter-session/session_document`)
- overlay preview cell-group building (`tool-state/lasso_stroke`, `selection/selection_stroke`); the entrypoint reads the in-progress strokes through the accessors
- pure rasterization (`painter-operations`)

## children-encapsulations
- none

## contents
- `canvas_pointer.rs`
  - `CanvasPointerStrokes` (state + `begin_press` / `continue_drag` / `finish_selection_stroke` / `finish_pointer_stroke` / `cancel` / accessors), `CanvasPointerContext`, `TypingSessionBegin`, `StrokeStart`

## dependencies
- painter-session/tool-state: ToolState, PaintHand, PaintTarget, PaintTool, LassoStroke
- painter-session/selection: PainterSelection, SelectionMode
- painter-session/selection: SelectionStroke, interpolate_cell_path (selection_stroke)
- painter-session/session_document: stage_image_edit_chunk, commit_staged_paint_stroke, commit_selection_channel
- painter-session/text-entry: TextEntryState
- file/storage: SharedDocumentRuntime, SharedDocumentPaths
- painter-tools/shared: selection_behavior
- painter-operations/fill: CanvasBounds

## exposed interfaces
### CanvasPointerStrokes::begin_press — dispatch one canvas press for a hand
send: { ctx: &mut CanvasPointerContext, hand: PaintHand, position: CellPoint, bounds: CanvasBounds, orientation: CameraViewOrientation, current_breath: u32 }
returns: Option<TypingSessionBegin>
effects: mutates in-progress stroke state, stages image edits, reads document blocks

### CanvasPointerStrokes::continue_drag — continue the owning hand's stroke
send: { ctx: &mut CanvasPointerContext, hand: PaintHand, position: CellPoint, bounds: CanvasBounds, orientation: CameraViewOrientation }
returns: ()
effects: mutates in-progress stroke state, stages image edits, applies per-position selection edits

### CanvasPointerStrokes::finish_selection_stroke — commit a finished selection stroke
send: { ctx: &mut CanvasPointerContext, current_breath: u32 }
returns: ()
effects: mutates selection, commits the selection channel, writes document snapshots

### CanvasPointerStrokes::finish_pointer_stroke — release lasso bounds and staged strokes
send: { ctx: &mut CanvasPointerContext, orientation: CameraViewOrientation, current_breath: u32 }
returns: Vec<anyhow::Error> (commit errors for the entrypoint to report)
effects: paints lasso fills, commits selection channels and staged patch sets, writes document snapshots

### CanvasPointerStrokes::overlay_cell_groups — live preview overlays for in-progress strokes
send: { ctx: &mut CanvasPointerContext, orientation: CameraViewOrientation, vivid: CellColor }
returns: Vec<CellGroup> (plane-selection preview; open lasso bound path plus flash-preview groups)
effects: none — reads state only, never commits

## interface consumers
- orchestration/entrypoint (the painter event loop)

## tests
- `canvas_pointer`
  - light
  - validates press/drag/release dispatch per hand: lasso bound + fill parity, selection stroke begin, text typing handoff, other-hand drag isolation, cancel semantics, one-commit-per-stroke release, and overlay groups following in-progress strokes

## notes
- This seam exists because the entrypoint carried two near-identical copies of the press/drag/release dispatch (one per hand) and the copies drifted; new tools implement behavior in `tool-state` (or their own session module) and surface here only through the existing dispatch rules.
- A press with no raster block under the playhead breath begins nothing.
- Overlay previews must show exactly what the release commit will paint: the overlay target dispatch mirrors `finish_pointer_stroke`'s image/selection dispatch.
- Text-tool presses return the typing session instead of owning it here so keyboard routing stays with the entrypoint's input ownership seams.
