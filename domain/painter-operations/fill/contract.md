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
- source-of-truth from J (2026-09-10): flood comparison honors a `FillChannelMask` — unlocked channels are ignored when matching neighbors, so adjacent cells sharing color and weight but differing in graphic flood together when the graphic channel is unlocked. The painter feeds the hand's Select row into this mask.
- future fill tests

## data
- none
