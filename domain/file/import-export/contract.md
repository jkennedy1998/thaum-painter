# thaum-painter/domain/file/import-export

## purpose
Own supported import/export wrappers around the canonical file, including painter-to-game and painter-to-painter transfer packaging.

## owns
- supported external file wrappers
- export profile notes
- import/export policy around canonical file data
- boundaries for plain-text, JSON, or packaged transfer forms when supported

## does not own
- canonical manifest shape
- legacy migration internals
- renderer runtime behavior

## children-encapsulations
- none

## contents
- `contract.md`
  - import-export contract

## dependencies
- `thaum-painter/domain/file/manifest/`
- `thaum-painter/domain/rendering/render-space/`

## exposed interfaces
- none

## interface consumers
- future file menu flows
- future game/tool import flows

## artifacts
- future transfer samples

## tests
- future import/export coverage

## data
- none

## notes
- import/export should wrap or transform the canonical saved file represented by `domain/file/manifest/example-thaum-painter-file-v1.json`.
- renderer-facing export profiles may target handoff shapes equivalent to `domain/rendering/render-space/example-render-space-v1.json` without making that transient shape the canonical saved file.
