# /home/j/Repos/thaum-painter/domain/painter-tools/individuals/fill

## purpose
Own the fill tool's registration truth as one tool encapsulation.

## owns
- the `FillTool` registration type resolving fill's `ToolDescriptor` (id "fill", label, icon, `fill_diagonal` + `fill_match_channels` property rows, bucket hotkey action)
- fill's future session behavior home (behavior still lives in `painter-session/tool-state/` match arms)

## does not own
- the registry list
- flood fill semantics, owned by `painter-operations/fill/` (pure) and `painter-session/tool-state/` (session) until migration

## children-encapsulations
- none

## contents
- `contract.md`
  - fill tool contract
- `fill_tool.rs`
  - `FillTool` `RegisteredTool` impl + descriptor test

## dependencies
- the registry seam (`domain/painter-tools/`)

## exposed interfaces
- none

## interface consumers
- `domain/painter-tools/` registry

## artifacts
- none

## tests
- inline `#[cfg(test)]` in `fill_tool.rs`
  - light
  - validates fill's descriptor truth

## data
- none

## notes
- hotkey: `painter_select_bucket` on B, declared in the descriptor and drift-tested by `tai/`.
