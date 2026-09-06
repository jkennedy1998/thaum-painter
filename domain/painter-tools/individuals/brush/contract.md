# thaum-painter/domain/painter-tools/individuals/brush

## purpose
Own the brush tool's registration truth as one tool encapsulation.

## owns
- the `BrushTool` registration type resolving brush's `ToolDescriptor` (id "brush", label, icon, brush_size property row, pencil hotkey action)
- brush's future session behavior home (behavior still lives in `painter-session/tool-state/` match arms)

## does not own
- the registry list
- brush stroke/paint semantics, owned by `painter-operations/brush/` (pure) and `painter-session/tool-state/` (session) until migration

## children-encapsulations
- none

## contents
- `contract.md`
  - brush tool contract
- `brush_tool.rs`
  - `BrushTool` `RegisteredTool` impl + descriptor test

## dependencies
- the registry seam (`domain/painter-tools/`)

## exposed interfaces
- none

## interface consumers
- `domain/painter-tools/` registry

## artifacts
- none

## tests
- inline `#[cfg(test)]` in `brush_tool.rs`
  - light
  - validates brush's descriptor truth

## data
- none

## notes
- hotkey: `painter_select_pencil` on P, declared in the descriptor and drift-tested by `tai/`.
