# /home/j/Repos/thaum-painter/domain/painter-session/clipboard

## purpose
Own live clipboard and copy-buffer semantics for painter session workflows.

## owns
- in-app copy-buffer state
- copy/paste payload shape used during authoring
- clipboard mode notes for plain text, rich payload, and world-aware selections
- session-facing paste transform intent such as anchor, angle mode, or scale choices when that state is live rather than stored

## does not own
- OS clipboard transport details
- selection ownership
- canonical import/export ownership

## children-encapsulations
- none

## contents
- `contract.md`
  - clipboard contract

## dependencies
- `/home/j/Repos/thaum-painter/domain/painter-session/selection/`
- `/home/j/Repos/thaum-painter/domain/painter-document/`

## exposed interfaces
- none

## interface consumers
- painter-session
- painter-operations/text

## artifacts
- future clipboard payload fixtures

## tests
- future clipboard tests

## data
- none

## notes
- keep the live clipboard state separate from payload codecs and separate again from pure paste transforms if this seam starts growing too wide.
- world copy payloads and rich encoded payloads belong here conceptually, but long-term file transfer formats should stay under `domain/file/import-export/`.
