# thaum-painter/domain/file/storage

## purpose
Own persistence surfaces for saving, loading, autosaving, and locating thaum-painter files.

## owns
- local save/load policy
- autosave ownership
- file-path and storage-target conventions
- painter saved-file root resolution (`document_locations.rs` — "where files go" is storage policy)
- the load-time schema gate: kind + version checked before the body is trusted, typed `UnsupportedFileError` with an explicit version-difference message, so old/new-schema files reject cleanly instead of crashing or half-loading
- stored non-document session carryover that truly belongs to persistence

## does not own
- canonical file schema shape (owned by `../file-schema/`)
- renderer handoff
- live undo/redo state

## children-encapsulations
- none

## contents
- `contract.md`
  - storage contract
- `document_locations.rs`
  - painter file root resolution and per-root document loading (moved from `domain/file/document_locations.rs` 2026-09-07)
  - legacy root migration deleted 2026-09-07 (J-approved: roots empty, no long-term migration support; old files reject at the schema gate instead)

## dependencies
- `thaum-painter/domain/file/file-schema/`

## exposed interfaces
### shared-document snapshot persistence
send: shared-document runtime + paths (+ optional default document for first boot)
returns: loaded/reloaded runtime, or a `changed on disk` conflict error refusing the overwrite
effects: read-files, write-files
via: `load_or_create_shared_document`, `save_shared_document_snapshot`, `SharedDocumentRuntime::reload_from_disk`, `append_action_record`, `count_action_records`

### append atomicity guarantee
- `append_action_record` serializes each record into one buffer and issues a single
  `write_all` on an O_APPEND file, so a record is never torn across write syscalls
  and rival writers on a local filesystem cannot splice lines. Temp+rename would be
  wrong here (a wholesale replace silently drops records a rival appended in between);
  wholesale rewrites stay behind the log-divergence guard.

## interface consumers
- `thaum-painter/orchestration/entrypoint/`
- future file menus and autosave seams

## artifacts
- future autosave artifacts and file fixtures

## tests
- `storage.rs` inline `#[cfg(test)]` module
  - snapshot save/load round trips, revision conflict refusal, concurrent-append (log divergence) refusal, `reload_from_disk` adoption + local-history discard, undo/redo replay, history folding, block timing/split/blank/swap, binary re-tiling, selection channels, large-record append atomicity (`large_record_appends_stay_whole_records`).

## data
- none

## notes
- Rename + root absorption (2026-09-07, file-boundary cleanup plan): `document_locations.rs` root resolution moved here from a stray top-level file in `domain/file/` — "where files go" is storage policy. `migrate_legacy_painter_file_root` was deleted (both legacy roots are empty on this machine; J fixes friend files case-by-case, no long-term migration support).
- Load-time schema gate (2026-09-07): `load_document_file` checks `file_kind` + `schema_version` via `ensure_supported_file_schema` (from `file-schema/`) BEFORE deserializing the body. Rejection is a typed `UnsupportedFileError` (`KindMismatch | VersionMismatch`, path, found vs supported) with an explicit message ("file is schema v1, this app reads v2") — the schema version only changes when the shape breaks, so the number alone describes the mismatch. The open flow (`orchestration/build-commands` `file:open`) maps only this error class to a soft rejection: file does not open, no partial state, session keeps running.
- Dropped idea, recorded on purpose (2026-09-07): extracting the atomic-write helper into `tools/` was evaluated and REJECTED — a web/OPFS backend would not consume it and there is no second real consumer. Do not revisit until one exists.
- `storage.rs`'s live `SharedDocumentLayer`/`SharedDocumentRuntime` runtime schema (used by the running app) now carries `visible`/`locked`/`start_breath`/`length_breaths` per layer, with `set_layer_visible`, `set_layer_locked`, `rename_layer`, `remove_layer`, and `set_layer_timing` on the runtime, and `composited_canvas_in_layer_order` skips invisible layers. `start_breath`/`length_breaths` are purely an authoring/UI concept surfaced by `domain/modules/individuals/layers-panel/` for now — they do not gate compositing. This is intentionally still a simpler live/collaborative schema than the richer canonical save schema in `domain/file/file-schema/` (which also carries `opacity`, `placement`, full `timing`, and per-property blocks) — closing that full gap is future work, not part of this pass.
- Binary tiling (2026-09-07 bars redesign): every property track tiles the layer's full breath span — every breath is empty or solid, never a void. The runtime owns per-property-track block mutation for `domain/modules/individuals/layers-panel/`'s bar editing: `set_property_block_timing_pushed` (ripple, spacing-preserving), `set_property_block_timing_destructive` (the block takes its requested span; covered neighbors yield into blanks — partially overlapped ones keep only their remainder and turn empty with content discarded, fully covered ones are removed), `split_property_block` (returns the new right half's id), `blank_property_block` (turns a content block into a blank placeholder covering the same range), and `swap_property_blocks` (exchanges two blocks' breath ranges). The old clamp seam (`set_property_block_timing` + `properties::clamped_breath_span`) and the blank-merge seam (`merge_blank_property_block` + `PropertyBlockMergeDirection`) died with the void concept. After every pushed or destructive edit and on every `set_layer_timing` resize, the track re-tiles: blocks clip into the layer span and every uncovered window becomes a blank block (`retiled_property_track`), so coverage never breaks. The pushed seam must re-tile because a ripple shrink with no follower leaves a bare tail void, and a shift clamped at breath zero can strand an overlap (both were J-observed 2026-09-07). Coverage checks route through `properties::breath_in_span` so breath semantics stay defined in `domain/painter-document/properties/`.
- Split duplicates the channel's data. A split's two halves start as identical copies at different breaths (mirroring the old system's value-carrying split). The copy is not a canvas mutation on the side: `split_data_propagation_record` builds a full-cell `CellPatchSet` onto the new half (raster channels carry a canvas; data-free channels yield `None` and the halves stay empty), and orchestration records+applies it through the normal patch path — so live edits, replay, and save/load all rebuild the propagated data the same way.
- Property blocks carry an optional per-kind `value` (`serde_json::Value`, `serde(default)`) for value-carrying kinds. Move blocks hold an `{x,y,z}` render offset — the same value shape the old system's move properties and the file-schema import path use. The runtime owns the vector-move seam: `move_offset_for_layer(layer_id, breath)` resolves the block covering the breath (zero offset when track/block/value are absent or malformed — absence renders unshifted), and `add_move_offset(layer_id, breath, delta)` accumulates a world delta onto the covering block, auto-creating the move track and a block spanning the layer's timing window when missing. Move-offset edits persist through the document snapshot, not the per-cell undo log, like the other property-track metadata edits.
- Raster block timing now gates real content. `SharedDocumentRuntime` owns one canvas per raster block (`block_canvases` keyed `(layer_id, block_id)`), not one per layer. `canvas_for_layer(layer_id, current_breath)` resolves the raster block covering the playhead breath and returns its canvas — `None` for a breath in a gap between blocks, empty for blank blocks. `active_raster_block_id` is the paint-target resolver. Painting routes to the block under the playhead: `SharedDocumentAction::CellPatchSet` carries an optional `block_id` (serde default `None`), and painting into a blank block un-blanks it. Legacy records without `block_id` replay into the layer's first non-blank raster block, so pre-existing `actions.jsonl` logs stay replayable with no migration. Canvas-follows-block rules: `split_property_block` keeps the original canvas on the left block and the caller propagates a copy onto the right half through `split_data_propagation_record` (both halves start identical and diverge as they are edited separately); `blank_property_block` discards the block's content; `swap_property_blocks` exchanges only the breath ranges and leaves each canvas keyed to its own block, so content follows the block to its new span (swapping canvases too would be a visual no-op). `set_property_block_timing_destructive` turns covered victims into blanks: their canvases are discarded with them, and the edited block's vacated range re-tiles into a blank so no void appears. Undo/redo store the resolved `(layer_id, block_id)` target per action, so history works across playhead moves. Block structure mutations (split/blank/merge/swap/timing) are document mutations persisted through `document.json`, not action records — replay always starts from the saved document.
- The layers-panel playhead (`TimelineState::current_breath` in `domain/painter-session/timeline-state/`) is the breath gate for both painting and per-frame compositing (`build_document_layer_cell_groups(runtime, current_breath)` in `orchestration/entrypoint/src/main.rs`); the renderer's own breath clock (`data_lanes`) is not used for raster block selection. Each layer still becomes its own `CellGroup` per frame and the renderer composites in layer order — per-block canvases only changed which canvas feeds each group.
- Split-brain guard: `SharedDocumentFile.revision` (serde default 0) is a monotonic save counter. `save_shared_document_snapshot` now takes `&mut SharedDocumentRuntime` and verifies the on-disk revision still matches the runtime's loaded revision before overwriting — a second writer (another user or app instance) causes a loud `changed on disk` error instead of a silent last-writer-wins clobber that would split truth between document.json (structure) and actions.jsonl (content). The runtime bumps its revision only after both files are safely written, so a failed write stays retryable; `reload_from_disk` / `load_or_create_shared_document` re-seed the revision from disk, which is the recovery path after a conflict.
- Log divergence guard: strokes/undo records are APPENDED to actions.jsonl without a snapshot save, so another writer can grow the log without touching document.json — and every snapshot save rewrites the whole log from the writer's view. `save_shared_document_snapshot` therefore also counts on-disk records (`count_action_records`) and refuses with a `changed on disk` error when the log holds records this runtime never replayed. The revision guard alone would let a stale writer silently drop a rival's appended stroke.
- Conflict recovery: `SharedDocumentRuntime::reload_from_disk(paths)` adopts the on-disk truth wholesale (document + full log replay) and discards local in-memory state — undo/redo stacks, staged patches, unsaved local edits. The entrypoint's `recover_snapshot_conflict` detects the error shape, reloads, and resyncs the active layer, canvas, selection channel cache (`PainterSelection::replace_points`), and the action counter. The other writer's version always wins; local unsaved edits were refused anyway.
- Selection is document-owned, not layer-owned. `SharedDocumentFile.selection` (`SharedDocumentSelection`) carries named selection channels (`SharedDocumentSelectionChannel`: `channel_id` + a `BTreeSet<PersistedCellPoint>` 3D bitmap on the same coordinate system as painted cells). Only `DEFAULT_SELECTION_CHANNEL_ID` ("selection") exists until channel-picker UI arrives; the serde default materializes it for older `document.json` files, so pre-existing saves load unchanged. Selection is deliberately NOT part of the per-layer action log/undo: it is per file and shared across users, and changes persist through the document snapshot. `SharedDocumentRuntime` exposes `selection_points`/`selection_contains`/`apply_selection_points(channel, points, SharedSelectionWriteMode)` (Replace = whole-channel overwrite, Additive, Subtract, Intersect; unknown channel ids are created on demand; returns whether anything changed so no-op commits skip the snapshot save). The UI-side `PainterSelection` cell set in `domain/painter-session/selection/` is the interaction surface (full 3D set; `bounds` only marks the active interaction plane) and is mirrored exactly into the channel by `commit_selection_channel` in `orchestration/entrypoint/src/main.rs` on stroke release and on clear/invert/select-all — every commit is a full-set Replace, so channel == UI cache; boot restores the channel into the UI cache. `UNDO_HISTORY_DEPTH` (20) also lives here as the program-level undo seam constant: the number of recent action records that stay individually undoable before older history is squashed into the document snapshot (fold/squash runs on every snapshot save via `fold_history`).
- source-of-truth from J (2026-09-04): on jobo, the native rfd save dialog froze and then killed the whole program (only the program died, not the desktop). Cause: the GTK dialog enumerates trash through `gvfsd-trash`, which errors on this machine (`GFileInfo created without standard::name` bursts in the journal). Fix: the entrypoint sets `GIO_USE_VOLUME_MONITOR=unix` before any GTK/GIO init so dialogs skip the gvfs trash backend. If a future dialog path still chokes, the robust fallback is dropping the native dialog for typed in-app paths.
