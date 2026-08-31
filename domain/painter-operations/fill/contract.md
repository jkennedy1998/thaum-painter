# /home/j/Repos/thaum-painter/domain/painter-operations/fill

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
- `/home/j/Repos/thaum-painter/domain/painter-operations/brush/`

## exposed interfaces
- none

## interface consumers
- painter-session commands
- tool-state

## artifacts
- none

## tests
- future fill tests

## data
- none
