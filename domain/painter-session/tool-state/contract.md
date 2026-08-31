# /home/j/Repos/thaum-painter/domain/painter-session/tool-state

## purpose
Own live tool settings and active authoring-hand state for the painter session.

## owns
- active tool selections
- left/right hand brush state
- edit-channel masks
- text-entry and paste option state that belongs to the live session

## does not own
- persistent file manifest ownership
- pure tool operations
- history ownership

## children-encapsulations
- none

## contents
- `contract.md`
  - tool-state contract

## dependencies
- `/home/j/Repos/thaum-painter/domain/painter-operations/`
- `/home/j/Repos/thaum-painter/domain/file/storage/`

## exposed interfaces
- none

## interface consumers
- painter-session

## artifacts
- none

## tests
- future tool-state tests

## data
- none
