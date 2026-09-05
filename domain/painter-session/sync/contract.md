# /home/j/Repos/thaum-painter/domain/painter-session/sync

## purpose
Own the multiplayer convergence proof seam: independent document sessions syncing through one totally-ordered action-record log.

## owns
- `LoopbackActionBus` — the totally-ordered shared record log (loopback stand-in for the future multiplayer transport; order of the log is the order every session applies)
- `SyncedDocumentSession` — one user's editing session over its own `SharedDocumentRuntime` plus a per-session action counter and a read cursor into the shared log
- the publish semantics: forward edits apply locally then append to the log; undo/redo history records push onto the local history stacks without re-applying (mirroring the live `apply_shared_history_action` flow)
- the sync semantics: foreign records apply in log order, own records are skipped (they were applied at publish time), the cursor advances past the whole log
- convergence tests proving any number of sessions reach identical canvas state, including late joiners and undo-as-revert-record propagation

## does not own
- transport/networking itself (the bus is an in-memory loopback; a future socket host ships the same records)
- structure edits (layer add/remove/reshape) — those stay on the `document.json` revision path with the split-brain guard in `file/storage/`
- disk persistence of the log (`actions.jsonl` append stays in `file/storage/`)
- presence/cursor rendering

## children-encapsulations
- none

## contents
- `document_sync.rs`
  - the loopback bus, synced session, and convergence tests

## dependencies
- `/home/j/Repos/thaum-painter/domain/file/storage/`
- `/home/j/Repos/thaum-painter/domain/painter-session/session_document.rs`
- `/home/j/Repos/thaum-renderer/domain/`

## exposed interfaces
- `LoopbackActionBus::publish` — append one record to the shared log
- `SyncedDocumentSession::publish_action` — apply locally, then append (forward edits)
- `SyncedDocumentSession::publish_history_record` — push onto local history stacks without re-applying, then append (undo/redo revert records)
- `SyncedDocumentSession::sync_from_bus` — apply all unseen foreign records in log order, skipping own; returns the applied count

## interface consumers
- future `workers/` session host (the socket seam ships exactly these records)
- future live multiplayer wiring in the entrypoint

## artifacts
- none

## tests
- `document_sync` inline `#[cfg(test)]` module
  - two sessions converge on interleaved strokes
  - a session skips its own records on sync (no double-apply)
  - late joiners catch up on the full log
  - undo-as-revert-record converges across sessions
  - three sessions converge on a shared log

## data
- none

## notes
- convergence rests entirely on one shared totally-ordered log; per-cell conflict resolution is last-writer-in-log-order (patches carry full after-state)
- source-of-truth from J (2026-09-04): multiplayer prep starts with this convergence harness — prove two runtimes converge by exchanging action records before any networking exists, so the future session host is just "ship records, apply records"
- source-of-truth from J: undo is already the undo-as-operation multiplayer model (passive revert records in the all-forward-edits log), so cross-session undo needs no new record shapes
