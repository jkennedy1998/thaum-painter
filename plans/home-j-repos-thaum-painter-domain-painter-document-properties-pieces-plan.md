# encapsulation plan — properties/pieces

## pre-implementation-note
New encapsulation, dependency root of the binary-bars project (`plans/project-thaum-painter-binary-bars-plan.md`). Design truth lives in `context/bars-binary-design-truth.md`: channels are binary (empty/solid), UX routes by bar piece, never by bar size. Pieces: single head (1-breath bar), left head (first cell of 2+), right head (last cell of 2+), center (middle cells of 3+). Empties have all four pieces too. This seam only ever answers "what piece is this breath" — no pointer input, no mutation.

## target-encapsulation
- `thaum-painter/domain/painter-document/properties/pieces/`: new

## artifacts
- none

## tests
- inline `#[cfg(test)]` in `pieces.rs` — piece classification at every boundary, both cell types, tiling-total lookup.

## data
- none

## phases
### phase-1 — docs + contract + truth alignment
- [+] write `contract.md`: owns the `BarPiece` enum (single / left head / right head / center), the cell-type half of the classifier (empty/solid), the pure classify function `(covering bar, breath) -> piece × cell-type`, and the covering lookup that is total inside the layer span under tiling; does not own pointer input, mutation, or interpolation
- [+] declare dependency on `thaum-painter/domain/file/` (block shapes) and consumers (`layers-panel/`, `timeline-state/`, `render-space/`)
- [+] register as child of `properties/` contract

### phase-2 — classifier implementation
- [+] `pieces.rs`: `BarPiece` enum, `CellType`, `BarCell`, `BreathBar` trait (impls for stored + manifest block shapes), classify function
- [+] covering lookup (`covering_bar_cell`) — total within the layer span under tiling; `None` only for pre-tiling gap data
- [#] tests: 1-breath → single; 2-breath → left+right; 3+ → left + centers + right; both cell types on stored + manifest shapes; gap breaths resolve none
- [+] verify no unplanned cross-encapsulation work

### phase-3 — validation + repo-rule sweep + git commit
- [#] run the encapsulation tests (7 pieces tests + full workspace suite green)
- [ ] encapsulation-checker / repo-rule verification for the new encapsulation
- [ ] git commit if approved

## post-implementation-notes
- plan-finished: false
- encapsulation-git-commit: false
