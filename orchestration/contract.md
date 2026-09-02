# /home/j/Repos/thaum-painter/orchestration

## purpose
Own the minimal boot/composition seam for standing up thaum-painter usage, wiring `domain/`, `tools/`, and `workers/` together without taking over their behavior.

## owns
- the app-boot composition sequence for thaum-painter
- wiring saved-file load, editing-session start, and renderer handoff boot into one flow
- boot-time intake of any consumer-provided runtime configuration

## does not own
- domain design truth (owned by `domain/`)
- low-level reusable helpers (owned by `tools/`)
- background/async execution (owned by `workers/`)
- renderer-internal boot, owned by `/home/j/Repos/thaum-renderer/orchestration/`
- UI module composition (deferred; see `context/roadmap.md`)

## children-encapsulations
- `entrypoint/`
  - default

## contents
- none

## dependencies
- `/home/j/Repos/thaum-painter/domain/`
- `/home/j/Repos/thaum-painter/tools/`
- `/home/j/Repos/thaum-painter/workers/`
- `/home/j/Repos/thaum-renderer/orchestration/boot/`

## exposed interfaces
- none

## interface consumers
- humans and operators running thaum-painter locally, via `entrypoint/`

## artifacts
- none

## tests
- none

## data
- none

## notes
- `entrypoint/` is the first real boot-composition surface here — a runnable proof app, not yet real painter-file loading. This parent stays a pure boundary; concrete boot code lives in the child, matching `/home/j/Repos/thaum-renderer/orchestration/`'s own boot/asset-root split.
