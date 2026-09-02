# /home/j/Repos/thaum-painter/domain/painter-session/selection

## purpose
Own live selection state for plane and world-oriented authoring flows.

## owns
- plane selection state
- world selection state
- selection mode semantics like replace/add/subtract/intersect
- selection bounds and selection-presence session truth
- the common seam that decides whether image edits are allowed through the current plane selection

## does not own
- clipboard payloads
- pure shape rasterization
- saved file manifest ownership

## children-encapsulations
- none

## contents
- `contract.md`
  - selection contract
- `selection_state.rs`
  - live plane/world selection state, selection mode, plane helper ops, edit gating, flood-select traversal, and live plane-bounds retargeting

## dependencies
- `/home/j/Repos/thaum-painter/domain/painter-document/`
- `/home/j/Repos/thaum-painter/domain/painter-operations/shapes/`

## exposed interfaces
- `PainterSelection`
  - owns the live plane-selection state, world-selection state, active selection mode, and plane-edit gating helpers together
- `PlaneSelection`
  - bounded plane selection bitmap-like state
- `WorldSelection`
  - unbounded world-cell selection set
- `SelectionMode`
  - replace/additive/subtract/intersect semantics shared by plane and world selection application
- `flood_select_points(...)`
  - derives contiguous plane points from a canvas plus a select-channel mask

## interface consumers
- painter-session
- rendering camera

## artifacts
- none

## tests
- inline `#[cfg(test)]` in `selection_state.rs`
  - light
  - validates plane/world selection state, mode application, edit gating, invert/select-all helpers, border detection, and flood-select masking

## data
- none

## notes
- source-of-truth from J: "Selection as a concept is going to have to be owned by the painter because it's not a renderer thing."
- this seam lives under painter's own `domain/` on purpose for exactly that reason.
- plane-selection bounds can now be retargeted live when painter's active drawing-space gizmo moves or resizes the editable area; out-of-bounds selected cells are pruned immediately.
