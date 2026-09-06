# thaum-painter/domain/painter-document/properties/interpolation

## purpose
Own the resolution of what renders for a breath that no property block or raster segment covers, so gap-fill behavior (currently "nothing") and future real interpolation both live in one seam instead of being reimplemented per call site.

## owns
- the `GapFill` behavior enum for breaths not covered by any block/segment
- the `surrounding_items` lookup (nearest item before/after a gap breath)
- gap-fill resolution dispatch (`resolve_gap_fill`)

## does not own
- exact-coverage block/segment lookup (owned by `properties.rs`'s `block_covering_breath` and render-space's own segment lookups)
- the renderer handoff that consumes the resolved value
- real interpolation math (holding a value, lerping a `GridPoint`, etc.) — not yet built

## children-encapsulations
- none

## contents
- `contract.md`
  - interpolation contract
- `interpolation.rs`
  - `GapFill`, `BreathRanged`, `surrounding_items`, `resolve_gap_fill`

## dependencies
- `thaum-painter/domain/file/`

## exposed interfaces
### gap-fill resolution
send: a property's blocks or a group's raster segments, plus a breath not covered by any of them
returns: the `GapFill` behavior to render (today: always `NoContent`)
effects: none
via: `resolve_gap_fill`

## interface consumers
- `domain/rendering/render-space/`

## artifacts
- none

## tests
- inline `#[cfg(test)]` in `interpolation.rs`
  - light
  - validates `surrounding_items` on both sides of a gap and that `resolve_gap_fill` is `NoContent` today

## data
- none

## notes
- this is an intentional stub: `GapFill` has one variant (`NoContent`) matching the pre-existing "no block covering this breath means nothing renders" behavior. It exists so `render_space.rs` has one named place to call instead of ad hoc empty-returns, and so real interpolation (per property kind — move/rotation can lerp numerically, raster likely can't lerp voxel content the same way) has a landing spot later without replumbing call sites.
- `surrounding_items` is generic over `BreathRanged` so it works for both `PropertyBlock` (property tracks) and `RasterSegment` (raster tracks) without duplicating the lookup.
