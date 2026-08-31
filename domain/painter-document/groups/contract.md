# /home/j/Repos/thaum-painter/domain/painter-document/groups

## purpose
Own editing-facing group lookup and ordering semantics for the intra-module content stored inside one module of saved painter file state.

## owns
- group lookup helpers scoped to one owning module
- group ordering views within a module
- group visibility and lock-state document conveniences
- group-local (module-relative) editing projections that do not become saved-file ownership

## does not own
- canonical stored group schema
- group mutation history
- renderer composition
- module identity, module ordering, or module-level (global-board) placement, owned by `modules/`

## children-encapsulations
- none

## contents
- `contract.md`
  - groups contract

## dependencies
- `/home/j/Repos/thaum-painter/domain/file/`

## exposed interfaces
- none

## interface consumers
- painter-session
- rendering camera
- painter-document modules

## artifacts
- none

## tests
- future group-view tests

## data
- none

## notes
- group placement is stored as `local_placement` and is relative to the owning module's local coordinate space, not the shared board; see `context/module-concept-audit.md`.
