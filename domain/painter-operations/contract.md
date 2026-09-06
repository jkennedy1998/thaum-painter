# thaum-painter/domain/painter-operations

## purpose
Own pure painter editing operations that can run headlessly over painter data.

## owns
- draw/erase/line/rect/fill/text stamp semantics
- import-time raster transforms that are not renderer-owned
- operation-level cell comparison helpers for painter edits

## does not own
- session history policy
- renderer composition
- UI pointer choreography

## children-encapsulations
- `brush/`
  - default
- `shapes/`
  - default
- `fill/`
  - default
- `text/`
  - default
- `image-import/`
  - default

## contents
- `contract.md`
  - painter-operations contract

## dependencies
- `thaum-painter/domain/painter-document/`

## exposed interfaces
- none

## interface consumers
- future painter session/runtime
- future importer/exporter seams

## artifacts
- none

## tests
- none

## data
- none

## notes
- the old `tools.ts`, `image_import.ts`, and geometry-heavy helpers should be rebuilt as smaller pure operation seams.
