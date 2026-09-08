# thaum-painter/domain/modules/shared

## purpose
Own painter-only shared module helpers that don't belong in `thaum-renderer/domain/modules/shared/` because they're specific to painter's own module needs.

## owns
- painter-specific shared chrome or behavior reused across more than one of painter's own individual modules

## does not own
- generic gizmo/chrome/registry logic reusable by any consumer, owned by `thaum-renderer/domain/modules/shared/`
- any single individual module's content

## children-encapsulations
- none

## contents
- `contract.md`
  - shared contract

## dependencies
- `thaum-renderer/domain/modules/shared/`
- `thaum-painter/domain/modules/`

## exposed interfaces
- `legacy_indexed_palette()`

## interface consumers
- `thaum-painter/domain/modules/individuals/`

## artifacts
- none

## tests
- none yet

## data
- none

## notes
- empty as of this pass — expected to stay small or empty for a while, since most shared behavior should land in `thaum-renderer/domain/modules/shared/` where any consumer can use it.
