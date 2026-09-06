# render-space v1 example

## intent
This is the first implementation-ready renderer handoff example for `domain/rendering/render-space/`.

## mapping direction
### file-owned input
- one `cell_groups[]` entry is emitted per `document.groups[*]`, direct 1:1 — no module wrapper, no intra-unit compositing step; see `thaum-painter/context/module-concept-audit.md`'s "superseded" section.
- each group's active raster segment comes straight from `document.groups[*].raster_segments`
- stored timing and property blocks decide which segment/move-block is active at the chosen breath
- a voxel's cell position is its own local `x/y/z` plus any active move-block offset; the group's directly authored `placement` becomes the emitted cell-group's origin, not part of the cell position itself
- saved camera defaults provide baseline camera values

### live-session input
- active breath overrides reopen defaults
- live camera target/focus may override saved camera defaults
- visibility, preview, or selection-driven choices may affect what gets handed off for preview

## output shape
- `camera`
  - render-facing camera values already normalized for the renderer handoff
- `composition`
  - explicit group ordering for overlap-sensitive assembly
- `cell_groups`
  - one renderer-facing cell-group per authored group, carrying the group's directly authored `placement` and its resolved, active-breath cells
- `data_lanes`
  - renderer-standard dynamic lanes such as `breath`
- `derived`
  - translation diagnostics and ignored-field notes that help implementation and tests

## important notes
- this is not a saved file.
- this is not the editor session state.
- this is the transient handoff payload built from saved truth plus live overrides.
- render-space should emit active, resolved group content after timing/property resolution, not raw authored segments.
- file-only metadata and bookkeeping should be ignored unless a renderer-facing use case proves otherwise.
