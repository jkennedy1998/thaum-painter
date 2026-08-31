# /home/j/Repos/thaum-painter/domain/painter-document/modules

## purpose
Own editing-facing module lookup, ordering, and placement views over saved painter file state.

## owns
- module lookup helpers
- module ordering views
- module visibility and lock-state document conveniences
- module placement (position on the shared board) editing-facing views
- module-to-intra-module-group membership views

## does not own
- canonical stored module schema
- module placement mutation history
- renderer composition or the module-to-cell-group mapping
- intra-module group content itself (owned by `groups/`)

## children-encapsulations
- none

## contents
- `contract.md`
  - modules contract

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
- future module-view tests

## data
- none

## notes
- a module is the top-level authored unit that becomes exactly one renderer `CellGroup`; see `/home/j/Repos/thaum-painter/context/module-concept-audit.md` for the design truths behind this split.
- `groups/` owns intra-module content; this seam owns the module wrapper around one or more groups.
- module placement here is the same global-board position that `domain/rendering/render-space/` copies onto the renderer cell-group; group placement inside a module stays local to the module and is a separate concern owned by `groups/`.
