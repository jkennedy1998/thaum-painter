# thaum-painter/orchestration

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
- renderer-internal boot, owned by `thaum-renderer/orchestration/`
- UI module composition (deferred; see `context/roadmap.md`)

## children-encapsulations
- `build-commands/`
  - default

## contents
- none

## dependencies
- `thaum-painter/domain/`
- `thaum-painter/tools/`
- `thaum-painter/workers/`
- `thaum-renderer/orchestration/boot/`

## exposed interfaces
- none

## interface consumers
- humans and operators running thaum-painter locally, via `build-commands/`

## artifacts
- none

## tests
- none

## data
- none

## notes
- `build-commands/` is the first real boot-composition surface here — the runnable app plus its two build commands (`build-all-environments`, `run-and-build-linux`) and the per-environment lanes under `orchestration/builds/`. This parent stays a pure boundary; concrete boot code lives in the child, matching `thaum-renderer/orchestration/`'s own boot/asset-root split.
