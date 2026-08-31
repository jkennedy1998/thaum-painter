# /home/j/Repos/thaum-painter/domain/painter-document/properties

## purpose
Own editing-facing property-block views over painter file state.

## owns
- property lookup helpers
- block-range lookup views
- document conveniences for raster, move, rotation, and related authoring properties

## does not own
- canonical stored property schema
- mutation commands
- renderer handoff

## children-encapsulations
- none

## contents
- `contract.md`
  - properties contract

## dependencies
- `/home/j/Repos/thaum-painter/domain/file/`

## exposed interfaces
- none

## interface consumers
- painter-session
- render-space

## artifacts
- none

## tests
- future property-view tests

## data
- none
