# encapsulation plan — file/storage

## pre-implementation-note
Edit under the binary-bars project (`plans/project-thaum-painter-binary-bars-plan.md`). Truth: the tiling invariant lives on every edit seam — freed windows become blank blocks, coverage never breaks. `merge_blank_property_block` dies (its job was re-creating voids). Schema breaks with no migration: no importer, no long-term migration support; old files fail into the typed rejection, handled case by case by J.

Ownership split (2026-09-07, J-approved, see `plans/project-thaum-painter-file-boundary-cleanup-and-schema-gate-plan.md`): the unsupported-load rejection MECHANISM (typed `UnsupportedFile` error + clean open-flow rejection) is operator-2's; this plan only bumps the version constant — `SHARED_DOCUMENT_SCHEMA_VERSION`, or `FILE_SCHEMA_VERSION` after operator-2's rename — and re-tiles the edit seams. Operator-2's load-time gate must land before or with the bump so old files reject cleanly the moment v2 exists.

## target-encapsulation
- `thaum-painter/domain/file/storage/`: edit

## artifacts
- none

## tests
- inline `#[cfg(test)]` in `storage.rs` — every edit seam leaves tracks fully tiled; old-schema files rejected cleanly.

## data
- none

## phases
### phase-1 — docs + contract + truth alignment
- [ ] contract: tiling invariant as owned truth; merge-blank mutation removed from exposed surface; schema-break + no-migration policy recorded
- [ ] list every edit seam that must re-tile: `set_property_block_timing*`, split, `blank_property_block`, destructive timing, layer resize, window change

### phase-2 — tiling edit seams + schema break
- [ ] each listed edit seam re-tiles: freed windows become blank blocks
- [ ] delete `merge_blank_property_block`
- [ ] bump the schema version constant (`SHARED_DOCUMENT_SCHEMA_VERSION`, or `FILE_SCHEMA_VERSION` post-rename) — old files then fail into operator-2's typed gate; no load-path changes here
- [ ] tests: move/trim/split/blank/destructive/resize/window each leave the track fully tiled
- [ ] verify no unplanned cross-encapsulation work

### phase-3 — validation + repo-rule sweep + git commit
- [ ] run the encapsulation tests
- [ ] encapsulation-checker / repo-rule verification
- [ ] git commit if approved

## post-implementation-notes
- plan-finished: false
- encapsulation-git-commit: false
