# thaum-painter/domain/modules/individuals

## purpose
Own painter's own concrete modules — panels the ASCII painter needs that the game (or other thaum-renderer consumers) never will.

## owns
- one folder per painter-only module, each with its own `contract.md`, e.g. color picker, character picker, toolbar

## does not own
- modules generic enough for any consumer, which belong in `thaum-renderer/domain/modules/individuals/` instead (e.g. a controls remap panel)
- the shared logic these modules are built from, owned by `thaum-renderer/domain/modules/shared/` and `domain/modules/shared/`

## children-encapsulations
- `toolbar/`
  - default
- `toolbox/`
  - default
- `paint-color-picker/`
  - default
- `paint-color-block/`
  - default
- `material-picker/`
  - default
- `hand-settings/`
  - default
- `graphic-picker/`
  - default
- `layers-panel/`
  - default
- `session-panel/`
  - default

## contents
- `contract.md`
  - individuals contract

## dependencies
- `thaum-painter/domain/modules/`
- `thaum-renderer/domain/modules/shared/`

## exposed interfaces
- none

## interface consumers
- `thaum-painter/orchestration/entrypoint/`

## artifacts
- none

## tests
- none yet

## data
- none

## notes
- `toolbar/` was the first small proof module built here: a row of tool buttons implementing `thaum-renderer`'s `Module` trait directly.
- `toolbox/` is the first old-UX rebuild that directly matters for painter workflow: left click assigns the left-hand tool, right click assigns the right-hand tool, matching J's source-of-truth preference from the old painter.
- `paint-color-picker/` binds the renderer's generic swatch picker into painter's left/right hand color state instead of forking the picker itself.
- `paint-color-block/` owns arbitrary RGB picking over the shared hand color seam.
- `material-picker/` owns material palette picking over that same hand color seam.
- `hand-settings/` owns the compact weight/mask/target/lock controls tied to shared hand state.
- `graphic-picker/` owns choosing the live hand `graphic` itself, showing sprite choices plus glyphs grouped by Thaum Mono section order.
- remaining candidates from the old system's `src/mono_ui/modules/`: `character_editor_module.ts`, `painter_file_menu_module.ts`.
- `layers-panel/` is the old system's `groups_module.ts` rebuild. Current pass: layer list (select, add, delete, per-layer visibility/lock toggle) plus a labeled timeline header (auto-key + scrubbable breath ruler/playhead) wired to `painter-session/timeline-state/`, with nested raster/move property rows under the selected layer as the real edit surface. This required extending `SharedDocumentLayer` (`domain/file/storage/`) with `visible`/`locked`/`start_breath`/`length_breaths` fields and matching `SharedDocumentRuntime` methods, since the live runtime schema previously only carried `layer_id`+`name`. The panel was also widened (`ModuleRect { x0: 25, y0: -24, x1: 70, y1: -3 }`) to give the timeline region real room, matching the old system's wider `groups_module.ts` panel proportions.
- Still deliberately deferred on `layers-panel/`: rename (needs a `Module` trait text-input/keyboard hook that does not exist yet — a cross-repo `thaum-renderer` change), drag-based reorder, a document-level frames-per-breath control (needs a playback-settings seam on the live document schema, mirroring the `visible`/`locked`/timing gaps already closed for layers), and the deeper shared bar-editing seam for blank/create/merge/compact behaviors (`painter-operations/bar-editing/`).
