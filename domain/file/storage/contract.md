# /home/j/Repos/thaum-painter/domain/file/storage

## purpose
Own persistence surfaces for saving, loading, autosaving, and locating thaum-painter files.

## owns
- local save/load policy
- autosave ownership
- file-path and storage-target conventions
- stored non-document session carryover that truly belongs to persistence

## does not own
- canonical file manifest shape
- renderer handoff
- live undo/redo state

## children-encapsulations
- none

## contents
- `contract.md`
  - storage contract

## dependencies
- `/home/j/Repos/thaum-painter/domain/file/manifest/`

## exposed interfaces
- none

## interface consumers
- future app boot
- future file menus

## artifacts
- future autosave artifacts and file fixtures

## tests
- future save/load tests

## data
- none

## notes
- `storage.rs`'s live `SharedDocumentLayer`/`SharedDocumentRuntime` runtime schema (used by the running app) now carries `visible`/`locked`/`start_breath`/`length_breaths` per layer, with `set_layer_visible`, `set_layer_locked`, `rename_layer`, `remove_layer`, and `set_layer_timing` on the runtime, and `composited_canvas_in_layer_order` skips invisible layers. `start_breath`/`length_breaths` are purely an authoring/UI concept surfaced by `domain/modules/individuals/layers-panel/` for now — they do not gate compositing. This is intentionally still a simpler live/collaborative schema than the richer canonical save schema in `domain/file/manifest/` (which also carries `opacity`, `placement`, full `timing`, and per-property blocks) — closing that full gap is future work, not part of this pass.
- `SharedDocumentRuntime` also owns per-property-track block mutation for `domain/modules/individuals/layers-panel/`'s bar editing: `set_property_block_timing`, `split_property_block`, `blank_property_block` (turns a content block into a blank placeholder covering the same range), `merge_blank_property_block` (extends a blank's left/right content neighbor over the blank's range and removes it — `PropertyBlockMergeDirection`), and `swap_property_blocks` (exchanges two blocks' breath ranges). None of these consume/rewrite an overlapping neighbor block or auto-fill an exposed gap with a new blank the way the old system's "destructive" edge/split semantics did — each only ever touches the block(s) named in the call.
