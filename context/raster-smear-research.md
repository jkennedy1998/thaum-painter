# raster smear research

## J intent

> "i want to create a new raster interpolation mode called 'smear'"
>
> "the idea of creating raster frames that are inbetween drawings that preserve the drawing details but create a smear / translation of movmement with the cells."
>
> "pixel motion blur is this concept i think in after effects. the idea of computing motion from pixel information."

This is research context, not an implementation decision yet.

## J trail decision

> "the trails should show straks / cell trails."

`smear` must therefore render visible discrete streaks/cell trails, not merely transport a crisp cell to its inbetween position.

## J trail-appearance decision

> "the trail should ideally blend characters that are alike with this ideally so we can take account for weight. we do this a bit with our current interpolation for raster which we built today."

A matched cell trail must use the same weight-aware shape transition as the existing raster interpolation. Repeating one source glyph at lower weight is insufficient when the paired source/target glyphs are visually related.

## distinction

The desired result is closer to **motion-aware frame interpolation** than conventional motion blur. Motion blur averages samples along a motion path and intentionally loses crisp detail. A raster `smear` mode should instead infer correspondence between the two keyframe canvases, transport cells along that correspondence, and only use a visible trail where that is an intentional part of the style.

## relevant research

- Rutherford's *Interpolating Pixel Art* adapts energy-cost minimization to assign a motion vector at each pixel position, but explicitly investigates why ordinary image interpolation does not adequately handle pixel art. This supports a discrete, pixel/cell-aware approach rather than applying a normal optical-flow filter unchanged. <https://mars.gmu.edu/items/bd00dbfe-0d9b-49f8-86de-76208168a43f>
- Mahajan et al.'s *Moving Gradients* describes a path-based interpolation method which solves multiple inbetween frames together for temporal consistency. The useful idea here is paths/correspondence, not its continuous-image output. <https://dl.acm.org/doi/10.1145/1531326.1531348>
- Modern optical-flow interpolation must explicitly handle occlusion and disocclusion. OCAI uses forward warping, occlusion awareness, and forward/backward consistency to decide which source owns a destination and to fill holes. Those concerns remain even on a discrete cell grid. <https://arxiv.org/html/2403.18092v1>

## local fit

`domain/painter-document/properties/interp_raster.rs` is already the single raster interpolation resolver. It receives the two keyframe `Canvas` values and the eased empty-bar progress; its current `interpolate` mode blends cells only at identical grid coordinates.

`domain/painter-document/properties/interp_mode.rs` owns the persisted mode vocabulary and the timeline glyph/cycle. `domain/file/storage/storage.rs` owns the empty-bar mode mutation. `domain/rendering/render-space/render_space.rs` already consumes the resolved canvas, so a smear result needs no renderer-handoff or saved-canvas change.

The existing `move` property is a separate authored whole-layer transform. Smear should not silently combine its inferred movement with that transform: doing both for the same apparent translation would double-move the drawing.

## recommended first shape

Add `smear` as a raster empty-bar interpretation, resolved only in `interp_raster`:

1. Build a bounded, discrete correspondence field between occupied source and target cells. Score candidate matches by local cell-patch similarity, compatible graphic/color/weight, spatial distance, and neighbor agreement. A plain nearest-cell match is insufficient: it scrambles nearby drawing detail.
2. Reject low-confidence matches and mark unmatched source/target cells as disappearances/disocclusions.
3. At an inbetween progress `t`, forward-warp accepted source cells to their rounded `source + t * vector` locations and backward-warp target cells from `target - (1 - t) * vector`. Resolve collisions by correspondence confidence plus endpoint proximity; never synthesize a new glyph/color outside the current palette/typeface rules.
4. Use the ordinary one-sided fade behavior for unmatched cells. Leave holes rather than inventing detail. A later explicit visual-trail rule can fill a cell's integer path if J wants a true smear-streak aesthetic.

This preserves discrete authored cells and provides deterministic, testable behavior without pulling in a heavyweight continuous optical-flow model. It also keeps the existing `interpolate`, `hold`, and loop meanings intact.

## online findings: how trails are generally built

- Traditional smear frames are deliberately non-literal drawings: animators stretch a moving silhouette or show multiple positions to make rapid motion readable. The visual goal is a brief, directional motion path, not physically correct image averaging. <https://www.bloopanimation.com/the-art-of-smear-frames>
- Conventional real-time motion blur starts with a motion/velocity vector per pixel, samples along that vector, and accumulates the samples. That explains the path shape but its averaging is wrong for thaum's crisp indexed cells. <https://developer.nvidia.com/gpugems/gpugems3/part-iv-image-effects/chapter-27-motion-blur-post-processing-effect>
- Flow-based frame interpolation instead forward-warps source pixels into an inbetween frame. Multiple source pixels can map to one destination, so a *splat* resolve chooses an owner; Softmax Splatting uses an importance metric for this exact collision problem. <https://arxiv.org/abs/2003.05534>
- Occlusion is unavoidable: a source cell may be covered in the target frame, and a target cell may be newly exposed. Modern flow work validates motion using forward/backward consistency and leaves/fills only the regions that are not safely owned by a correspondence. <https://arxiv.org/html/2403.18092v1>
- Pixel-art-specific research warns that ordinary continuous-image interpolation does not transfer cleanly to discrete art. The discrete cell grid, nearest palette, authored glyph shapes, and hard edges need to remain first-class rules. <https://mars.gmu.edu/items/bd00dbfe-0d9b-49f8-86de-76208168a43f>

## trail construction fit for thaum

For every confident source→target cell correspondence, use its motion vector for two outputs:

1. **head:** place the transported cell at its eased inbetween location;
2. **trail:** rasterize a discrete line of cells behind that head along the same vector. This is a *swept-cell* or *stroked-path* operation, not a blurred color average.

The trail should have an envelope: zero length at either keyframe, widening around the fast middle of the transition, then collapsing back to zero. At a given breath, a trail is the section of the correspondence path inside that exposure window, quantized to grid cells. A simple first pass can use an integer line walker and repeat the source glyph/color along the path, reducing `weight_index` from head to tail. The existing weighted shape-fade rules can then choose lower-coverage glyphs at the tail without introducing unsupported characters.

Splat/collision policy is needed because trails cross: a real transported head wins over a trail; otherwise prefer the higher-confidence correspondence, then the sample closest to its head. Unmatched/occluded cells retain the existing one-sided fade rather than emitting a speculative long streak.

## open product decisions

- Should the matching be allowed to pair changed glyph/color cells when their surrounding patch indicates the same moving detail, or only near-identical cells?
- Does a mode need a maximum motion radius/confidence setting, or should a conservative fixed bound be proven first?
- What is the rule when explicit layer `move` animation and raster `smear` coexist: forbid it, make smear use only residual raster motion, or let both layer deliberately?

## first-pass scope accepted

> "it should only be availible for empties that are not on the left or right side within the raster property channel"

`smear` is therefore a raster-only mode for an interior empty: it needs a source keyframe on its left and a target keyframe on its right. Raster edge blanks and every move-channel blank skip it when cycling modes.

## encapsulated implementation shape

Keep `interp_raster.rs` as the raster-channel dispatcher: it owns blank-bar mode resolution, loop behavior, keyframe lookup, and the existing ordinary `interpolate` result. Do not place correspondence, line walking, or collision rules in storage, the timeline UI, or render-space.

Add one focused child encapsulation:

- `domain/painter-document/properties/raster-smear/`
  - `contract.md` — owns only raster keyframe correspondence, swept-cell trail construction, confidence/occlusion policy, and deterministic output collision selection.
  - `raster_smear.rs` — a pure `smear_canvases(from, to, eased_progress, graphic_fade) -> Canvas` resolver plus focused tests.

`interp_raster.rs` dispatches its new `smear` mode to that resolver. `interp_mode.rs` remains the single persisted mode vocabulary and gains `smear` as an ease-adjustable middle-empty mode. `storage.rs` and the layers panel remain generic because they already cycle/render the shared vocabulary.

The only shared implementation seam justified by this second use is the current private matched-cell appearance blend in `interp_raster.rs`. Promote it narrowly as an internal raster-cell appearance resolver, taking `from`, `to`, shape progress, and output weight. It must continue to:

- numeric-lerp the authored weights and select a glyph at the actual output render weight through `ShapeFade::resolve_weighted_shape_fade`;
- lerp flat RGB then snap to the indexed palette;
- hard-cut material colors;
- use existing endpoint fallbacks when no typeface shape-fade graph is available.

The smear resolver calls that seam once for each trail sample. Its local path progress chooses the glyph/color transition; its tail attenuation lowers the requested **output** weight before glyph selection, so the trail's chosen glyph is actually supported at that lower weight. This is the crucial distinction from blending first and merely reducing the finished cell's weight afterward.

The renderer's `shape-fade` encapsulation should remain the owner of glyph similarity/weighted resolution. If correspondence needs a glyph-similarity score beyond exact equality, add one small query there rather than reading its neighbor graph from painter code. Without that graph, the smear matcher should only consider exact graphics compatible; it should never guess that two glyphs are similar.

### resolver stages

1. Read effective source/target cells and form bounded candidate pairs.
2. Score candidate pairs by existing shape similarity, authored weight/color similarity, distance, and nearby cells supporting the same displacement; choose deterministic one-to-one correspondences.
3. For each match, trace the back-facing section of its discrete 3D path at the current eased progress. The exposure-width envelope is zero at both endpoints and largest mid-transition.
4. Resolve every head/trail sample through the shared weighted raster-cell appearance seam. Heads use full output weight; trail samples taper toward zero weight.
5. Splat samples into the output: a head beats a trail; then confidence; then the sample nearest its head. Render unmatched cells through the existing one-sided fade behavior.

This leaves the saved canvas/keyframe shape unchanged, keeps all work pure and replay-safe, and requires no renderer-handoff change.

## initial proof cases

1. A 3×3 glyph/color cluster translates right four cells: intermediate breaths move the coherent cluster without breakup.
2. The cluster changes color/glyph while translating: position transports while existing palette and weight/shape-fade rules remain valid.
3. Two clusters cross: they do not exchange identities or blend into one another.
4. A cell appears from behind another or disappears: unmatched content fades without a fabricated counterpart.
5. A raster pair with no confident correspondence falls back to conservative existing blending rather than a wild long-distance warp.
