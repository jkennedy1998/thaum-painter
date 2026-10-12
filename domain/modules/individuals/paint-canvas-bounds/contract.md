# thaum-painter/domain/modules/individuals/paint-canvas-bounds

## purpose
Own painter's live drawing-space bounds gizmo so the active paint area is visible, movable, and resizable without hardcoding a fixed entrypoint-only rectangle.

## owns
- painter's shared `CanvasBounds` gizmo binding
- syncing move/resize gizmo drags into live drawing-space bounds
- keeping plane-selection bounds in sync with the active drawing-space bounds
- exposing the canvas-bounds gizmo hot spots painter uses to avoid accidental paint clicks on the gizmos themselves
- the drawing-space border-row gizmos: the wheel-mode toggle and the drawing-background widget (J)
- applying a hand to the drawing file's background through the background widget (content hand fills, clear hand sets transparent)
- the drawing-space backdrop group: a screen-locked box behind the module bounds filled with the drawing file's background color, rendered behind all characters regardless of depth

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
  - light
  - validates the background widget's border-row slot, gizmo-scheme look, hand-to-background clicks, and the backdrop group's rect fill

## data
- none

## notes
- the bounds themselves stay world-space (`CanvasBounds`), which keeps this compatible with later 3d camera swing/roll work even though the current gizmo interaction still rides the 2d module seam.
- the background widget renders as the 'B' glyph in the gizmos' own color scheme (medium, vivid on hover), regardless of the background color it represents (J 2026-10-11). it sits right of the wheel-mode toggle with one border gap on each side, and the DRAWING SPACE title starts two cells past the widget.
- the drawing file's background color renders only as this module's backdrop box; the window clear color stays the app's UI background (J 2026-10-11). the file-truth storage section lives in `domain/file/storage/`.
