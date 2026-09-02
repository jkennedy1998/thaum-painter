# /home/j/Repos/thaum-painter/domain/painter-document/properties

## purpose
Own editing-facing property-block views over painter file state.

## owns
- property lookup helpers
- block-range lookup views
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
- `/home/j/Repos/thaum-painter/domain/file/`

## exposed interfaces
### block lookup
send: a property's block list plus a breath
returns: the block covering that breath, if any
effects: none
via: `block_covering_breath`

## interface consumers
- painter-session
- painter-session/timeline-state
- render-space

## artifacts
- none

## tests
- inline `#[cfg(test)]` in `properties.rs`
  - light
  - validates exact-boundary lookups, gap breaths, and an empty track all resolve correctly

## data
- none
