# project plan — shape-fade glyph space (cell-graphic interpolation)

## pre-implementation-note
Ownership: the glyph/sprite tile substrate lives in thaum-renderer (`domain/cell-graphic/glyph/glyph_graphic.rs` — `GlyphFontSet`, `GlyphTileRaster`, 12×16 u8 alpha, `rasterize_glyph_tile(char, weight)` with sprite tiles taking precedence). The shape-similarity graph and fade resolution are **renderer-domain** state derived from the typeface; the painter document layer never stores masks or similarity data. The painter's `interp_raster` consumes the fade through an injected seam so `thaum-painter-domain` stays free of font knowledge.

Research grounding (2026-09-07): ascii-morph (deconstruction effect, no correspondence), niccolofanton/morphing-ascii-shader (per-cell temporal memory + SDF glyph morphing), Structure-based ASCII Art (SIGGRAPH Asia 2010), AnimeInbet (ICCV 2023). Consensus across all schools: correspondence is the hard part; discrete-glyph projection along the A→B line in mask space is the tractable middle.

## current-state
`interp_raster::blend_cells` hard-cutoffs the graphic at eased-t = 0.5. The renderer rasterizes every rendered glyph/sprite into a 12×16 alpha tile already, but throws the shape away after drawing — no similarity structure exists.

## target-encapsulation
- `thaum-renderer/domain/cell-graphic/shape-fade/` (encapsulation, contract drafted 2026-09-07): mask space, similarity metric, neighbor graph, fade resolution + cache. Owned by renderer; rebuilt on `GlyphFontSet` load (typeface update ⇒ rebuild falls out naturally — it is derived state, never persisted).
- `thaum-painter/domain/painter-document/properties/interp_raster.rs`: consume an injected fade resolver `Fn(&CellGraphic, &CellGraphic, f32) -> CellGraphic`; default (no resolver / no fonts) = existing halfway cutoff.

## core design
1. **Mask space**: binary coverage masks from `GlyphTileRaster.alpha` (threshold > 0), packed 12×16 → `[u64; 3]` (192 bits). Keyed by char with the same sprite-over-font precedence `rasterize_glyph_tile` uses, so glyph↔sprite fades work in one space.
2. **Metric**: Dice coefficient via popcount: `sim = 2|A∩B| / (|A|+|B|)`; empty mask (space) has similarity 0 to everything — fading to space *is* the dissolve.
3. **The mapping** (the "how each character fades to others" ask): full N² pairwise similarity computed once per typeface load (N ~ few hundred ⇒ ~1.5M popcount ops, milliseconds), kept as per-char k-nearest-neighbor lists (k = 8). No persistence; rebuilt on font-set load.
4. **Fade resolution**: `resolve_shape_fade(from, to, t) -> char`:
   - Candidates = kNN(from) ∪ kNN(to) ∪ {from, to} (≤ 2k + 2).
   - Ideal blend mask M(t) = per-pixel alpha lerp of the two endpoint tiles (alpha arrays already exist in `GlyphTileRaster`).
   - Pick the candidate minimizing L1 alpha distance to M(t) — but **accept an intermediate only if it beats both endpoints** at that t; otherwise return the nearer endpoint. Guarantees never-worse-than-today: degenerate/disjoint pairs degrade to the honest cutoff.
   - Quantize t (e.g. 32 buckets) and LRU-cache per (from, to, bucket) → per-cell runtime cost is one hash lookup per frame.
5. **Determinism**: pure function of (from, to, t, typeface). No randomness — scrub-back stability, replay/reload identity, and the fuzzer invariants all hold.
6. **Weight**: graph built at one canonical weight (shape topology is near weight-invariant); rendered cells keep their authored weight. Per-weight graphs only if the canonical graph proves too coarse.
7. **Eases flow through unchanged**: t is already the eased progress from `interp_move::empty_progress`; the fade path just walks it.

## why this won't suck (guardrails)
- never-worse fallback (rule 4), bounded precompute, bounded per-frame cost, zero document-schema impact, pure/deterministic, fully property-testable off synthetic masks (no font files needed in tests: build `GlyphTileRaster`s by hand).
- Named risk: some pairs pass through ugly intermediates; the accept-only-if-closer rule keeps those rare, and k small keeps the candidate pool local in shape space.

## phases
### phase-1 — renderer: mask space + metric + graph
- [ ] `glyph-shape-fade` module: mask packing, Dice similarity, all-pairs kNN graph built from a `GlyphFontSet` at load; rebuilt on reload
- [ ] property tests on hand-built masks: symmetry, self-similarity, space-dissolve, sprite-over-font parity

### phase-2 — renderer: fade resolution + cache
- [ ] `resolve_shape_fade(from, to, t)` with accept-only-if-closer rule, t-quantized LRU cache
- [ ] tests: 'O'→'#'-class bridges resolve through plausible intermediates; disjoint pairs fall back to endpoints; cache hit path identical to cold path

### phase-3 — painter: consume the seam
- [ ] inject the resolver into the raster blend (storage/render seam param, default = halfway cutoff)
- [ ] `interp_raster` tests updated: matched cells resolve the fade path instead of the binary flip when a resolver is present; no-resolver path byte-identical to current behavior
- [ ] end-to-end: two keyframes + interpolating empty scrubs through intermediate glyphs

## tests
- renderer: mask packing round-trip, metric properties, graph rebuild, fade resolution fallbacks, cache behavior
- painter: injected-resolver blend, default-cutoff parity, determinism across scrub directions

## data
- none (all derived from the typeface at load)
