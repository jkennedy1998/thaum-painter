# /home/j/Repos/thaum-painter/workers

## purpose
Own bounded background/async seams for thaum-painter, such as future autosave scheduling.

## owns
- background/async execution seams that must not block editing-session state

## does not own
- live editing session truth (owned by `domain/painter-session/`)
- persistence policy itself (owned by `domain/file/storage/`; a worker may trigger it, not own it)
- hosted/shared multiplayer presence stores — explicitly deferred; see `context/ascii-painter-feature-audit.md`'s "defer until core contracts are stable" list

## children-encapsulations
- none

## contents
- none

## dependencies
- none

## exposed interfaces
- none

## interface consumers
- future `orchestration/` boot flow

## artifacts
- none

## tests
- none

## data
- none

## notes
- kept empty; the old painter's hosted/shared session store is intentionally deferred, not assigned here yet.
