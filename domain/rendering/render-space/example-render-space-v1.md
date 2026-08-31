# render-space v1 example

## intent
This is the first implementation-ready renderer handoff example for `domain/rendering/render-space/`.

## mapping direction
### file-owned input
- one `cell_groups[]` entry is emitted per `document.modules[*]`, not per group; see `/home/j/Repos/thaum-painter/context/module-concept-audit.md`.
- a module's intra-module groups come from `document.modules[*].groups[*].raster_segments`
- stored timing and property blocks decide which segment/move-block is active at the chosen breath
- each group's `local_placement` (plus any active move-block offset) is applied before merging its cells into the owning module's single cell field
- saved camera defaults provide baseline camera values

### live-session input
- active breath overrides reopen defaults
- live camera target/focus may override saved camera defaults
- visibility, preview, or selection-driven choices may affect what gets handed off for preview

## output shape
- `camera`
  - render-facing camera values already normalized for the renderer handoff
- `composition`
  - explicit module ordering for overlap-sensitive assembly
- `cell_groups`
  - one renderer-facing cell-group per module, carrying the module's global `placement` and its composited, resolved cells
- `data_lanes`
  - renderer-standard dynamic lanes such as `breath`
- `derived`
  - translation diagnostics and ignored-field notes that help implementation and tests

## important notes
- this is not a saved file.
- this is not the editor session state.
- this is the transient handoff payload built from saved truth plus live overrides.
- render-space should emit active, composited module content after timing/property resolution, not raw authored segments and never one cell-group per intra-module group.
- file-only metadata and bookkeeping should be ignored unless a renderer-facing use case proves otherwise.
