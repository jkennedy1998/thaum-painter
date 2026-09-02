# /home/j/Repos/thaum-painter/domain/painter-document/layers

## purpose
Own editing-facing layer lookup, ordering, and placement views over saved painter file state. A layer is the top-level authored unit and maps directly to exactly one renderer `CellGroup`.

## owns
- layer lookup helpers
- layer ordering views
- layer visibility and lock-state document conveniences
- layer placement (position on the shared board) editing-facing views

## does not own
- canonical stored layer schema
- layer mutation history
- renderer composition or the layer-to-cell-group mapping

## children-encapsulations
- none

## contents
- `contract.md`
  - layers contract

## dependencies
- `/home/j/Repos/thaum-painter/domain/file/`

## exposed interfaces
- none

## interface consumers
- painter-session
- rendering camera
- rendering render-space

## artifacts
- none

## tests
- future layer-view tests

## data
- none

## notes
- one saved layer maps directly to exactly one renderer `CellGroup`; there is no intermediate wrapper. See `context/module-concept-audit.md`'s "superseded" section for why the earlier module-wrapper shape was reversed.
- layer placement is the same global-board position that `domain/rendering/render-space/` copies onto the renderer cell-group.
- "layer" and "group" name the same authored unit; `layer` is the word going forward across UI, docs, and this seam. `CellGroup` stays the renderer's own name for its generic composited unit.
