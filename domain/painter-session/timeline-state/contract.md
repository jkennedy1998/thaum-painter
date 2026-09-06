# thaum-painter/domain/painter-session/timeline-state

## purpose
Own the live, unsaved playhead and auto-key state that drives which breath a session is editing at.

## owns
- the current playhead breath
- the auto-key toggle (per user/session, not per property, not saved to the file)
- the rule for which breath a property edit is allowed to land on: auto-key on always allows the current playhead breath; auto-key off only allows the breath when an existing bar already covers it, otherwise the edit is rejected

## does not own
- saved document timing/playback state (`document_window_*`, `frames_per_breath`, loop window) — that is `painter-document/timing/`
- bar/property-block lookup itself — that is `painter-document/properties/`
- bar mutation (create/move/trim/split/merge) — that is `painter-operations/`
- timeline UI drawing, scrubbing, or hit-testing — that is the future `layers-panel` module

## children-encapsulations
- none

## contents
- `contract.md`
  - timeline-state contract
- `timeline_state.rs`
  - `TimelineState` (current breath, auto-key flag) and `resolve_editable_breath` (the auto-key/reject-edit rule)

## dependencies
- `thaum-painter/domain/painter-document/properties/`

## exposed interfaces
- none

## interface consumers
- painter-session
- future `layers-panel` module

## artifacts
- none

## tests
- inline `#[cfg(test)]` in `timeline_state.rs`
  - light
  - validates auto-key-on always allows the current breath, auto-key-off allows only a breath already covered by a bar, and auto-key-off rejects edits when the playhead sits in a gap

## data
- none

## notes
- source-of-truth from J: auto-key is a per-user/session toggle, not per property track.
- source-of-truth from J: when auto-key is off and the playhead is not inside any existing bar for the property being edited, the edit must be rejected outright — no falling back to the nearest bar like the old system did.
