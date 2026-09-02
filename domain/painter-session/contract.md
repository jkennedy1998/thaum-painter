# /home/j/Repos/thaum-painter/domain/painter-session

## purpose
Own the bounded editing/session semantics that mutate a painter document over time.

## owns
- active document session state
- command/reducer-style mutations over painter documents
- undo/redo-friendly change packet semantics
- active breath/frame selection and group/layer focus state

## does not own
- renderer core format ownership
- UI module rendering
- low-level raster algorithms better placed in painter-operations

## children-encapsulations
- `commands/`
  - default
- `history/`
  - default
- `selection/`
  - default
- `clipboard/`
  - default
- `tool-state/`
  - default
- `timeline-state/`
  - default

## contents
- `contract.md`
  - painter-session contract

## dependencies
- `/home/j/Repos/thaum-painter/domain/painter-document/`
- `/home/j/Repos/thaum-painter/domain/painter-operations/`

## exposed interfaces
- none

## interface consumers
- future painter app surfaces

## artifacts
- none

## tests
- none

## data
- none

## notes
- the old `painter_session_core.ts`, selection helpers, copy/paste helpers, and tool persistence should come back through smaller session seams rather than one fused runtime boundary.
