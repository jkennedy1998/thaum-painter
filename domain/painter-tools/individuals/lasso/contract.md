# thaum-painter/domain/painter-tools/individuals/lasso

## purpose
Own the lasso tool's registration truth as one tool encapsulation.

## owns
- the `LassoTool` registration type resolving lasso's `ToolDescriptor` (id "lasso", label, icon, no property rows yet, lasso hotkey action)
- lasso's future session behavior home (drag-path accumulation lives in `painter-session/tool-state/lasso_stroke`, application in `painter-session/tool-state` match seams)

## does not own
- the registry list
- polygon rasterization, owned by `painter-operations/shapes/lasso/` (pure)
- selection mode semantics, owned by `painter-session/selection/`

## children-encapsulations
- none

## contents
- `contract.md`
  - lasso tool contract
- `lasso_tool.rs`
  - `LassoTool` `RegisteredTool` impl + descriptor test

## dependencies
- the registry seam (`domain/painter-tools/`)

## exposed interfaces
- none

## interface consumers
- `domain/painter-tools/` registry

## artifacts
- none

## tests
- inline `#[cfg(test)]` in `lasso_tool.rs`
  - light
  - validates lasso's descriptor truth

## data
- none

## notes
- source-of-truth from J: lasso lets the user draw a bound by clicking with the hand that has lasso equipped, dragging the path, and releasing to close it; releasing fills the enclosed cells with the selected character.
- source-of-truth from J: the selected character may be an empty cell; the fill must flow through the standard hand-state seams (channel masks, blank-glyph preservation), not re-decide them. (Former lock mention superseded — channel locks removed 2026-09-03.)
- source-of-truth from J: lasso only edits within the selected tiles, exactly like the other image-editing tools.
- hotkey: `painter_select_lasso` on L, declared in the descriptor and drift-tested by `tai/`.
