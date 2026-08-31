# /home/j/Repos/thaum-painter/domain/painter-session/history

## purpose
Own undo/redo history semantics for painter session changes.

## owns
- history stack rules
- grouped action semantics
- undo/redo snapshot or patch policy

## does not own
- command vocabulary
- pure tool operations
- file persistence

## children-encapsulations
- none

## contents
- `contract.md`
  - history contract

## dependencies
- `/home/j/Repos/thaum-painter/domain/painter-session/commands/`

## exposed interfaces
- none

## interface consumers
- painter-session

## artifacts
- future history fixtures

## tests
- future undo/redo tests

## data
- none
