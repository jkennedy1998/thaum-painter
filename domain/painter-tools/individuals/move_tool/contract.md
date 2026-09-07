# thaum-painter/domain/painter-tools/individuals/move_tool

## purpose
Own the move tool's registration truth as one tool encapsulation.

## owns
- the `MoveTool` registration type resolving move's `ToolDescriptor` (id "move", label, icon, no property rows, move hotkey action)
- move's future session behavior home (selection-move application lives in `painter-session/canvas-pointer`, gating in `painter-session/tool-state`)

## does not own
- the registry list
- the selection set or its document channel (`painter-session/selection/`)
- the stroke commit path (`painter-session/session_document`)
- hand-state channel resolution (`painter-session/tool-state`)

## children-encapsulations
- none

## contents
- `contract.md`
  - move tool contract
- `move_tool.rs`
  - `MoveTool` `RegisteredTool` impl + descriptor test

## dependencies
- the registry seam (`domain/painter-tools/`)

## exposed interfaces
- none

## interface consumers
- `domain/painter-tools/` registry

## artifacts
- none

## tests
- inline `#[cfg(test)]` in `move_tool.rs`
  - light
  - validates move's descriptor truth

## data
- none

## notes
- source-of-truth from J: one Move tool, two behaviors split by whether an active selection exists. With a selection: press-drag-release moves the selected raster content — release clears the old selection area, pastes the content at the new place, and re-anchors the selection there (still active). Without a selection: the tool will offset the layer's render position without touching raster data (animation movement), deferred; the current no-selection behavior is a stub that does nothing.
- interaction truth from J: the move preview flashes exactly like the stamp/lasso previews — two-phase vivid flash showing where the content will land — and one drag is one bounded undoable move.
- 3D truth from J: a move drag must survive camera changes mid-drag. If the user swings or rolls the view mid-drag, the displacement follows the cursor in view space, so the final move re-aims with the view (the landed displacement can come out rotated); if the user changes the focus-plane depth mid-drag (drawing-space scroll or the focus-depth keys), the content lands at the new depth. The selection re-anchors as an exact 3D set — it may legitimately span depths after a mid-drag rotation.
- behavior split truth from J: one Move tool, two behaviors. With a selection the drag moves raster content; without one it authors the active layer's `move` property block — a positional render offset over time, no raster data change. No rotation is bundled into either move behavior; rotation will be a separate tool J builds later (out of scope for the current edits).
- hotkey: `painter_select_move` on M, declared in the descriptor and drift-tested by `tai/`.
