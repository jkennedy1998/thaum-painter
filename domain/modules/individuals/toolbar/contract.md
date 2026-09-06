# thaum-painter/domain/modules/individuals/toolbar

## purpose
Own painter's toolbar panel: a row of selectable tool buttons (brush, eraser, fill, ...), implementing `thaum-renderer`'s `Module` trait so it can be registered and drawn like any other renderer module.

## owns
- the `ToolbarModule` type and its `ToolbarButton` data shape
- toolbar button layout, active-tool selection state, and click-to-select behavior
- the `Module` trait implementation (`draw`, `on_pointer_event`) for this panel

## does not own
- the `Module` trait itself or `ModuleRegistry` semantics, owned by `thaum-renderer/domain/modules/`
- wiring the active tool into real painting behavior (drawing voxels, editing the manifest) — this only tracks which tool is selected
- toolbar placement/boot wiring in a running window, owned by whichever consumer registers it (currently `orchestration/entrypoint/`)

## children-encapsulations
- none

## contents
- `toolbar_module.rs`
  - `ToolbarModule`, `ToolbarButton`, and their `Module` trait implementation

## dependencies
- `thaum-renderer/domain/modules/`

## exposed interfaces
- `ToolbarModule::new(id, rect, buttons)` — construct a toolbar with a fixed button set
- `ToolbarModule::active_tool()` — currently selected tool name, if any

## interface consumers
- `thaum-painter/orchestration/entrypoint/`

## artifacts
- none

## tests
- inline `#[cfg(test)]` in `toolbar_module.rs`
  - light
  - validates button layout/spacing, default active tool, click-to-select, and out-of-bounds clicks being ignored

## data
- none

## notes
- first concrete module built under `domain/modules/individuals/`, proving painter can own a `Module` implementation the same way `thaum-renderer`'s own `BlankPanelModule` proves the trait itself.
- button set is currently hardcoded by the caller (see `orchestration/entrypoint/src/main.rs`); wiring a real, configurable tool list is deferred until painter has real tool behavior to select between.
- real mouse clicks now reach `on_pointer_event` end-to-end: `orchestration/entrypoint/` converts the renderer's clip-space click into a world/cell point and dispatches it through `ModuleRegistry::dispatch_pointer_event_at`, so clicking a button in the running window actually changes `active_tool()`.
