# painter-session encapsulation plan

## current-state
The old painter fused commands, history, selection, clipboard, and tool state into broad runtime behavior. This repo should sort those as separate session seams before implementation.

## target-encapsulation
- `/home/j/Repos/thaum-painter/domain/painter-session/`: edit

## artifacts
- future session fixtures

## tests
- future reducer, history, selection, and clipboard tests

## data
- none

## phases
### phase-1
- [ ] align contract notes with the split session responsibilities
- [ ] note the old fused runtime behavior that should not come back monolithically

### phase-2
- [ ] define commands, history, selection, clipboard, and tool-state child seams
- [ ] define the ownership boundaries between those child seams

### phase-3
- [ ] define implementation-ready notes for reducer and authoring workflows
- [ ] leave session ready to consume document and operation seams

### phase-4
- [ ] validate the shape against the old painter feature surface

### phase-5
- [ ] final validation sweep
- [ ] tests/proofs as available
- [ ] git commit if approved
