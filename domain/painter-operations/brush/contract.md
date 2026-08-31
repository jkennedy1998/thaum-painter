# /home/j/Repos/thaum-painter/domain/painter-operations/brush

## purpose
Own direct brush-style cell application and erase semantics.

## owns
- single-cell draw semantics
- erase semantics
- brush-to-cell mapping rules including color, weight, and renderable appearance payloads

## does not own
- history
- selection ownership
- renderer handoff

## children-encapsulations
- none

## contents
- `contract.md`
  - brush contract

## dependencies
- `/home/j/Repos/thaum-painter/domain/painter-document/`

## exposed interfaces
- none

## interface consumers
- painter-session commands
- shape/text/fill operations

## artifacts
- none

## tests
- future brush operation tests

## data
- none
