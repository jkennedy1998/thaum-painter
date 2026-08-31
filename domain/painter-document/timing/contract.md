# /home/j/Repos/thaum-painter/domain/painter-document/timing

## purpose
Own editing-facing timing and breath playback views over saved painter file state.

## owns
- document timing lookup helpers
- group breath-span and playback views
- editor-friendly timing normalization for authoring flows

## does not own
- canonical stored timing schema
- clock workers
- renderer data-lane execution

## children-encapsulations
- none

## contents
- `contract.md`
  - timing contract

## dependencies
- `/home/j/Repos/thaum-painter/domain/file/`
- `/home/j/Repos/thaum-renderer/domain/data-lanes/`

## exposed interfaces
- none

## interface consumers
- painter-session
- render-space

## artifacts
- none

## tests
- future timing-view tests

## data
- none

## notes
- keep this seam editing-facing: saved time asset structure belongs in `domain/file/manifest/`, while render-facing timing/data-lane mapping belongs in `domain/rendering/render-space/`.
- if timing grows, split only after timeline editing and authored time-asset views prove they have different consumers or test surfaces.
