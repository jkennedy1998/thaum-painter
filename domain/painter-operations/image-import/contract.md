# /home/j/Repos/thaum-painter/domain/painter-operations/image-import

## purpose
Own image-to-ASCII conversion and related import-time raster transforms for painter authoring.

## owns
- image sampling into painter cells
- luminance or ramp mapping into glyph output
- color reduction policy for imported images
- scaling rules used during image import

## does not own
- OS clipboard transport details
- saved file manifest ownership
- renderer runtime behavior

## children-encapsulations
- none

## contents
- `contract.md`
  - image-import contract

## dependencies
- `/home/j/Repos/thaum-painter/domain/painter-operations/brush/`
- `/home/j/Repos/thaum-painter/domain/painter-session/clipboard/`

## exposed interfaces
- none

## interface consumers
- painter-session commands
- future import flows

## artifacts
- future image import fixtures

## tests
- future image import tests

## data
- none
