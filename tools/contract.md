# /home/j/Repos/thaum-painter/tools

## purpose
Own reusable, low-level thaum-painter helpers that do not carry domain truth themselves.

## owns
- generic geometry/math helpers used across multiple `domain/` seams
- generic data-structure utilities with no painter-specific meaning on their own

## does not own
- saved file truth, editing-facing views, live session state, or pure painter operations (all owned under `domain/`)
- app-boot composition (owned by `orchestration/`)
- background/async execution (owned by `workers/`)

## children-encapsulations
- none

## contents
- none

## dependencies
- none

## exposed interfaces
- none

## interface consumers
- future `domain/` and `orchestration/` seams

## artifacts
- none

## tests
- none

## data
- none

## notes
- kept empty until a real cross-seam helper proves it does not belong inside one domain boundary; do not move domain-owned logic here just to make it "shared" prematurely.
