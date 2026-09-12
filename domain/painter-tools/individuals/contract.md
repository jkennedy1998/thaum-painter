# thaum-painter/domain/painter-tools/individuals

## purpose
Host one folder per registered painter tool, each owning that tool's registration truth via the `RegisteredTool` seam.

## owns
- one folder per tool: `brush/`, `fill/`, `lasso/`, `text/`, `picker/`, `stamp/`, `move_tool/`
- each folder's descriptor resolution and its contract

## does not own
- the registry list itself, owned by the parent `painter-tools/registry.rs`
- tool session behavior, still owned by `painter-session/tool-state/` until per-tool behavior migration
- pure painting operations, owned by `painter-operations/`

## children-encapsulations
- `brush/`
  - default
- `fill/`
  - default
- `lasso/`
  - default
- `text/`
  - default
- `picker/`
  - default
- `stamp/`
  - default

## contents
- `contract.md`
  - individuals contract
- `brush/brush_tool.rs`, `fill/fill_tool.rs`, `lasso/lasso_tool.rs`, `text/text_tool.rs`, `picker/picker_tool.rs`, `stamp/stamp_tool.rs`, `move_tool/move_tool.rs`
  - per-tool `RegisteredTool` impls

## dependencies
- the parent registry seam (`domain/painter-tools/`)

## exposed interfaces
- none

## interface consumers
- `domain/painter-tools/` registry drift tests

## artifacts
- none

## tests
- inline `#[cfg(test)]` per tool file
  - light
  - validates each tool resolves its own descriptor with correct registration truth

## data
- none

## notes
- new tool folders join here; the registry line plus the `PaintTool` arm are the only other touches.
- the clear character is selected in the graphics picker and is applied by Brush, Fill, Lasso, and future ordinary paint tools.
