# thaum-painter/domain/painter-tools/individuals/stamp

## purpose
Own the stamp tool's registration truth as one tool encapsulation.

## owns
- the `StampTool` registration type resolving stamp's `ToolDescriptor` (id "stamp", label, icon, no property rows, stamp hotkey action)
- stamp's future session behavior home (payload-to-canvas application lives in `painter-session/tool-state` stamp seams, staging in `painter-session/canvas-pointer`)

## does not own
- the registry list
- the clipboard payload shape or per-user buffers (`painter-session/clipboard/`)
- hand-state channel resolution or selection gating (`painter-session/tool-state`, `painter-session/selection/`)

## children-encapsulations
- none

## contents
- `contract.md`
  - stamp tool contract
- `stamp_tool.rs`
  - `StampTool` `RegisteredTool` impl + descriptor test

## dependencies
- the registry seam (`domain/painter-tools/`)

## exposed interfaces
- none

## interface consumers
- `domain/painter-tools/` registry

## artifacts
- none

## tests
- inline `#[cfg(test)]` in `stamp_tool.rs`
  - light
  - validates stamp's descriptor truth

## data
- none

## notes
- source-of-truth from J: pasting is a tool, not a bare key. V equips Stamp; the stamp places the user's copied cells at the click cell instead of the old paste-at-cursor keybinding.
- source-of-truth from J (supersedes the earlier "stamped edits obey the locked hand" rule): the stamp pastes the copied cells' own graphic/color/weight — the copy is the content, the hand's brush state never leaks into pasted cells. Enabled edit channels paste the copied value; a locked channel keeps the target cell's existing value. A gfx-locked stamp on an empty target edits nothing: the point is skipped entirely (a space carries no color or weight).
- preview truth from J: while stamping, the user sees a two-phase vivid flash of the paste area — the cells as currently drawn vs the cells the stamp will place — like the lasso's live interior preview.
- hotkey: `painter_select_stamp` on V (formerly `painter_clipboard_paste`), declared in the descriptor and drift-tested by `tai/`.
