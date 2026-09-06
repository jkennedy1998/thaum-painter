# thaum-painter/domain/painter-session/commands

## purpose
Own the command vocabulary for mutating painter session and document state.

## owns
- reducer command shapes
- command validation notes
- command grouping boundaries between file, document, selection, and tool state mutations

## does not own
- pure raster operations
- undo history storage
- renderer handoff

## children-encapsulations
- none

## contents
- `contract.md`
  - commands contract

## dependencies
- `thaum-painter/domain/painter-document/`
- `thaum-painter/domain/painter-operations/`

## exposed interfaces
- none

## interface consumers
- painter-session

## artifacts
- none

## tests
- future command-shape tests

## data
- none
