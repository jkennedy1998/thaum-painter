# encapsulation plan — properties

## pre-implementation-note
Edit under the binary-bars project (`plans/project-thaum-painter-binary-bars-plan.md`). Truth: bar placement becomes binary — no gaps, no overlaps, ever. `clamped_breath_span`'s gap-allowing and unwedge paths die (moves resolve through `pushed_breath_span` ripple and swap); the destructive path reduces to "victims become blanks" so coverage never breaks. New child `pieces/` carries piece × cell-type classification.

## target-encapsulation
- `thaum-painter/domain/painter-document/properties/`: edit

## artifacts
- none

## tests
- inline `#[cfg(test)]` in `properties.rs` — updated for tiling truth, ripple/swap, clamp removal.

## data
- none

## phases
### phase-1 — docs + contract + truth alignment
- [ ] contract: owns binary tiling semantics (tracks tile the full layer span edge to edge, two cell types only); `pieces/` child declared
- [ ] note consumers of the removed clamp seam (storage edit seams) for the boundary handoff
- [ ] confirm `span_end_breath` / `breath_in_span` / `pushed_breath_span` coverage stays

### phase-2 — remove the clamp seam
- [ ] delete `clamped_breath_span` and its unwedge/gap tests
- [ ] destructive span resolution: victims become blanks, coverage preserved
- [ ] update tests for ripple + swap as the only move resolutions
- [ ] verify no unplanned cross-encapsulation work

### phase-3 — validation + repo-rule sweep + git commit
- [ ] run the encapsulation tests
- [ ] encapsulation-checker / repo-rule verification
- [ ] git commit if approved

## post-implementation-notes
- plan-finished: false
- encapsulation-git-commit: false
