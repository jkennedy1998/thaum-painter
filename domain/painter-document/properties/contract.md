# thaum-painter/domain/painter-document/properties

## purpose
Own editing-facing property-block views over painter file state.

## owns
- property lookup helpers
- block-range lookup views
- shared breath-span semantics for every block shape in the painter: `span_end_breath` (exclusive end of a start+length span) and `breath_in_span` (coverage). Binary tiling: property tracks tile the full layer span — every breath is empty or solid, never a void, so the old gap-allowing/unwedging `clamped_breath_span` no longer exists; moves resolve through ripple (`pushed_breath_span`) and swap, and destructive resolution turns covered victims into blanks
- document conveniences for raster, move, rotation, and related authoring properties

## does not own
- canonical stored property schema
- mutation commands
- renderer handoff
- per-channel interpolation resolution itself (the move channel lives in `interp_move`, the raster channel in `interp_raster`; both consume this folder's span semantics and `interp_mode`'s mode/ease vocabulary)

## children-encapsulations
- `pieces/`
  - default
- `raster-smear/`
  - default

## contents
- `contract.md`
  - properties contract
- `properties.rs`
  - `block_covering_breath`, the lookup that finds the bar (if any) covering a given breath
- `interp_mode.rs`
  - the interpolation-mode vocabulary: interpolate / hold / loop_out / loop_in on every property channel, plus ease-adjustable `smear` only on interior raster empties; owns availability, edge locking, cycling, and center glyphs
- `interp_move.rs`
  - the move channel's real resolver: hold / interpolate (with ease-bent lerped offset) / edge-locked loops, plus the shared `empty_progress` ease model the raster channel reuses
  - settled truth (J 2026-09-09): a valueless solid (the born-tiled placeholder, or a split remainder of it) IS the identity keyframe — it renders unshifted at every breath it covers, so an interpolating empty beside it lerps from the unmoved position. It never counts as "no keyframe": the old hold-next degradation beside a placeholder was the broken move-interpolate J reported
- `interp_raster.rs`
  - the raster channel's real resolver: hold / interpolate / interior-only smear / edge-locked loops over keyframe canvases. Ordinary blending matches cells by grid position; flat RGB lerps then resolves to the nearest indexed palette color; shape-faded glyphs score their source and target at their authored render weights and select intermediates at the current output weight; material colors hard-cut at halfway; one-sided cells fade their weight and walk toward/from the low-coverage `▪` clear-transition glyph
- `raster-smear/`
  - raster-only first-pass correspondence and discrete transported cell trails; owns bounded one-to-one matching, trail envelope/path rasterization, and head/trail collision selection while consuming `interp_raster`'s shared weight-aware cell appearance resolver
- `pieces/`
  - bar-piece classification: `BarPiece` (single / left head / right head / center) × `CellType` (empty/solid) per breath, the `BreathBar` shape trait, and the covering `BarCell` lookup

## dependencies
- `thaum-painter/domain/file/`
- `thaum-painter/domain/painter-session/paint-color/`

## exposed interfaces
### block lookup
send: a property's block list plus a breath
returns: the block covering that breath, if any
effects: none
via: `block_covering_breath`

### breath-span semantics
send: start+length spans (any block shape) plus a breath, or a requested span plus the channel's other spans
returns: coverage truth, span end, the ripple resolution (`pushed_breath_span`), or the destructive resolution whose victims become blanks (`destructive_breath_span`)
effects: none
via: `span_end_breath`, `breath_in_span`, `pushed_breath_span`, `destructive_breath_span`

## interface consumers
- painter-session
- painter-session/timeline-state
- render-space

## artifacts
- none

## tests
- inline `#[cfg(test)]` in `properties.rs`
  - light
  - validates exact-boundary lookups, gap breaths, and an empty track all resolve correctly; span helper coverage/boundary truth; the ripple seam preserving relative spacing on both edges; the destructive seam blanking/removing covered victims and behaving as a plain trim on shrink
- inline `#[cfg(test)]` in `interp_mode.rs` and `interp_move.rs`
  - light
  - validates mode/ease cycling, edge locking, glyphs, and the move resolver's hold/interpolate/loop behavior over authored tracks
- inline `#[cfg(test)]` in `interp_raster.rs` and `raster-smear/raster_smear.rs`
  - light
  - validates ordinary canvas blending, smear trail/head resolution, endpoint identity, unmatched fallback, collision priority, the per-mode resolution over authored tracks, and loop replay

## data
- none
