# /home/j/Repos/thaum-painter/domain/painter-session/selection

## purpose
Own live selection state for plane and world-oriented authoring flows.

## owns
- plane selection state
- world selection state
- selection mode semantics like replace/add/subtract/intersect
- selection bounds and selection-presence session truth

## does not own
- clipboard payloads
- pure shape rasterization
- saved file manifest ownership

## children-encapsulations
- none

## contents
- `contract.md`
  - selection contract

## dependencies
- `/home/j/Repos/thaum-painter/domain/painter-document/`
- `/home/j/Repos/thaum-painter/domain/painter-operations/shapes/`

## exposed interfaces
- none

## interface consumers
- painter-session
- rendering camera

## artifacts
- none

## tests
- future selection tests

## data
- none
