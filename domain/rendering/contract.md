# thaum-painter/domain/rendering

## purpose
Own painter app rendering semantics that prepare and route app state into `thaum-renderer` without making app file/session concerns renderer-owned.

## owns
- app-side rendering boundaries
- render-state preparation rules for painter data
- the split between painter-owned state and renderer-facing handoff state

## does not own
- renderer internals
- saved file manifest ownership
- UI module behavior

## children-encapsulations
- `camera/`
  - default
- `render-space/`
  - default

## contents
- `contract.md`
  - rendering domain contract

## dependencies
- `thaum-painter/domain/file/`
- `thaum-renderer/`

## exposed interfaces
- none

## interface consumers
- future app boot and editor surfaces

## artifacts
- `thaum-painter/domain/rendering/render-space/example-render-space-v1.json`
  - first concrete renderer handoff example

## tests
- future rendering-boundary checks against `domain/rendering/render-space/example-render-space-v1.json`

## data
- none

## notes
- the current concrete renderer-handoff reference is `domain/rendering/render-space/example-render-space-v1.json`.
- rendering seams should keep consuming saved truth from `domain/file/file-schema/` rather than inventing parallel persisted render-state ownership.
