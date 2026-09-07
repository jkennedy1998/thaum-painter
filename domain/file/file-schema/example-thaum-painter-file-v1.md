# thaum painter file v1 example

## intent
This is the first implementation-ready saved file example for `domain/file/file-schema/`.

## top-level sections
- `kind`
  - file family marker
- `version`
  - schema version
- `metadata`
  - title, tags, document identity, timestamps
- `document`
  - stored authored modules, their intra-module group content, and the shared playback window
- `time_assets`
  - saved authored time-linked assets that are not renderer-owned
- `saved_camera_defaults`
  - reopen defaults only, not live camera session truth
- `import_export_bookkeeping`
  - painter-owned transfer notes

## module structure
- `document.module_order` / `document.modules[]` is the top-level authored unit. One module becomes exactly one renderer cell-group; see `thaum-painter/context/module-concept-audit.md`.
- each module carries its own `placement`, which is the global-board position `domain/rendering/render-space/` copies onto the renderer cell-group.
- each module owns `group_order` / `groups[]` for its intra-module content, nested one level deeper than in the pre-module shape.
- a group's `local_placement` is relative to its owning module's local coordinate space, not the shared board.

## ownership notes
- saved truth lives here even if render-space later derives something from it.
- `document.modules[*].groups[*].raster_segments` are stored authored content, not renderer-ready composed output.
- property blocks are saved authoring semantics, not direct renderer commands.
- `time_assets` remain file-owned even when render-space later turns some of them into renderer-facing lanes or preview-only helpers.
- `saved_camera_defaults` are intentionally weaker than live editor camera state.

## normalization direction
- `module_order` and each module's `group_order` are canonical even if `modules`/`groups` are stored as records in a runtime implementation.
- timing windows are explicit at group level so reopen behavior is stable.
- import/export bookkeeping stays optional and painter-owned.
- renderer-facing transient state should not be written back into this file unless it becomes authored truth.
