# thaum painter roadmap

## first boundaries to build
- `domain/file/`
  - canonical app-owned file manifest, saved asset shape, group identity/placement, and all authoring data the renderer does not own
- `domain/rendering/`
  - app-side rendering seams including camera intent and render-space handoff into `thaum-renderer`, including the flat group-to-cell-group mapping
- `domain/painter-document/`
  - painter editing-facing document view over file state, including `groups/` (the top-level authored unit, one group per renderer `CellGroup`) alongside `timing/` and `properties/`
- `domain/modules/`
  - the renderer-owned interactive UI-panel format (color picker, character picker, and similar), mirrored per-consumer; painter's own `individuals/` holds painter-only panels like the character picker that the game never uses
- `domain/painter-session/`
  - reducer-like editing/session state surface over painter-document
- `domain/painter-operations/`
  - pure draw/fill/line/text/import operations independent from UI

## defer until after headless core
- the `domain/modules/` UI-panel format itself (canvas/toolbar/menu shell composition, i.e. `painter_canvas_module.ts`'s future home) and its gizmo/registry machinery
- mouse-heavy selection systems
- clipboard/paste chrome
- deep multiplayer/editor shells

## notes
- use `context/module-concept-audit.md` for the full module/group history: an intermediate `document.modules[*].groups[]` wrapper shape was tried and reversed back to a flat `document.groups[]`, one saved group mapping directly to exactly one `thaum-renderer` `CellGroup`. "module" now means only the `domain/modules/` UI-panel format, never a data/authoring concept.
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
