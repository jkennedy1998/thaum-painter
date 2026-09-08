# thaum-painter/domain/painter-document/properties/raster-smear

## purpose
Own the raster-only `smear` interpolation result between two raster keyframe canvases.

## owns
- bounded, deterministic source-to-target cell correspondence for a single raster transition
- discrete swept-cell paths and the middle-weighted trail exposure window
- trail/head splatting and deterministic collision selection
- conservative unmatched-cell fallback through the ordinary raster blend

## does not own
- persisted interpolation vocabulary, cycling, or empty-bar eligibility (`../interp_mode.rs` and storage own those)
- empty-bar timeline progress/easing (`../interp_move.rs` owns that)
- normal same-position raster interpolation and the shared cell appearance resolver (`../interp_raster.rs`)
- glyph similarity/rasterization or shape-fade paths (`thaum-renderer/domain/cell-graphic/shape-fade/`)
- canvas persistence or renderer handoff

## contents
- `contract.md` — this ownership boundary
- `raster_smear.rs` — pure smear canvas resolver and inline tests

## dependencies
- `thaum-painter/domain/painter-document/properties/interp_raster.rs`
- `thaum-painter/domain/painter-operations/brush/`
- `thaum-renderer/domain/cell-graphic/shape-fade/`

## exposed interfaces
### smear canvas resolver
send: source canvas, target canvas, eased transition progress, optional loaded shape-fade resolver
returns: a resolved inbetween canvas with transported heads and discrete cell trails
effects: none
via: `smear_canvases`

## tests
- inline `#[cfg(test)]` in `raster_smear.rs`
  - translation heads/trails, endpoint identity, unmatched fallback, and collision priority

## notes
- This first pass is deliberately conservative: it only proposes bounded one-to-one matches that have compatible graphics/color/weight evidence. Glyph pairs may differ, but their visual transition remains owned by the injected weighted shape-fade resolver.
- Smear exists only for interior blank blocks of the raster property channel. Storage owns enforcing that authoring rule; this resolver only receives two already-valid keyframes.
- The resolver remains 3D-grid based. Its swept path has no camera orientation dependency.
