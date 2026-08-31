# /home/j/Repos/thaum-painter/domain/file/migration

## purpose
Own migration and import-only compatibility from older painter save shapes into the new canonical file.

## owns
- legacy format detection
- upgrade rules into the new file manifest
- compatibility notes for old grid, voxel, and broad painter-document formats

## does not own
- the new canonical manifest itself
- normal save/load persistence
- renderer handoff mapping

## children-encapsulations
- none

## contents
- `contract.md`
  - migration contract

## dependencies
- `/home/j/Repos/thaum-painter/domain/file/manifest/`

## exposed interfaces
- none

## interface consumers
- future import flows

## artifacts
- future migration fixtures

## tests
- future migration coverage

## data
- none
