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
- Document-owned truth split: the canonical selection store is the document's selection channel — `SharedDocumentFile.selection` in `domain/file/storage/` (per file, one shared 3D bitmap, single "selection" channel for now, later channel picker). This seam's `PainterSelection`/`PlaneSelection` hold the full 3D set as the interaction surface: `PlaneSelection.bounds` is only the active interaction plane (where strokes/select-all/invert land); the cell set itself is 3D and is never pruned by camera moves (`set_bounds` retargets without touching cells, so selections survive depth/plane changes). Plane-scoped semantics: `apply_points` Replace rewrites only the active plane's slice (other depths survive), select-all unions the slice, invert flips only the slice, clear drops everything. The overlay renders every selected cell across all depths the camera can see (filled glyphs, no border-only pass), matching the shared 3D bitmap expectation. Every commit mirrors the full UI set into the document channel as an exact Replace (`commit_selection_channel` in `orchestration/entrypoint/src/main.rs` — stroke release, clear/invert/select-all), so channel == UI cache always; boot restores the channel into the UI cache via `restore_points` (not plane-filtered). Selection changes are not per-layer undo records — they persist through the document snapshot.
