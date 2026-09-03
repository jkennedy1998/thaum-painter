# /home/j/Repos/thaum-painter/domain/painter-operations/shapes

## purpose
Own pure line, rectangle, lasso, and later 3d shape rasterization helpers for painter authoring.

## owns
- line rasterization usage
- lasso (closed freehand polygon) rasterization into enclosed cells, pure and session-free
- rect and polygon shape point derivation
- future 3d primitive raster op boundaries for box, sphere, cylinder, and cone tools

## does not own
- selection session state
- history
- renderer composition

## notes
- source-of-truth from J: the lasso tool's bound is drawn by press-drag-release; releasing closes the path and acts on the enclosed cells, so the pure enclosed-cell derivation lives here, shared by image fills and selection surfaces.

## children-encapsulations
- none

## contents
- `contract.md`
  - shapes contract
- `lasso.rs`
  - `lasso_points(path)` even-odd closed-polygon rasterization (interior plus bound, path's plane) + raster tests

## dependencies
- `/home/j/Repos/thaum-painter/domain/painter-operations/brush/`

## exposed interfaces
### lasso rasterization
send: one closed freehand path of cells (implicitly closed, first point's plane)
returns: every enclosed cell (interior plus the path itself)
effects: none
via: `lasso_points(path)`

## interface consumers
- painter-session selection
- painter-session commands
- painter-session tool-state (lasso fill/selection application)

## artifacts
- none

## tests
- inline `#[cfg(test)]` in `lasso.rs`
  - light
  - validates closed-square interiors, concave notch exclusion, degenerate single-point/empty paths, two-point lines, and plane (z) preservation

## data
- none
