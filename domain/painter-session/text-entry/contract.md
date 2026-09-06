# thaum-painter/domain/painter-session/text-entry

## purpose
Own the stateful live typing mode for the text tool: cursor ownership of the
keyboard, per-keystroke cell changes, and the Enter/exit commit granularity.

## owns
- the typing session state (`TextEntryState`): anchor, free view-plane cursor,
  line bookkeeping, pending cell changes
- key semantics inside typing: chars insert, Space advances (clearing the cell
  only when `space_replace`), Enter commits and steps by `enterlead`/`enterspace`,
  Backspace steps back / wraps to the previous line's end, Delete erases in
  place, arrows move the cursor freely, Home/End jump to the current line's
  start / typing end, Escape finishes
- brush capture at session start (the click's brush stays fixed for the whole
  session, matching the old `getBrushForButton(text_mode_button)` capture)

## does not own
- text layout math (`domain/painter-operations/text/`)
- `PaintedCell` composition from brush settings (the entrypoint captures the
  brush cell and hands it to `begin`)
- staging changes onto the shared document / committing records (session bridge
  `stage_text_entry_change` + `commit_staged_paint_stroke`)
- which physical keys map to `TextEntryKey` (entrypoint input translation)
- input-focus gating while typing (held keys, pointer, wheel, reserved
  camera/depth bindings — renderer `domain/controls/typing-mode/`)
- text tool properties UI panels

## children-encapsulations
- none

## contents
- `contract.md`
  - text-entry contract
- `text_entry.rs`
  - `TextEntryState`, `TextEntryKey`, `TextEntryOutcome`, `PendingChange`

## dependencies
- `thaum-painter/domain/painter-operations/text/`
- `thaum-renderer/domain/camera/`
- `thaum-renderer/domain/controls/typing-mode/` (the entrypoint
  gates dispatch through it while this session is active)

## exposed interfaces
### live typing session
send: anchor cell + view orientation + layout options + space_replace + captured brush cell, then `TextEntryKey`s
returns: per-key outcomes (`Applied` cell changes to stage, `Committed`/`Finished` commit points) and the pending change list
effects: none (pure session state; callers stage/commit)
via: `TextEntryState::begin`, `handle_key`, `take_pending`

## interface consumers
- `orchestration/entrypoint/` (click begins the session and the typing-mode
  gate; key loop routes through it)

## artifacts
- none

## tests
- `text_entry.rs` inline `#[cfg(test)]` module
  - char advance with spacing/charlead, Enter line stepping + commit outcome,
    space replace on/off, Backspace step-back + line wrap + anchor no-op,
    free arrow movement, Escape finishing with pending preserved

## data
- none

## notes
- Old-system reference: `THAUMWORLD-AUTO-STORY-TELLER/src/mono_ui/modules/painter_canvas_module.ts`
  (`text_mode_active` key handling, `commitPendingTextChanges('Type Text')`,
  `moveTextCursorToNextLine`) and `canvas_app/painter_app_state.ts`
  `handle_text_mode_reserved_shortcut` (camera swing/roll + depth step were the
  only reserved bindings during typing).
- Divergence from the old system: the old 2D grid exited typing when the cursor
  left the grid edge; the voxel canvas is unbounded, so no edge exit exists.
