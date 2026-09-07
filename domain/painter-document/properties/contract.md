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
- gap-fill/interpolation behavior (the old `interpolation/` child was deleted 2026-09-07 with the binary-bars redesign — voids stopped existing, and per-row interpolation is a future pass)

## children-encapsulations
- `pieces/`
  - default

## contents
- `contract.md`
  - properties contract
- `properties.rs`
  - `block_covering_breath`, the lookup that finds the bar (if any) covering a given breath
- `pieces/`
  - bar-piece classification: `BarPiece` (single / left head / right head / center) × `CellType` (empty/solid) per breath, the `BreathBar` shape trait, and the covering `BarCell` lookup

## dependencies
- `thaum-painter/domain/file/`

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

## data
- none
