# thaum-painter/domain

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
- `modules/`
  - default
- `user-state/`
  - machine-local user session state: per-user prefs + persisted UI session, not profile saves
- `tai/`
  - painter tool-assisted input test seam consuming the renderer's TAI runner

## contents
- `contract.md`
  - domain contract

## dependencies
- `thaum-painter/`
- `thaum-renderer/`

## exposed interfaces
- none

## interface consumers
- future painter implementation surfaces

## artifacts
- none

## tests
- `tai/`'s inline test module
  - light
  - registry-driven: loads every registered painter TAI and validates parse + replay against painter bindings.

## data
- none

## notes
- start with headless boundaries first, then layer UI/modules later.
- `modules/` mirrors `thaum-renderer/domain/modules/`'s shape and holds painter-only panels (color picker, character picker, toolbar) built on the renderer's shared registry/gizmo logic.
- the first pass should be more encapsulated than the old system: file ownership and render-space handoff should not hide inside one broad painter-document seam.
- prefer one clear renderer handoff seam under `domain/rendering/render-space/` rather than reviving a separate broad painter-render bridge.
