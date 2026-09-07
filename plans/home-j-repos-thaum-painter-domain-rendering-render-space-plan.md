# encapsulation plan — rendering/render-space

## pre-implementation-note
Edit under the binary-bars project (`plans/project-thaum-painter-binary-bars-plan.md`). The two `resolve_gap_fill` match sites in `render_space.rs` exist for void breaths; voids stop existing. Interim behavior locked by J: playhead over an empty renders nothing; per-row custom interpolation is a later pass outside this project.

## target-encapsulation
- `thaum-painter/domain/rendering/render-space/`: edit

## artifacts
- none

## tests
- existing render-space tests updated for the empty-covers-breath case.

## data
- none

## phases
### phase-1 — docs + contract + truth alignment
- [ ] contract: interim empty behavior (render nothing) recorded; per-row interpolation noted as a future pass

### phase-2 — collapse the gap-fill call sites
- [ ] both `resolve_gap_fill` sites reduce to: covering bar empty → render nothing; solid → render as today
- [ ] drop the `interpolation/` dependency
- [ ] update tests
- [ ] verify no unplanned cross-encapsulation work

### phase-3 — validation + repo-rule sweep + git commit
- [ ] run the encapsulation tests
- [ ] encapsulation-checker / repo-rule verification
- [ ] git commit if approved

## post-implementation-notes
- plan-finished: false
- encapsulation-git-commit: false
