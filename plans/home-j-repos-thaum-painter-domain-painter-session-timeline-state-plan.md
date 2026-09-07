# encapsulation plan — painter-session/timeline-state

## pre-implementation-note
Edit under the binary-bars project (`plans/project-thaum-painter-binary-bars-plan.md`). Truth locked by J: auto-key off + playhead over an **empty** → edit rejected, because the user cannot submit a new key and is not looking at a key. Today `resolve_editable_breath` only checks whether any bar covers the breath; under binary tiling blanks are bars too, so the rule must become cell-type aware.

## target-encapsulation
- `thaum-painter/domain/painter-session/timeline-state/`: edit

## artifacts
- none

## tests
- inline `#[cfg(test)]` in `timeline_state.rs` — auto-key off over empty rejects; over solid allows; auto-key on allows both.

## data
- none

## phases
### phase-1 — docs + contract + truth alignment
- [ ] contract: owns the empty-rejection rule explicitly (auto-key off + empty → reject), sourcing cell type from `properties/pieces/`

### phase-2 — cell-type-aware editable-breath rule
- [ ] `resolve_editable_breath` consumes cell type and rejects empties when auto-key is off
- [ ] tests for all four combinations (auto-key on/off × empty/solid)
- [ ] verify no unplanned cross-encapsulation work

### phase-3 — validation + repo-rule sweep + git commit
- [ ] run the encapsulation tests
- [ ] encapsulation-checker / repo-rule verification
- [ ] git commit if approved

## post-implementation-notes
- plan-finished: false
- encapsulation-git-commit: false
