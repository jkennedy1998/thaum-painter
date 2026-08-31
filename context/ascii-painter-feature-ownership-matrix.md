# ascii painter feature ownership matrix

## purpose
Map the old ASCII painter feature surface into the new thaum-painter encapsulations so each major behavior has one clear primary owner, clear secondary consumers, and a visible growth-risk note.

## owner legend
- primary owner: the seam that should hold the main truth or behavior
- secondary consumers: seams that read, adapt, or trigger the feature without owning it
- pressure: where a seam may need to split later if the feature grows

## matrix

| feature | old files | primary owner | secondary consumers | why this owner | pressure / future split |
| --- | --- | --- | --- | --- | --- |
| top-level saved file kind/version | `painter_document.ts`, `save_system.ts` | `domain/file/manifest/` | `domain/file/storage/`, `domain/file/import-export/`, `domain/painter-document/`, `domain/rendering/render-space/` | canonical saved truth belongs in the file seam | low |
| saved document bounds and stored content sections | `painter_document.ts` | `domain/file/manifest/` | `domain/painter-document/groups/`, `domain/rendering/render-space/` | this is persisted authoring truth, not runtime-only state | medium if stored content format gets large |
| saved module identity, order, and module-level (global-board) placement | `module_position_storage.ts` | `domain/file/manifest/` | `domain/painter-document/modules/`, `domain/rendering/render-space/` | one saved module maps to exactly one renderer `CellGroup`; see `module-concept-audit.md` | medium if module count/placement rules grow |
| saved authoring metadata/title/description/tags | `painter_document.ts`, `save_system.ts` | `domain/file/manifest/` | `domain/file/storage/`, `domain/file/import-export/` | catalog/reopen concerns are file-owned | low |
| saved camera defaults/preferences | `painter_document.ts`, `voxel_space.ts` | `domain/file/manifest/` | `domain/rendering/camera/`, `domain/rendering/render-space/` | defaults may be saved, but live motion should not be | medium |
| saved time assets / particle-like assets | `painter_time_assets.ts`, `painter_document.ts` | `domain/file/manifest/` | `domain/painter-document/timing/`, `domain/rendering/render-space/` | these are saved authored assets first | medium-high |
| save/load/autosave file persistence | `save_system.ts` | `domain/file/storage/` | `orchestration/`, future UI/file menu surfaces | storage policy is separate from manifest shape | medium |
| import/export wrapper formats | `export.ts`, `save_system.ts` | `domain/file/import-export/` | `domain/file/manifest/`, `domain/rendering/render-space/` | transfer wrappers should not own canonical file truth | medium |
| legacy grid / voxel-space / painter-document upgrade | `save_system.ts`, `painter_document_legacy_adapter.ts`, `voxel_space.ts` | `domain/file/migration/` | `domain/file/manifest/`, `domain/file/import-export/` | compatibility is its own burden and should stay isolated | medium |
| editing-facing module lookup/order/visibility/lock/placement views | `module_position_storage.ts`, `painter_document.ts` | `domain/painter-document/modules/` | `domain/painter-document/groups/`, `domain/painter-session/commands/`, `domain/rendering/render-space/` | module-facing convenience views should not own saved file or live session | medium |
| editing-facing group lookup/order/visibility/lock views (intra-module) | `painter_document_runtime.ts`, `painter_document.ts` | `domain/painter-document/groups/` | `domain/painter-document/modules/`, `domain/painter-session/commands/`, `domain/rendering/camera/`, `domain/rendering/render-space/` | document-facing convenience views should not own saved file or live session | medium |
| editing-facing timing and breath lookup | `painter_breath.ts`, `painter_document_runtime.ts` | `domain/painter-document/timing/` | `domain/painter-session/commands/`, `domain/rendering/render-space/` | authoring-facing timing reads are different from saved timing truth | high if time assets expand |
| editing-facing property block lookup | `painter_document.ts`, `painter_document_runtime.ts` | `domain/painter-document/properties/` | `domain/painter-session/commands/`, `domain/rendering/render-space/` | property inspection helpers should stay separate from mutation logic | medium-high |
| runtime visible voxel/group resolution | `painter_document_runtime.ts` | `domain/rendering/render-space/` | `domain/painter-document/groups/`, `domain/painter-document/timing/` | this is derived handoff-oriented state, not saved truth | high; likely split later |
| renderer-ready cell/module/composition/data-lane handoff (one `CellGroup` per module) | `painter_document_runtime.ts`, `painter_view_projection_adapter.ts` | `domain/rendering/render-space/` | `domain/rendering/camera/`, `thaum-renderer` seams | this is the canonical painter-to-renderer adapter seam; the renderer does not need to know about modules, only the resulting `CellGroup` | high; likely split into scene/timing/preview children |
| live camera intent/focus subject resolution | `painter_camera_resolver.ts`, `voxel_space.ts` | `domain/rendering/camera/` | `domain/rendering/render-space/`, `domain/painter-session/selection/`, `domain/painter-document/groups/` | live camera decisions are app rendering state, not file truth | medium |
| projected preview space / view-projection helpers | `painter_view_projection_adapter.ts` | `domain/rendering/render-space/` | `domain/rendering/camera/` | projection exists to feed preview/render handoff | high if editor preview becomes rich |
| group and timing mutation command vocabulary | `painter_session_types.ts`, `painter_session_core.ts` | `domain/painter-session/commands/` | `domain/painter-document/*`, `domain/painter-operations/*` | commands coordinate changes without owning pure ops or saved truth | medium |
| live undo/redo and action grouping | `history.ts` | `domain/painter-session/history/` | `domain/painter-session/commands/`, future UI surfaces | history is live session policy | medium |
| plane selection bitmap | `selection.ts` | `domain/painter-session/selection/` | `domain/painter-session/history/`, `domain/rendering/camera/` | live selection state is session truth | medium |
| world selection set | `world_selection.ts`, `painter_canvas_module.ts` | `domain/painter-session/selection/` | `domain/painter-session/clipboard/`, `domain/rendering/camera/` | still session truth, just another selection space | high if 3d selection tools expand |
| live clipboard buffer | `copy_paste.ts`, `world_selection.ts` | `domain/painter-session/clipboard/` | `domain/painter-session/selection/`, `domain/painter-operations/text/`, `domain/painter-operations/image-import/` | authoring clipboard state is live session concern | medium |
| clipboard payload codecs for rich/world payloads | `copy_paste.ts`, `world_selection.ts` | `domain/painter-session/clipboard/` | `domain/file/import-export/` | currently editor-session oriented, not durable exchange format | high; may split into codecs child later |
| live paste transform intent (scale/angle/anchor/ignore rules) | `copy_paste.ts`, `gradiator.ts`, `painter_canvas_module.ts` | `domain/painter-session/clipboard/` | `domain/painter-session/tool-state/`, `domain/painter-operations/text/`, `domain/painter-operations/image-import/` | this is live authoring state around paste | high |
| active tool selection and brush hand state | `painter_canvas_module.ts`, `save_system.ts` | `domain/painter-session/tool-state/` | `domain/painter-session/commands/`, `domain/painter-operations/*` | active hand/tool settings are session truth | medium |
| persisted tool preferences across launches | `save_system.ts` | `domain/file/storage/` | `domain/painter-session/tool-state/` | persistent storage is separate from live tool state | medium |
| single-cell draw / erase semantics | `tools.ts` | `domain/painter-operations/brush/` | `domain/painter-session/commands/` | pure edit logic should stay headless | low |
| line / rect / lasso / primitive rasterization | `tools.ts`, shared geometry, `painter_canvas_module.ts` | `domain/painter-operations/shapes/` | `domain/painter-session/selection/`, `domain/painter-session/commands/` | geometry derivation is pure behavior | medium-high |
| flood fill rules | `tools.ts` | `domain/painter-operations/fill/` | `domain/painter-session/commands/`, `domain/painter-session/tool-state/` | fill traversal is pure behavior | low |
| text entry / text stamping | `tools.ts`, `tools_text_entry.test.ts` | `domain/painter-operations/text/` | `domain/painter-session/tool-state/`, `domain/painter-session/clipboard/`, `domain/painter-session/commands/` | pure text placement belongs with operations, not session | medium |
| image to ASCII conversion | `image_import.ts` | `domain/painter-operations/image-import/` | `domain/painter-session/clipboard/`, `domain/file/import-export/` | import-time raster transforms are pure operations | medium |
| character ramp / gradiator editing state | `gradiator.ts`, `gradiator_storage.ts` | `domain/painter-session/tool-state/` | `domain/painter-operations/image-import/`, `domain/file/storage/` | the chosen ramp is live authoring state; persistence is separate | medium |
| document lineage / authoritative revision sync | `painter_session_core.ts` | `domain/painter-session/commands/` | `domain/file/storage/`, future collab/network seams | sync metadata lives with session mutation orchestration | medium-high |
| group-plane registry / plane assignment helpers | `painter_session_core.ts` | `domain/painter-session/selection/` | `domain/rendering/camera/`, `domain/painter-document/groups/` | this is live editor navigation state more than file or renderer truth | medium |
| big canvas interaction choreography | `painter_canvas_module.ts` | future `modules/` seam, not current domain seams | consumes nearly all domain seams | UI orchestration should sit above the domain seams | very high; never re-fuse into one truth owner |

## current hot spots to watch

### `domain/rendering/render-space/`
Likely future child seams:
- `scene/`
- `timing/`
- `preview/`
- `visible-stack/`

### `domain/painter-session/clipboard/`
Likely future child seams:
- `buffer/`
- `codecs/`
- `paste-preview/`

### `domain/painter-document/timing/`
Likely future split:
- editing-facing timing view
- saved time asset structure stays in `domain/file/manifest/`
- render-facing timing mapping stays in `domain/rendering/render-space/`

### `domain/file/manifest/`
Watch for stored content complexity around:
- group content sections
- time asset sections
- saved camera defaults
- migration version edges

## immediate conclusions
- the current high-level split is good.
- the old system's biggest fusion point was not one missing seam, but many responsibilities being mixed inside runtime and UI layers.
- the new system should keep one owner per feature and let other seams consume it, not shadow it.
- the main likely future splits are render-space, clipboard, and timing.
- `module_position_storage.ts` was originally mis-bucketed under "auxiliary authoring systems." It is a structural concept, not an extra: one saved module maps to exactly one `thaum-renderer` `CellGroup`, and `domain/painter-document/modules/` plus `document.modules[]` in `domain/file/manifest/` now own it directly. See `module-concept-audit.md`.
