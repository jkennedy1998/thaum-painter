# ascii painter feature audit

## purpose
Capture the major features in the old ASCII painter and map them into cleaner thaum-painter encapsulations before implementation starts.

## old feature clusters observed
- document/runtime core
  - `src/ascii_painter/painter_document.ts`
  - `src/ascii_painter/painter_document_runtime.ts`
  - `src/ascii_painter/painter_session_core.ts`
  - `src/ascii_painter/painter_session_types.ts`
- time and playback
  - `src/ascii_painter/painter_breath.ts`
  - `src/ascii_painter/painter_time_assets.ts`
- drawing tools and geometry
  - `src/ascii_painter/tools.ts`
  - `src/shared/painter_tools.ts`
  - `src/ascii_painter/edit_mask.ts`
- selection and clipboard
  - `src/ascii_painter/selection.ts`
  - `src/ascii_painter/world_selection.ts`
  - `src/ascii_painter/copy_paste.ts`
- persistence and transfer
  - `src/ascii_painter/save_system.ts`
  - `src/ascii_painter/export.ts`
  - `src/shared/painter_document_store.ts`
  - `src/shared/painter_hosted_session_store.ts`
- import helpers
  - `src/ascii_painter/image_import.ts`
- rendering and camera adapters
  - `src/ascii_painter/painter_view_projection_adapter.ts`
  - `src/ascii_painter/camera/painter_camera_resolver.ts`
  - `src/ascii_painter/voxel_dom_renderer.ts`
  - `src/mono_ui/modules/painter_canvas_module.ts`
- document structure helpers
  - group/property/timing behavior spread through runtime and breath helpers
- auxiliary authoring systems
  - `src/ascii_painter/gradiator.ts`
  - `src/ascii_painter/gradiator_storage.ts`
- module identity and placement
  - `src/ascii_painter/module_position_storage.ts`
  - see `module-concept-audit.md` for the full reassignment: this is not an auxiliary system, it is the saved module wrapper that maps 1:1 onto a `thaum-renderer` `CellGroup`

## strongest old capabilities
- saved painter document with grouped voxel content
- time-aware document playback by breath/frame windows
- group timing, visibility, lock state, ordering, and property blocks
- draw, erase, line, rect, bucket, text, and shape-oriented authoring behavior
- selection in both plane and world-like coordinates
- copy/paste with richer payload preservation than plain text
- image import to ASCII
- persistent tool settings and autosave
- export paths for painter document and game-ish asset payloads
- camera focus helpers and render adapters for editor preview

## clean first-pass ownership split
### `domain/file/`
Own saved file shape and stored app truth.
- manifest/versioning
- authoring metadata
- module identity, module order, and module-level (global-board) placement
- intra-module group order and stored content sections
- stored timing/playback defaults
- stored import/export bookkeeping
- stored tool/session persistence that should survive app restarts
- migration from legacy formats

### `domain/rendering/render-space/`
Own how saved or live state becomes renderer-facing state.
- renderer-ready camera payload
- renderer-ready group/cell/data-lane handoff
- derived preview/render state
- mapping rules for ignored painter-only fields

### `domain/rendering/camera/`
Own app-side camera intent before renderer consumption.
- focus subject resolution
- editor preview camera defaults
- camera state derived from document/selection/tool anchors

### `domain/painter-document/`
Own editing-facing document views over file state.
- document convenience projections
- module/group/property/timing lookup helpers for editing
- editor-friendly normalization

### `domain/painter-session/`
Own live mutation and authoring session state.
- commands
- undo/redo
- selection state
- clipboard state
- tool state

### `domain/painter-operations/`
Own pure headless operations.
- brush cell application
- shape raster ops
- fill ops
- text stamping
- image-import transforms

## recommended contract-first encapsulations
- `domain/file/manifest/`
- `domain/file/storage/`
- `domain/file/import-export/`
- `domain/file/migration/`
- `domain/rendering/camera/`
- `domain/rendering/render-space/`
- `domain/painter-document/modules/`
- `domain/painter-document/groups/`
- `domain/painter-document/timing/`
- `domain/painter-document/properties/`
- `domain/painter-session/commands/`
- `domain/painter-session/history/`
- `domain/painter-session/selection/`
- `domain/painter-session/clipboard/`
- `domain/painter-session/tool-state/`
- `domain/painter-operations/brush/`
- `domain/painter-operations/shapes/`
- `domain/painter-operations/fill/`
- `domain/painter-operations/text/`
- `domain/painter-operations/image-import/`

## defer until core contracts are stable
- broad UI modules
- pointer choreography
- toolbar/menu shells
- DOM-specific renderers
- hosted/shared presence stores

## notes
- the old painter had real power, but many concerns were fused together.
- the refactor should create small truth-owning seams before rebuilding behavior.
- renderer-facing data should stay aligned to `thaum-renderer`, but painter should keep ownership of saved app structure and authoring workflow state.
