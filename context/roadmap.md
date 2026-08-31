# thaum painter roadmap

## first boundaries to build
- `domain/file/`
  - canonical app-owned file manifest, saved asset shape, module identity/placement, and all authoring data the renderer does not own
- `domain/rendering/`
  - app-side rendering seams including camera intent and render-space handoff into `thaum-renderer`, including the module-to-cell-group mapping
- `domain/painter-document/`
  - painter editing-facing document view over file state, including `modules/` (the top-level authored unit) alongside `groups/`, `timing/`, and `properties/`
- `domain/painter-session/`
  - reducer-like editing/session state surface over painter-document
- `domain/painter-operations/`
  - pure draw/fill/line/text/import operations independent from UI

## defer until after headless core
- UI modules (canvas/toolbar/menu shell composition, i.e. `painter_canvas_module.ts`'s future home; this is a distinct concept from the `modules/` domain seam below and should not be confused with it)
- mouse-heavy selection systems
- gizmos
- clipboard/paste chrome
- deep multiplayer/editor shells

## notes
- use `context/module-concept-audit.md` for why `domain/painter-document/modules/` and `document.modules[]` exist: one saved module is the top-level authored unit and maps to exactly one `thaum-renderer` `CellGroup`.
- start from the old `painter_document.ts`, `painter_document_runtime.ts`, `painter_session_core.ts`, `copy_paste.ts`, `selection.ts`, `painter_breath.ts`, and related feature seams, but split them into cleaner ownership boundaries first.
- do a contract-first pass across most core encapsulations before code-heavy rebuild work.
- keep renderer as the owner of the low-level render primitives and directly consumed scene semantics.
- keep `domain/file/` as the owner of saved app asset structure and all non-renderer authoring state.
- keep `domain/rendering/render-space/` as the owner of translating file/live app state into the renderer-facing handoff shape.
- keep one clear renderer handoff seam and avoid restoring the old broad bridge habit under a new name.
- use `context/ascii-painter-feature-ownership-matrix.md` as the current audit map for old-feature-to-new-seam ownership.
- use `context/third-pass-encapsulation-review.md` as the current call on which seams are ready, which should wait, and which would only split under real implementation pressure.
- use `context/implementation-ready-shapes.md` as the index for the first concrete saved-file and render-space examples.
- use `context/example-alignment-review.md` as the current check that the wider contract tree still matches those concrete examples.
