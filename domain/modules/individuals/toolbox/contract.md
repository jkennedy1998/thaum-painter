# thaum-painter/domain/modules/individuals/toolbox

## purpose
Own painter's toolbox panel: the old-feeling left/right tool chooser rebuilt as a renderer `Module`.

## owns
- the `ToolboxModule` type and its tool-row presentation
- left-click assigns the left-hand tool
- right-click assigns the right-hand tool
- visible left/right/both assignment indicators per tool row
- the vertical scrollbar over the tool rows (shared `thaum-renderer/domain/modules/shared/scrollbar/` seam): spans exactly the tool-row band on the rightmost content column, drawn only when the tool list overflows the panel; wheel over the panel scrolls the rows (falling through when everything fits), thumb drag + track page-jump, and tool labels crop one column short of the bar when it shows (J's bar-aware rule from the graphic picker) — built for the tool list growing
- one tooltip row hotspot per visible tool: the row's content-span rect with title "<icon> <label>" and the registry descriptor's description, rendered by the shared tooltip seam
- the direct bridge from toolbox clicks into live painter `tool-state`

## does not own
- the renderer's `Module` contract or registry semantics, owned by `thaum-renderer/domain/modules/`
- pure brush/fill and clear-character behavior, owned by `domain/painter-operations/`
- app boot placement, owned by `orchestration/entrypoint/`

## children-encapsulations
- none

## contents
- `toolbox_module.rs`
  - `ToolboxModule`, `ToolDef`, and row hit-testing/drawing behavior

## dependencies
- `thaum-painter/domain/painter-session/tool-state/`
- `thaum-renderer/domain/modules/`
- `thaum-renderer/domain/modules/shared/ui-palette/`
- `thaum-renderer/domain/modules/shared/panel-chrome/`
- `thaum-renderer/domain/modules/shared/scrollbar/`
- `thaum-renderer/domain/modules/shared/scroll-state/`

## exposed interfaces
- `ToolboxModule::new(id, rect, tool_state, tool_defs)`
  - build one toolbox bound directly to live painter tool-state

## interface consumers
- `thaum-painter/orchestration/entrypoint/`

## artifacts
- none

## tests
- inline `#[cfg(test)]` in `toolbox_module.rs`
  - light
  - validates row hit-testing, left/right assignment, draw indicators, per-tool-row tooltip hotspots, and the scrollbar: full-height bar on overflow without any interaction, bar-aware label cropping, wheel scrolling with click-through to scrolled rows, wheel fall-through when everything fits, and thumb drag to both edges

## data
- none

## notes
- source-of-truth from J: the toolbox UX should follow the old painter habit where left and right mouse buttons choose the left-hand and right-hand tools directly.
- source-of-truth from J: tool behavior itself should stay encapsulated separately from the toolbox UI; the toolbox is selection UI, not the owner of brush/fill logic.
- source-of-truth from J: "make sure tools alsop has a scrollbar if it needs one / is too short to display all toolsl. we will be adding another tool shortly and its going to expand as well." — the toolbox scrolls its tool rows through the shared scrollbar seam whenever the panel is too short, ready for the tool list to grow.
