# render-space encapsulation plan

## current-state
The first pass used a broad bridge idea. This repo now needs a dedicated `domain/rendering/render-space/` seam for how file or live app state sends data to `thaum-renderer`.

## target-encapsulation
- `thaum-painter/domain/rendering/render-space/`: add

## artifacts
- future sample render-space payloads and fixtures

## tests
- future translation and mapping tests

## data
- none

## phases
### phase-1
- [x] align contract notes with render-space ownership of renderer handoff
- [x] note the fields that should stay in file instead of leaking into this seam

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
- [x] final validation sweep
- [x] tests/proofs as available
- [ ] git commit if approved

## post-implementation-notes
- 26-08-31: implemented `render_space.rs` (`build_render_space`, `build_composition`, `build_module_cell_group`, breath-driven raster-segment/move-block resolution) against real `thaum-renderer-domain` types rather than the v1 JSON handoff shape, matching the in-process live-preview architecture decision. Compiler-verified: `cargo test -p thaum-painter-domain` passes, 18 tests total (11 manifest + 7 render-space). Hand-traced and confirmed the breath-4 output matches `example-render-space-v1.json` exactly.
- camera mapping was deliberately left out of this seam: `build_render_space` takes an already-resolved `thaum_renderer_domain::Camera` as input. `example-render-space-v1.json`'s `camera` block shape doesn't match the renderer's real `Camera` struct — see `contract.md`'s "known gap" note. `domain/rendering/camera/` needs to land before that mapping can be written for real.
- 26-08-31: the module-wrapped compositing shape (`build_module_cell_group`, one `CellGroup` per module) is reversed — see `context/module-concept-audit.md`'s "superseded" section. Target shape is now one `CellGroup` per authored group, direct 1:1, no compositing pass; renderer's own `overlap-policy`/`pass-order` handle inter-group overlap. Contract updated ahead of code; `render_space.rs` still implements the pre-reversal shape and needs a matching implementation pass before camera work lands on top of it.
- plan-finished: false
