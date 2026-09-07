# thaum-painter/domain/painter-document/properties/pieces

## purpose
Own the bar-piece classification truth for property bars: which UX piece (single / left head / right head / center) and which cell type (empty / solid) a breath resolves to. The single seam the 48-branch interaction surface keys off.

## owns
- the `BarPiece` enum and its boundary rule (1 breath = single head; 2 breaths = left + right heads; 3+ = left head + centers + right head)
- the `CellType` distinction (empty/solid) read off a bar's blank flag
- the `BreathBar` bar-shape view trait so every block shape classifies through one seam
- the covering lookup resolving a breath to its `BarCell` (piece × cell type)

## does not own
- pointer input or interaction routing — that is `modules/individuals/layers-panel/`
- mutation or tiling-invariant enforcement on edit seams — that is `file/storage/`
- interpolation (dies with the void concept; per-row interpolation returns later as a fresh seam)
- the layer span itself — the lookup only classifies what callers pass

## children-encapsulations
- none

## contents
- `contract.md`
  - pieces contract
- `pieces.rs`
  - `BarPiece`, `CellType`, `BarCell`, `BreathBar`, `classify_bar_piece`, `cell_type_of`, `covering_bar_cell`

## dependencies
- `thaum-painter/domain/file/` (manifest + stored block shapes)

## exposed interfaces
### bar-piece classification
send: a bar shape (start, length, blank flag) plus a breath
returns: the `BarPiece` × `CellType` at that breath, or none when no bar covers the breath (only possible in pre-tiling data)
effects: none
via: `covering_bar_cell`, `classify_bar_piece`, `cell_type_of`

## interface consumers
- none yet — `modules/individuals/layers-panel/`, `painter-session/timeline-state/`, and `rendering/render-space/` consume this seam in the binary-bars project phases

## artifacts
- none

## tests
- inline `#[cfg(test)]` in `pieces.rs`
  - light
  - validates piece boundaries on 1/2/3+ breath bars, cell-type reads on both block shapes, and that gap breaths (pre-tiling only) resolve to none

## data
- none

## notes
- growth guard: this seam only ever answers "what piece is this breath". Interaction, mutation, and interpolation are foreign by contract — adding them here would blur the boundary the binary-bars project set up.
- under the tiling invariant every breath inside the layer span is covered by exactly one bar, so `covering_bar_cell` is total there; `None` survives only for pre-tiling stored data.
- `PropertyTrackBlock` (layers-panel mirror shape) implements `BreathBar` next to its own module to keep the document seam from depending on a UI module.
