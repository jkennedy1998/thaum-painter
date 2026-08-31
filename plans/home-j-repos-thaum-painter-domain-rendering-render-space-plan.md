# render-space encapsulation plan

## current-state
The first pass used a broad bridge idea. This repo now needs a dedicated `domain/rendering/render-space/` seam for how file or live app state sends data to `thaum-renderer`.

## target-encapsulation
- `/home/j/Repos/thaum-painter/domain/rendering/render-space/`: add

## artifacts
- future sample render-space payloads and fixtures

## tests
- future translation and mapping tests

## data
- none

## phases
### phase-1
- [ ] align contract notes with render-space ownership of renderer handoff
- [ ] note the fields that should stay in file instead of leaking into this seam

### phase-2
- [x] define direct mappings from file state into renderer-facing camera/group/cell/data-lane state
- [x] define derived render-only state needed for clean handoff
- [x] define ignored painter-only fields

### phase-3
- [x] define implementation-ready handoff examples for future boot integration
- [x] leave the seam ready for render proof work inside thaum-painter

### phase-4
- [x] validate mapping notes against painter-to-game and painter-preview needs

### phase-5
- [ ] final validation sweep
- [ ] tests/proofs as available
- [ ] git commit if approved
