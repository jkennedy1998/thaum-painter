# thaum-painter/domain/modules/individuals/hand-settings

## purpose
Own painter's compact hand-settings panel: the left/right paint-state editor for weight, edit/select masks, target mode, selection mode, and tool-property rows.

## owns
- the `HandSettingsModule` type
- the compact per-hand settings layout and hit-testing
- the two-zone row layout: always-shown standard hand rows (weight, select/edit masks, target, selection mode) on top, tool-specific rows below that only render when either equipped hand's tool declares them via `tool-state`'s per-tool property manifest
- direct edits into shared live painter `tool-state`
- direct edits into shared live painter `selection`
- row scrolling over the property rows via the shared renderer `ScrollState`, with the top three content rows reserved for the 3x3 hand-preview blocks (J 2026-09-10: tools/graphic/color text rows and the bottom left/right hand-color blocks were replaced by two 3x3 blocks — the ring is the hand's graphic in the hand's weight and color, the center is the hand letter in weight two and the hand's color)

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
- `thaum-painter/domain/painter-session/tool-state/`
- `thaum-painter/domain/painter-session/selection/`
- `thaum-renderer/domain/modules/shared/ui-palette/`
- `thaum-renderer/domain/modules/shared/panel-chrome/`

## exposed interfaces
- `HandSettingsModule::new(id, rect, tool_state, selection)` — build one compact settings panel bound to shared live tool-state and selection state

## interface consumers
- `thaum-painter/orchestration/entrypoint/`

## artifacts
- none

## tests
- inline `#[cfg(test)]` in `hand_settings_module.rs`
  - light
  - validates weight, select/edit masks, target, selection mode, brush/fill property toggles, plus tool-row hiding driven by the equipped tools (the fill match row was removed 2026-09-10 — the Select row is fill's comparison truth)
  - validates wheel behavior: over a number field it nudges that value by one; anywhere else it scrolls the row list so cropped tool rows stay reachable

## data
- none

## notes
- The panel crops rows that do not fit; the wheel scrolls the row window (shared `thaum-renderer` `ScrollState`) so tool rows tucked below the fold stay reachable. Wheel over a number field still nudges that value, matching the panel's original field-nudge semantic.
- The top three content rows are reserved for the hand-preview blocks; property rows start below them (`PropertyRows` `top_offset`), so scrolled matrix tokens never slide under them.
- Select and Edit rows carry whole-row tooltip copy from J (2026-09-10): Select = "lock a graphic, color, or weight channel per hand for selection oriented tool use. like fill sensing adjacent tiles"; Edit = "lock or unlock a graphic, color, or weight channel per hand for placement oriented tool use. like fill placing down the actual content".
- source-of-truth from J: left and right should each keep their own color/weight selection, with useful masking for deeper illustration work; channel locks were bloat and have been removed from the panel.
- source-of-truth from J: the panel hosts the never-changing standard property block on top and tool properties that change below; rows unused by both equipped tools stay hidden so the module feels thin while tools accumulate shared properties.
- this panel intentionally stays compact and state-bound; it is not a second toolbox.
