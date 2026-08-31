# painter-document encapsulation plan

## current-state
Painter-document has only a contract. With the stronger encapsulation split, painter-document should no longer be the owner of the saved file manifest or the renderer handoff. It should become the editing-facing document view over `domain/file/` state.

## target-encapsulation
- `/home/j/Repos/thaum-painter/domain/painter-document/`: edit

## artifacts
- example document-view notes or fixtures may be added later

## tests
- future editing-view validation tests should be added once implementation starts

## data
- none

## phases
### phase-1
- [ ] align contract notes with the new file and render-space split
- [ ] note the old painter document responsibilities that should stay outside this boundary

### phase-2
- [ ] define the editing-facing document view responsibilities
- [ ] define how it reads from or references file state
- [ ] define what live editing concerns belong here versus session

### phase-3
- [ ] define implementation-ready notes for session and operation work
- [ ] define what document conveniences must not become file ownership or render-space ownership

### phase-4
- [ ] validate the shape against future editing flows
- [ ] leave implementation-ready notes for session work

### phase-5
- [ ] final validation sweep
- [ ] tests/proofs as available
- [ ] git commit if approved
