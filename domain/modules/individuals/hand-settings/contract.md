# /home/j/Repos/thaum-painter/domain/modules/individuals/hand-settings

## purpose
Own painter's compact hand-settings panel: the left/right paint-state editor for weight, edit/select masks, target mode, selection mode, and tool-property rows.

## owns
- the `HandSettingsModule` type
- the compact per-hand settings layout and hit-testing
- the two-zone row layout: always-shown standard hand rows (weight, select/edit masks, target, selection mode) on top, tool-specific rows below that only render when either equipped hand's tool declares them via `tool-state`'s per-tool property manifest
- direct edits into shared live painter `tool-state`
- direct edits into shared live painter `selection`

## does not own
- actual paint operation semantics, owned by `domain/painter-session/tool-state/` and `domain/painter-operations/`
- generic renderer module seams or panel chrome, owned by `thaum-renderer/domain/modules/`
- boot placement in the app window, owned by `orchestration/entrypoint/`

## children-encapsulations
- none

## contents
- `hand_settings_module.rs`
  - module UI for left/right hand settings

## dependencies
- `/home/j/Repos/thaum-painter/domain/painter-session/tool-state/`
- `/home/j/Repos/thaum-painter/domain/painter-session/selection/`
- `/home/j/Repos/thaum-renderer/domain/modules/shared/ui-palette/`
- `/home/j/Repos/thaum-renderer/domain/modules/shared/panel-chrome/`

## exposed interfaces
- `HandSettingsModule::new(id, rect, tool_state, selection)` — build one compact settings panel bound to shared live tool-state and selection state

## interface consumers
- `/home/j/Repos/thaum-painter/orchestration/entrypoint/`

## artifacts
- none

## tests
- inline `#[cfg(test)]` in `hand_settings_module.rs`
  - light
  - validates weight, select/edit masks, target, selection mode, brush/fill property and match toggles, plus tool-row hiding driven by the equipped tools

## data
- none

## notes
- source-of-truth from J: left and right should each keep their own color/weight selection, with useful masking for deeper illustration work; channel locks were bloat and have been removed from the panel.
- source-of-truth from J: the panel hosts the never-changing standard property block on top and tool properties that change below; rows unused by both equipped tools stay hidden so the module feels thin while tools accumulate shared properties.
- this panel intentionally stays compact and state-bound; it is not a second toolbox.
