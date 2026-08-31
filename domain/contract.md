# /home/j/Repos/thaum-painter/domain

## purpose
Own the painter-specific semantic boundaries for authoring ASCII scenes without taking ownership of renderer internals.

## owns
- file semantics for saved painter assets and authoring-only metadata
- rendering semantics for app-side render state handoff into `thaum-renderer`
- painter session semantics
- painter editing operation semantics

## does not own
- renderer-owned matrix intake schema semantics
- boot-only orchestration
- low-level reusable helpers better placed in `tools/`

## children-encapsulations
- `file/`
  - default
- `rendering/`
  - default
- `painter-document/`
  - default
- `painter-session/`
  - default
- `painter-operations/`
  - default

## contents
- `contract.md`
  - domain contract

## dependencies
- `/home/j/Repos/thaum-painter/`
- `/home/j/Repos/thaum-renderer/`

## exposed interfaces
- none

## interface consumers
- future painter implementation surfaces

## artifacts
- none

## tests
- none

## data
- none

## notes
- start with headless boundaries first, then layer UI/modules later.
- the first pass should be more encapsulated than the old system: file ownership and render-space handoff should not hide inside one broad painter-document seam.
- prefer one clear renderer handoff seam under `domain/rendering/render-space/` rather than reviving a separate broad painter-render bridge.
