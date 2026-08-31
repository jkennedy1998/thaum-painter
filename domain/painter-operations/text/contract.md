# /home/j/Repos/thaum-painter/domain/painter-operations/text

## purpose
Own text-entry and text-stamping semantics for painter authoring.

## owns
- text cell building rules
- spacing, lead, and newline stepping semantics
- text paste behaviors that are still pure operations

## does not own
- clipboard transport
- session cursor ownership
- history

## children-encapsulations
- none

## contents
- `contract.md`
  - text contract

## dependencies
- `/home/j/Repos/thaum-painter/domain/painter-operations/brush/`
- `/home/j/Repos/thaum-painter/domain/painter-session/clipboard/`

## exposed interfaces
- none

## interface consumers
- painter-session commands
- tool-state

## artifacts
- none

## tests
- future text operation tests

## data
- none
