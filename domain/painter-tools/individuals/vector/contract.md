# thaum-painter/domain/painter-tools/individuals/vector

## purpose
Register the one Vector tool: point/handle shape authoring across a construction session.

## owns
- Vector registration descriptor

## does not own
- vector construction-session behavior, owned by `painter-session/tool-state/`
- pointer dispatch and preview, owned by `painter-session/canvas-pointer/`
- vector geometry and rasterization, owned by `painter-operations/shapes/` `vector.rs`
- property panel rendering, owned by `modules/individuals/hand-settings/`

## children-encapsulations
- none

## contents
- `vector_tool.rs` — Vector registration implementation
- `contract.md` — this boundary

## dependencies
- `painter-tools/`: `RegisteredTool`, `ToolDescriptor`, `require_by_id`

## exposed interfaces
- none

## interface consumers
- `painter-tools/registry.rs`: `VectorTool::descriptor`

## artifacts
- none

## tests
- `vector_resolves_its_own_descriptor` — light

## data
- none

## notes
- The shared stroke/fill source rows (J 2026-11-09) and the brush size row (stroke width) ride the descriptor; the hotkey `H` is a low-priority placeholder J expects to revisit.
