# project plan — binary bars

Written 2026-09-07. Source-of-truth: `context/bars-binary-design-truth.md` (approved by J this morning).

## current-state
Property tracks currently have three states per channel breath: content bar, blank bar, and void (no block). Voids prop up three helper surfaces (`GapFill` family in `properties/interpolation/`, the gap-allowing/unwedge paths of `clamped_breath_span`, and the merge-blank UX in `layers-panel`). The approved design makes channels binary: every property track tiles its full layer span with empty/solid bars, UX routes by bar piece (single / left head / center / right head) × cell type (empty / solid), and voids stop existing. File schema breaks with no migration.

## affected-encapsulations
- `thaum-painter/domain/painter-document/properties/pieces/`: new
- `thaum-painter/domain/painter-document/properties/`: edit
- `thaum-painter/domain/painter-document/properties/interpolation/`: delete
- `thaum-painter/domain/file/storage/`: edit
- `thaum-painter/domain/painter-session/timeline-state/`: edit
- `thaum-painter/domain/rendering/render-space/`: edit
- `thaum-painter/domain/modules/individuals/layers-panel/`: edit

## encapsulation-plans
- `thaum-painter/domain/painter-document/properties/pieces/`: `plans/home-j-repos-thaum-painter-domain-painter-document-properties-pieces-plan.md`
- `thaum-painter/domain/painter-document/properties/`: `plans/home-j-repos-thaum-painter-domain-painter-document-properties-plan.md`
- `thaum-painter/domain/painter-document/properties/interpolation/`: `plans/home-j-repos-thaum-painter-domain-painter-document-properties-interpolation-plan.md`
- `thaum-painter/domain/file/storage/`: `plans/home-j-repos-thaum-painter-domain-file-storage-plan.md`
- `thaum-painter/domain/painter-session/timeline-state/`: `plans/home-j-repos-thaum-painter-domain-painter-session-timeline-state-plan.md`
- `thaum-painter/domain/rendering/render-space/`: `plans/home-j-repos-thaum-painter-domain-rendering-render-space-plan.md`
- `thaum-painter/domain/modules/individuals/layers-panel/`: `plans/home-j-repos-thaum-painter-domain-modules-individuals-layers-panel-plan.md`

## encapsulation-details
### `thaum-painter/domain/painter-document/properties/pieces/`
- action: new
- intent: the single source of truth for "what piece is this breath" — `BarPiece` (single / left head / right head / center) × cell type (empty / solid) classified from the covering bar and breath position. Also owns the tiling-total covering lookup (never `None` inside the layer span).
- interface delta: new classifier interface; consumes `properties/` block lookup.
- dependencies: `thaum-painter/domain/painter-document/properties/`
- consumers: `layers-panel/` (48-branch routing), `timeline-state/` (empty rejection), `render-space/` (empty renders nothing).
- artifacts: none
- tests: inline `#[cfg(test)]` — piece boundaries on 1/2/3+ breath bars, both cell types, tiling-total lookup.
- data: none

### `thaum-painter/domain/painter-document/properties/`
- action: edit
- intent: contract moves to binary tiling semantics; `clamped_breath_span` (gap-allowing + unwedge paths) dies; moves resolve via `pushed_breath_span` ripple and swap; destructive victim resolution reduces to "victims become blanks" so coverage is never broken.
- interface delta: removes `clamped_breath_span`; keeps span coverage/ripple helpers; declares `pieces/` child.
- dependencies: `thaum-painter/domain/file/`
- consumers: painter-session, timeline-state, render-space, storage.
- artifacts: none
- tests: inline `#[cfg(test)]` updated — tiling truth, ripple/swap, no clamp seam.
- data: none

### `thaum-painter/domain/painter-document/properties/interpolation/`
- action: delete
- intent: `GapFill`, `surrounding_items`, `resolve_gap_fill` existed only for voids; voids stop existing. Per-row interpolation (move lerp vs raster, user-settable empty mode) rebuilds in a fresh later pass.
- interface delta: removes `resolve_gap_fill`.
- dependencies: none (was `thaum-painter/domain/file/`)
- consumers: `render-space/` call sites collapse in the same pass.
- artifacts: none
- tests: removed with the encapsulation.
- data: none

### `thaum-painter/domain/file/storage/`
- action: edit
- intent: tiling invariant on every edit seam (`set_property_block_timing*`, split, blank, destructive, layer resize, window change): freed windows become blank blocks, coverage never breaks. `merge_blank_property_block` dies. Schema version bump — no migration pass, no importer; old files recognized as unsupported without crashing.
- ownership note (2026-09-07, J-approved): operator-2 takes over the unsupported-load rejection MECHANISM (typed error + clean open-flow rejection) in `plans/project-thaum-painter-file-boundary-cleanup-and-schema-gate-plan.md`. This plan's storage scope stays: bump the version constant, re-tile edit seams, and let old files fail into that typed rejection.
- interface delta: removes merge-blank mutation; edit seams now re-tile.
- dependencies: `thaum-painter/domain/painter-document/properties/`
- consumers: painter-session sync, persistence.
- artifacts: none
- tests: inline `#[cfg(test)]` — every edit seam leaves the track fully tiled; old-schema file rejected cleanly.
- data: none

### `thaum-painter/domain/painter-session/timeline-state/`
- action: edit
- intent: `resolve_editable_breath` becomes cell-type aware: auto-key off + playhead over an **empty** → reject (user cannot submit a new key and is not looking at a key); over solid → allowed as today.
- interface delta: none (same function, new rule).
- dependencies: `thaum-painter/domain/painter-document/properties/` (+ `pieces/` for cell type).
- consumers: painter-session edit gating.
- artifacts: none
- tests: inline `#[cfg(test)]` — auto-key off over empty rejects; over solid allows.
- data: none

### `thaum-painter/domain/rendering/render-space/`
- action: edit
- intent: the two `resolve_gap_fill` call sites collapse: playhead over an empty renders nothing (interim stub), over solid renders as today. Per-row interpolation is a later pass outside this project.
- interface delta: none outbound; stops depending on `interpolation/`.
- dependencies: `thaum-painter/domain/painter-document/properties/`
- consumers: renderer handoff.
- artifacts: none
- tests: existing render-space tests updated for the empty-covers-breath case.
- data: none

### `thaum-painter/domain/modules/individuals/layers-panel/`
- action: edit
- intent: the 48-branch interaction matrix — 6 interactions (L-click, R-click, L-drag, R-drag, dbl-L, dbl-R) × 4 pieces × 2 cell types — routed cleanly off the `pieces/` classifier, deliberately not one big calculation. `resolve_property_drag_mode` rewritten against piece × cell type; merge-blank UX (`MergeBlankPropertyBlock`, `blank_merge_direction`) dies; blanks get the same piece-based UX, similar but not identical.
- interface delta: removes `MergeBlankPropertyBlock` action; piece-keyed dispatch replaces size-keyed drag-mode resolution.
- dependencies: `thaum-painter/domain/painter-document/properties/pieces/`
- consumers: orchestration applies queued actions.
- artifacts: none
- tests: layers-panel interaction tests rewritten per piece × cell type; merge-blank cases removed.
- data: none

## phases
### phase-1 — project alignment
- [x] design truths locked with J and recorded in `context/bars-binary-design-truth.md`
- [x] touched-encapsulation declaration and dependency ordering
- [x] one encapsulation plan per touched encapsulation, linked above
- [x] placement approved: classifier in `properties/pieces/`, routing boxed in `layers-panel/`

### phase-2 — `properties/pieces/` (new, dependency root)
- [+] land the classifier encapsulation per its plan

### phase-3 — `properties/` (edit)
- [ ] binary tiling semantics, `clamped_breath_span` removal per its plan

### phase-4 — `file/storage/` (edit)
- [ ] re-tiling edit seams, schema break, merge-blank death per its plan

### phase-5 — `timeline-state/` + `render-space/` (edit)
- [ ] empty-rejection rule and gap-fill call-site collapse per their plans

### phase-6 — `interpolation/` (delete)
- [ ] remove the dead encapsulation per its plan

### phase-7 — `layers-panel/` (edit)
- [ ] 48-branch piece × cell-type routing per its plan

### phase-8 — final verification
- [ ] full test suite green across touched encapsulations
- [ ] encapsulation-checker / repo-rule review on each touched encapsulation
- [ ] git commit if approved

## post-implementation-notes
- plan-finished: false
- encapsulation-git-commit: false
