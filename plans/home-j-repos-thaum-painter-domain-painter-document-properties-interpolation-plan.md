# encapsulation plan — properties/interpolation (delete)

## pre-implementation-note
Deletion under the binary-bars project (`plans/project-thaum-painter-binary-bars-plan.md`). `GapFill`, `surrounding_items`, and `resolve_gap_fill` existed only to handle void breaths; voids stop existing under binary tiling. Real per-row interpolation (move lerp vs raster, user-settable empty mode) is a later, fresh pass — not this plan.

## target-encapsulation
- `thaum-painter/domain/painter-document/properties/interpolation/`: delete

## artifacts
- none

## tests
- removed with the encapsulation.

## data
- none

## phases
### phase-1 — docs + contract + truth alignment
- [ ] confirm no remaining consumers beyond `render-space/` (collapse lands in the render-space plan)
- [ ] remove the `interpolation/` child declaration from the `properties/` contract
- [ ] note in the parent contract that per-row interpolation returns later as a fresh seam

### phase-2 — deletion
- [ ] delete `interpolation/` (contract.md + interpolation.rs)
- [ ] confirm the crate compiles with render-space call sites already collapsed
- [ ] verify no unplanned cross-encapsulation work

### phase-3 — validation + repo-rule sweep + git commit
- [ ] run the touched-crate tests
- [ ] encapsulation-checker / repo-rule verification
- [ ] git commit if approved

## post-implementation-notes
- plan-finished: false
- encapsulation-git-commit: false
