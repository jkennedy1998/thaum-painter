# thaum-painter/domain/modules/individuals/paint-canvas-bounds

## purpose
Own painter's live drawing-space bounds gizmo so the active paint area is visible, movable, and resizable without hardcoding a fixed entrypoint-only rectangle.

## owns
- painter's shared `CanvasBounds` gizmo binding
- syncing move/resize gizmo drags into live drawing-space bounds
- keeping plane-selection bounds in sync with the active drawing-space bounds
- exposing the canvas-bounds gizmo hot spots painter uses to avoid accidental paint clicks on the gizmos themselves

## does not own
- generic gizmo math or pointer-capture behavior, owned by `thaum-renderer/domain/modules/shared/`
- painter's actual cell editing rules, owned by `domain/painter-session/tool-state/`
- scene-space rendering of the paint cells themselves, owned by `orchestration/entrypoint/` today

## children-encapsulations
- none

## contents
- `contract.md`
  - this contract
- `paint_canvas_bounds_module.rs`
  - painter-only bounds gizmo module

## dependencies
- `thaum-painter/domain/painter-session/selection/`
- `thaum-painter/domain/painter-operations/fill/`
- `thaum-renderer/domain/modules/`

## exposed interfaces
- `PaintCanvasBoundsModule`
  - painter module that edits shared drawing-space bounds through move/resize gizmos

## interface consumers
- `thaum-painter/orchestration/entrypoint/`

## artifacts
- none

## tests
- inline `#[cfg(test)]` in `paint_canvas_bounds_module.rs`
  - light
  - validates move/resize gizmo syncing into shared bounds and selection pruning

## data
- none

## notes
- the bounds themselves stay world-space (`CanvasBounds`), which keeps this compatible with later 3d camera swing/roll work even though the current gizmo interaction still rides the 2d module seam.
