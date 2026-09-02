# /home/j/Repos/thaum-painter/domain/modules/shared

## purpose
Own painter-only shared module helpers that don't belong in `thaum-renderer/domain/modules/shared/` because they're specific to painter's own module needs.

## owns
- painter-specific shared chrome or behavior reused across more than one of painter's own individual modules
- painter-owned indexed palette data shared by more than one painter module

## does not own
- generic gizmo/chrome/registry logic reusable by any consumer, owned by `thaum-renderer/domain/modules/shared/`
- any single individual module's content

## children-encapsulations
- none

## contents
- `contract.md`
  - shared contract
- `legacy_indexed_palette.rs`
  - painter's old 37-color indexed palette, reused by the indexed swatch picker and RGB block

## dependencies
- `/home/j/Repos/thaum-renderer/domain/modules/shared/`
- `/home/j/Repos/thaum-painter/domain/modules/`

## exposed interfaces
- `legacy_indexed_palette()`

## interface consumers
- `/home/j/Repos/thaum-painter/domain/modules/individuals/`

## artifacts
- none

## tests
- none yet

## data
- none

## notes
- empty as of this pass — expected to stay small or empty for a while, since most shared behavior should land in `thaum-renderer/domain/modules/shared/` where any consumer can use it.
