# thaum-painter/domain/painter-operations/fill

## purpose
Own flood-fill and region replacement semantics for painter authoring.

## owns
- fill traversal rules
- equality rules for candidate matching
- depth or adjacency options used by fill behavior

## does not own
- selection ownership
- history
- file persistence

## children-encapsulations
- none

## contents
- `contract.md`
  - fill contract

## dependencies
- `thaum-painter/domain/painter-operations/brush/`

## exposed interfaces
- none

## interface consumers
- painter-session commands
- tool-state

## artifacts
- none

## tests
- source-of-truth from J (unified empty-cell rule): flood matching treats authored blanks (space-glyph cells) as `None` on both sides, so a stored blank never splits a flood region from truly empty space.
- future fill tests

## data
- none
