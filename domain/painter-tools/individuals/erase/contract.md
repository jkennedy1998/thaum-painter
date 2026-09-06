# thaum-painter/domain/painter-tools/individuals/erase

## purpose
Own the erase tool's registration truth as one tool encapsulation.

## owns
- the `EraseTool` registration type resolving erase's `ToolDescriptor` (id "erase", label, icon, brush_size property row, no hotkey yet)
- erase's future session behavior home (behavior still lives in `painter-session/tool-state/` match arms)

## does not own
- the registry list
- erase semantics, owned by `painter-operations/brush/` (pure) and `painter-session/tool-state/` (session) until migration

## children-encapsulations
- none

## contents
- `contract.md`
  - erase tool contract
- `erase_tool.rs`
  - `EraseTool` `RegisteredTool` impl + descriptor test

## dependencies
- the registry seam (`domain/painter-tools/`)

## exposed interfaces
- none

## interface consumers
- `domain/painter-tools/` registry

## artifacts
- none

## tests
- inline `#[cfg(test)]` in `erase_tool.rs`
  - light
  - validates erase's descriptor truth

## data
- none

## notes
- no hotkey today; add one by declaring `select_action` + `hotkey` on the descriptor and binding it in `tai/`.
