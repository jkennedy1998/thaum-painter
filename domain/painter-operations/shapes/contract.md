# /home/j/Repos/thaum-painter/domain/painter-operations/shapes

## purpose
Own pure line, rectangle, lasso, and later 3d shape rasterization helpers for painter authoring.

## owns
- line rasterization usage
- rect and polygon shape point derivation
- future 3d primitive raster op boundaries for box, sphere, cylinder, and cone tools

## does not own
- selection session state
- history
- renderer composition

## children-encapsulations
- none

## contents
- `contract.md`
  - shapes contract

## dependencies
- `/home/j/Repos/thaum-painter/domain/painter-operations/brush/`

## exposed interfaces
- none

## interface consumers
- painter-session selection
- painter-session commands

## artifacts
- none

## tests
- future shape raster tests

## data
- none
