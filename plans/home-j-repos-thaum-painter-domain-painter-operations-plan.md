# painter-operations encapsulation plan

## current-state
The old painter had a wide pure-tool surface in `tools.ts`, `image_import.ts`, and related helpers. This repo should split those headless behaviors into smaller operation seams before implementation.

## target-encapsulation
- `/home/j/Repos/thaum-painter/domain/painter-operations/`: edit

## artifacts
- future operation fixtures

## tests
- future brush, shape, fill, text, and image-import tests

## data
- none

## phases
### phase-1
- [ ] align contract notes with the pure operation split
- [ ] note the old helper files feeding these seams

### phase-2
- [ ] define brush, shapes, fill, text, and image-import child seams
- [ ] define ownership boundaries between pure operations and session state

### phase-3
- [ ] define implementation-ready notes for geometry and edit behavior
- [ ] leave operations ready for piecewise rebuild

### phase-4
- [ ] validate the shape against the old painter feature surface

### phase-5
- [ ] final validation sweep
- [ ] tests/proofs as available
- [ ] git commit if approved
