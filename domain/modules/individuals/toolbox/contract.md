# thaum-painter/domain/modules/individuals/toolbox

## purpose
Own painter's toolbox panel: the old-feeling left/right tool chooser rebuilt as a renderer `Module`.

## owns
- the `ToolboxModule` type and its tool-row presentation
- left-click assigns the left-hand tool
- right-click assigns the right-hand tool
- visible left/right/both assignment indicators per tool row
- the direct bridge from toolbox clicks into live painter `tool-state`

## does not own
- the renderer's `Module` contract or registry semantics, owned by `thaum-renderer/domain/modules/`
- pure brush/erase/fill behavior, owned by `domain/painter-operations/`
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
  - validates row hit-testing, left/right assignment, and draw indicators

## data
- none

## notes
- source-of-truth from J: the toolbox UX should follow the old painter habit where left and right mouse buttons choose the left-hand and right-hand tools directly.
- source-of-truth from J: tool behavior itself should stay encapsulated separately from the toolbox UI; the toolbox is selection UI, not the owner of brush/fill logic.
