# encapsulation plan — modules/individuals/layers-panel

## pre-implementation-note
Edit under the binary-bars project (`plans/project-thaum-painter-binary-bars-plan.md`), consuming the `properties/pieces/` classifier. The interaction matrix is 6 interactions (left click, right click, left-drag, right-drag, double left, double right) × 4 pieces (single / left head / center / right head) × 2 cell types (empty / solid) = 48 branches. Locked truths: routed cleanly per branch, deliberately **not** one big calculation; blanks get heavily-similar-but-not-identical piece-based UX; `merge_blank_property_block` and the whole merge-direction UX die; empty-piece routing is where the future user-settable interpolative-mode UX lands (not built here).

## target-encapsulation
- `thaum-painter/domain/modules/individuals/layers-panel/`: edit

## artifacts
- none

## tests
- layers-panel interaction tests rewritten per piece × cell type; merge-blank cases removed.

## data
- none

## phases
### phase-1 — docs + contract + truth alignment
- [x] contract: owns the 48-branch piece × cell-type dispatch over property bars, consuming `properties/pieces/`; merge-blank action removed; does not own classification (consumed) or mutation (queued actions)
- [x] map the current size-keyed behavior (`resolve_property_drag_mode`, `blank_merge_direction`) to the new piece-keyed surface so no existing interaction silently changes meaning beyond the approved design

### phase-2 — piece-keyed interaction routing
- [x] replace `resolve_property_drag_mode` with piece × cell-type-keyed dispatch (48 clean branches)
- [x] remove `MergeBlankPropertyBlock` action and `blank_merge_direction` (already dead before this pass; the old `SplitPropertyBlock` action also died — double-left duplicates instead)
- [x] blanks: same piece-based UX as solids with the approved per-branch differences (e.g. empty pieces queue empty-appropriate actions; auto-key-off edit attempts on empties reject via timeline-state)
- [x] rewrite interaction tests per branch; delete merge-blank tests (and the split-on-body test)
- [x] verify no unplanned cross-encapsulation work

### phase-3 — validation + repo-rule sweep + git commit
- [x] run the encapsulation tests
- [x] full workspace test suite green (358 domain tests + entrypoint/workers)
- [x] git commit

## post-implementation-notes
- plan-finished: true
- encapsulation-git-commit: true
- implemented 2026-09-07: hit-testing classifies through `properties/pieces/` (`classify_bar_piece` + `cell_type_of`); the old `PropertyBlockHitMode` enum and `property_block_hit_mode` fn died. New actions `DuplicatePropertyBlock` + `MergeEmptyPropertyBlock` wired through `layers_runtime.rs` (`duplicate_property_block` + `duplicate_data_propagation_record` mirror the old split propagation; `merge_empty_property_block` handles the left-preferred seam). Scrub-on-blank-center removed per the matrix.
