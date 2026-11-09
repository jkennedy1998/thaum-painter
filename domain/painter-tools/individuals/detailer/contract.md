# thaum-painter/domain/painter-tools/individuals/detailer

## purpose
Register the one Detailer tool and its per-hand add/erase property.

## owns
- Detailer registration descriptor

## does not own
- detail-stroke session behavior, owned by `painter-session/tool-state/`
- property panel rendering, owned by `modules/individuals/hand-settings/`

## children-encapsulations
- none

## contents
- `detailer_tool.rs` — Detailer registration implementation
- `contract.md` — this boundary

## dependencies
- `painter-tools/`: `RegisteredTool`, `ToolDescriptor`, `require_by_id`

## exposed interfaces
- none

## interface consumers
- `painter-tools/registry.rs`: `DetailerTool::descriptor`

## artifacts
- none

## tests
- `detailer_resolves_its_own_descriptor` — light

## data
- none

## notes
- Add and erase are one per-hand property, never separate registered tools.
