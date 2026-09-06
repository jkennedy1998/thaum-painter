# thaum-painter/domain/painter-document/properties

## purpose
Own editing-facing property-block views over painter file state.

## owns
- property lookup helpers
- block-range lookup views
- shared breath-span semantics for every block shape in the painter: `span_end_breath` (exclusive end of a start+length span), `breath_in_span` (coverage), and `clamped_breath_span` (the no-overlap rule: a block may never cross another block in its channel — requests are clamped into the free window between the moving block's neighbors, and straddling blocks in already-overlapping stored data fold to the nearer side so a move unwedges them)
- document conveniences for raster, move, rotation, and related authoring properties

## does not own
- canonical stored property schema
- mutation commands
- renderer handoff

## children-encapsulations
- `interpolation/`
  - default

## contents
- `contract.md`
  - properties contract
- `properties.rs`
  - `block_covering_breath`, the lookup that finds the bar (if any) covering a given breath

## dependencies
- `thaum-painter/domain/file/`

## exposed interfaces
### block lookup
send: a property's block list plus a breath
returns: the block covering that breath, if any
effects: none
via: `block_covering_breath`

### breath-span semantics
send: start+length spans (any block shape) plus a breath, or a requested span plus the moving block's original span and its channel's other spans
returns: coverage truth, span end, or the span clamped so no two blocks in one channel share a breath
effects: none
via: `span_end_breath`, `breath_in_span`, `clamped_breath_span`

## interface consumers
- painter-session
- painter-session/timeline-state
- render-space

## artifacts
- none

## tests
- inline `#[cfg(test)]` in `properties.rs`
  - light
  - validates exact-boundary lookups, gap breaths, and an empty track all resolve correctly; span helper coverage/boundary truth; the clamp seam stopping blocks at neighbors, shrinking trims that would cross, and unwedging previously overlapping data

## data
- none
