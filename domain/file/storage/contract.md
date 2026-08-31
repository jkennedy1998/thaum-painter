# /home/j/Repos/thaum-painter/domain/file/storage

## purpose
Own persistence surfaces for saving, loading, autosaving, and locating thaum-painter files.

## owns
- local save/load policy
- autosave ownership
- file-path and storage-target conventions
- stored non-document session carryover that truly belongs to persistence

## does not own
- canonical file manifest shape
- renderer handoff
- live undo/redo state

## children-encapsulations
- none

## contents
- `contract.md`
  - storage contract

## dependencies
- `/home/j/Repos/thaum-painter/domain/file/manifest/`

## exposed interfaces
- none

## interface consumers
- future app boot
- future file menus

## artifacts
- future autosave artifacts and file fixtures

## tests
- future save/load tests

## data
- none
