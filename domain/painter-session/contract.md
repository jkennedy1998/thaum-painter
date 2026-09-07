# thaum-painter/domain/painter-session

## purpose
Own the bounded editing/session semantics that mutate a painter document over time.

## owns
- active document session state
- command/reducer-style mutations over painter documents
- undo/redo-friendly change packet semantics
- active breath/frame selection and group/layer focus state

## does not own
- renderer core format ownership
- UI module rendering
- low-level raster algorithms better placed in painter-operations

## children-encapsulations
- `commands/`
  - default
- `history/`
  - default
- `selection/`
  - default
- `clipboard/`
  - default
- `identity/`
  - default
- `tool-state/`
  - default
- `timeline-state/`
  - default
- `sync/`
  - default

## contents
- `contract.md`
  - painter-session contract
- `session_document.rs`
  - session→document bridge: staged strokes, stroke commits, undo/redo records, selection channel commits, snapshot-conflict recovery (`recover_snapshot_conflict`), shared action ids (minted with the session's `user_id` baked in via `next_action_id`, so ids are globally unique); `stage_painted_cells_chunk` stages resolved cell changes (text-entry keystrokes, stamp placements) without creating an action record
  - the former `app_runtime.rs` headless-runtime draft (never wired into `lib.rs`, never compiled) was deleted — the entrypoint frame loop is the live implementation, and its pointer-stroke logic was superseded by the session_document bridge + tool_state

## dependencies
- `thaum-painter/domain/painter-document/`
- `thaum-painter/domain/painter-operations/`

## exposed interfaces
### session→document bridge
send: runtime + live session state (canvas, tool state, selection, action counter)
returns: document mutations (staged patches, committed records, reverted history) + persisted snapshot/log writes
effects: write-files (actions.jsonl, document.json via storage)
via: `stage_image_edit_chunk`, `commit_staged_paint_stroke`, `commit_selection_channel`, `commit_move_offset`, `apply_shared_history_action`, `recover_snapshot_conflict`

## interface consumers
- `orchestration/entrypoint/` (the live app frame loop)

## artifacts
- none

## tests
- `session_document.rs` / `selection_state.rs` / `tool_state.rs` inline test modules
  - session bridge + selection + tool semantics validated through `cargo test -p thaum-painter-domain`

## data
- none

## notes
- the old `painter_session_core.ts`, selection helpers, copy/paste helpers, and tool persistence should come back through smaller session seams rather than one fused runtime boundary.
