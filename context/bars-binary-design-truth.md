# bars binary design truth

## purpose
Source-of-truth from J for the binary bar redesign: property tracks tile fully with empty/solid bars, UX routes by bar piece, and voids stop existing. Recorded 2026-09 before implementation. This is the alignment record; the work lands across `domain/painter-document/properties/`, `domain/file/storage/`, `domain/modules/individuals/layers-panel/`, and `domain/painter-session/timeline-state/`.

## core truth: binary channels
- A property channel has exactly two cell types: **empty** and **solid** (content). Voids (breaths with no block) stop existing.
- Every property track (raster, move, and future rows: rotation, opacity, …) tiles the **entire layer breath span** edge to edge. All pieces of a bar — single, left head, middle, right head — live inside bars that together cover the whole layer span.
- Bar placement is binary: no gaps, no overlaps, ever. `clamped_breath_span`'s gap-allowing and unwedge paths die; moves resolve through ripple (`pushed_breath_span`) and swap.
- Move track and raster track use the **exact same bar logic**. Property rows will grow over time; bar behavior (binary, empty/solid interaction, UX) is single source of truth. Per-property logic is limited to interpolation and sometimes bar graphics — never bar structure.

## bar piece taxonomy
UX routes by **piece**, never by bar size. Pieces:
- **single head** — one-breath bar (one cell)
- **left head** — first cell of a 2+ breath bar
- **right head** — last cell of a 2+ breath bar (opposite of left)
- **center** — middle cells of a 3+ breath bar

Bar-size mapping: 1 breath = single head only; 2 breaths = left + right heads; 3+ = left head + centers + right head.

- Empties also have all four pieces. Empty UX is heavily similar to solid UX but **not identical** — it is still routed per piece so every interaction lands on a clean branch.
- Per piece, the user interaction surface is: left click, right click, left-click drag, right-click drag, double left click, double right click.
- Bar behavior must **not** be one big calculation: it should be sorted cleanly (piece × cell-type) so behaviors are easy to see and edit.

## cell-type transitions
- The only transitions between empty and solid:
  - paint / auto-key-on edit into an empty → it becomes solid (unblank-on-paint, existing rule, generalized)
  - blanking a solid → it becomes empty
- Auto-key **off** + playhead over an **empty** → edit **rejected** (the user cannot submit a new key and is not looking at a key).
- `merge_blank_property_block` / void-merge UX dies: its job was re-creating voids, which no longer exist.

## interpolation truth
- `GapFill`/`resolve_gap_fill`/`surrounding_items` die as-is (they existed only for voids).
- ~~Interim behavior: a playhead over an **empty displays nothing** (stub).~~ SUPERSEDED — real interpolation has landed for the move channel (see the landed passes below). Raster alone keeps the no-show-over-empties rule until its own channel pass.
- Future interpolation shape: each property row has its **own custom interpolation** (you cannot interpolate raster like you interpolate move). Empties fire in **one standard way**; the per-row difference lives in how a row's content resolves. The user will eventually set the interpolative mode of empty bars — that UX is not worked out yet.
- J-quote (2026-09-07): "it should stayt within the indexed color system regardless of what those indexed colors are."

## infinity truth (J 2026-09-07, after the operator's out_mode proposal was rejected)
- The track extends to **infinity on the right** (positive breaths). Left/negative is deferred on purpose — build asymmetrically right-first, but keep the architecture from kicking us later.
- **No track-level out_mode field.** J rejected that shape: it conflicts with interpretable keyframes. The interpretation lives on the **end blanks** — the trailing blank is the keyframe whose interpretation is what happens at infinity (Hold / Loop Out, per channel). The leading blank is the mirror (Loop In, when negative time arrives). Only the first/last blanks carry interpretation modes.
- This is analogous to After Effects easing, but the available interpretation modes differ **per property channel** (raster vs move resolve differently).
- The infinite region **is a blank**: it must look like a blank, hit-test like a blank, and interact like a blank. No ghosted/dimmed rendering, no fake infinite block in storage — the region past the last finite block resolves as the trailing blank.
- The trailing blank visually extends all the way to the right edge of the layers panel. Loop Out set on it makes the authored region repeat through that blank; the left end blank analog goes down to breath 0.
- **Viewport ≠ editing space.** The viewport (ruler / layer span) is what plays on animation and what exports. The user can edit outside the viewport — navigation there happens when not animating. Viewport growth must never be coupled to editing behavior.
- Timeline **scrolling is coming** — do not hardcode panel widths or span-derived pixel math in the infinite-region work.
- Setting the interpretation mode has **no UX yet**; the goal now is architecture that makes the behavior reachable without rework.

## infinity implementation (landed 2026-09-07)
- `retiled_property_track` no longer clips on the right: the track covers `[span_start, ∞)`. Blocks may live past the viewport end.
- The track always ends with a **trailing blank representative** — a finite stored blank that semantically extends to infinity. If the last block is solid (even past the viewport end), a minimal representative is appended right after it.
- Panel render: the trailing blank draws from its start through the panel's right edge, all center glyphs (`▪`), no right head — it has no end.
- Hit-test: breaths past the stored extent resolve to the trailing blank as `Center/Empty`, so every empty interaction (merge, drags) works out there unchanged.
- Duplicate no longer grows the layer span (viewport decoupled from editing).
- `set_layer_timing` resizes the viewport only: growing extends the trailing representative, shrinking never clips content.
- `SharedDocumentPropertyBlock.interpretation: Option<String>` is the end-blank interpretation slot (serde-default, unset everywhere, no UX).
- **Defaults (J 2026-09-07):** every property kind (raster, move, future ones) is born on the same binary tiling — solid block over the viewport plus the trailing blank. No property row ever renders as "nothing". The panel's missing-track fallback mirrors this too.
- **The trailing blank representative carries the semantic id `tail`** (dedup'd) — it is THE end-blank keyframe and must not consume numeric ids (`block-N`) that future content blocks expect to grow into.
- **Swaps re-tile aggressively:** a swap that butts two empties together merges them on the spot — the no-adjacent-empties invariant holds after EVERY mutation, swap included.
- **Destructive = crop, not erase (J 2026-09-07, bug fix):** a destructive drag that partially overlaps a solid victim CROPS it — the un-overwritten remainder keeps its span AND its content. Only a victim fully encapsulated by the moved bar is deleted outright. A victim the edit straddles (edit span strictly inside it) crops on both sides and splits into two blocks, both keeping content (the split-off half gets a fresh id and its own empty canvas — per-block canvases cannot be split pixel-accurately). Implemented as `DestructiveBreathSpan.trimmed`/`.removed` in `properties.rs`.

## loop-mode edge lock + move interpolation (landed 2026-09-07, second pass)
- **Loop modes are edge-locked (J 2026-09-07): `loop_out` is only toggleable on the track's LAST blank — the trailing blank, the right edge of time — and `loop_in` only on the FIRST blank — the leading blank, the left edge.** Enforced three ways: the cycle SKIPS a locked mode the empty cannot carry (a middle empty just toggles interpolate ↔ hold, nothing rejects in the user's hand); `enforce_loop_mode_edges` runs inside `retiled_property_track` so every mutating seam (swap, drags, splits, duplicates, growth, move paints) strips a loop mode stranded off its edge back to default interpolate with cleared eases; and documents re-tile on LOAD, so older files come in already normalized.
- **Move is the first real per-channel interpolation (J 2026-09-07):** `properties/interp_move.rs` resolves the move row — solids hold their `{x,y,z}` across their span; empties resolve per their authored mode. **hold** = the previous keyframe's offset carries through the empty. **interpolate** = lerp previous → next across the empty's span, progress bent by the ease ends (ease-out slows the departure `t^(1+2o)`, ease-in decelerates the arrival `1-(1-x)^(1+2i)`, composed so 0%/0% is exactly linear; 33/66/100% steps). A missing side holds the existing side. **loop_out / loop_in** repeat the authored region (first keyframe start → last keyframe end) forward / mirrored-backward; the breath before the first keyframe resolves to the region's last value (After Effects loop-in behavior).
- **Breaths past the tail blank's stored extent resolve AS the tail blank** in `move_offset_for_layer`, so a loop-out keeps playing through the infinite region instead of dropping to the zero offset.
- **Raster stays no-show over empties** (J 2026-09-07: fine as is for now) — raster's own channel resolution is a later pass; each channel gets its own interp module, nothing shared but bar structure.
- Load-time re-tiling also fixed the new-layer bug: rows built from a pre-binary document (or the missing-track fallback) now show the full binary shape — solid over the span plus the trailing blank — so empties are visible on fresh layer rows.

## file break truth
- New file schema, **no migration pass**, no importer. Migration bloat stays out of the system on purpose.
- Attempting to open an old file should recognize it is unsupported and not fail hard when the system realizes it — clean recognition, not a crash. Exact mechanism deferred; bar behavior comes first.
- Old files are handled case by case, manually, by J. (Released quietly; friends were told to hold off on animations, so exposure is minimal.)

## interaction matrix (48 branches, J dictation 2026-09-07)
6 interactions (L-click, R-click, L-drag, R-drag, L-dblclick, R-dblclick) × 4 pieces × 2 cell types. `(open)` = unresolved question, see the inconsistency list below.

### content (solid) bar
- **single**
  - L-click: select the layer this belongs to
  - R-click: unused
  - L-drag: positional drag, overwrites whatever it lands on (destructive move)
  - R-drag: resize, overwrites whatever it expands over (destructive, edge from pointer side)
  - L-dblclick: duplicate this single frame to the right; lands on the right, growing the layer span by exactly the duplicate's length when it would pass the span end (bounded — never auto-fills). (A length-1 bar cannot split, so single keeps duplicate while center splits — J 2026-09-07.)
  - R-dblclick: replace with empty (this is the new delete — empties are voids now)
- **left head**
  - L-click: select the layer
  - R-click: unused
  - L-drag: positional drag preserving the timing of bars around it (pushed ripple, resizes this bar's start edge); dragging left pushes content left, dragging right pulls content in from the left. Preview of every affected bar plus the dragged bar.
  - R-drag: positional drag that does NOT preserve surrounding timing — overwrites blocks under without moving them (crop, or delete if fully encompassed); dragged right it leaves an empty in the wake
  - L-dblclick: unused (J-flip 2026-09-07: reserved for keyframe interpolation between the adjacent empty and this cell)
  - R-dblclick: the bar on the left merges into this one; the double-clicked bar's content stays — implemented as destructive resize to the left bar's start position (J-flip 2026-09-07: merges live on the right button so right-click stays the delete/merge family; right heads mirror, span-edge rejects)
- **right head**: exact mirror of left head
- **center**
  - L-click: select the layer
  - R-click: unused
  - L-drag: swaps the actual keyframe content with the bar the drag lands on (the existing swap seam: spans exchange, each block's content follows it, so the two keyframes trade time positions)
  - R-drag: slides the bar destructively, overwriting content it lands on, leaving an empty in its previous span
  - L-dblclick: split the bar at the double-clicked breath — the bar separates there, left half keeps the id and breaths before the split, the right half gets the rest, and the track's total content length is unchanged (J 2026-09-07, reverted from duplicate back to the original split; implemented as `SplitPropertyBlock` at `hit.breath`)
  - R-dblclick: replace the block span with empties

### blank bar (empty / 0 / null of the binary system)
- Global invariant: empties are **never adjacent to other empties** — they merge into one.
- **empty single**
  - L-click / R-click: unused (but every interaction still selects the layer first — see selection rule)
  - L-drag: positional drag resizing this empty the same time-preserving (pushed) way as a solid left head — same code
  - R-drag: resize, overwrites whatever it expands over (destructive, victims become empty)
  - L-dblclick: unused (J-flip 2026-09-07: reserved for keyframing — left clicks on empties will author/inspect keyframe interpolation later)
  - R-dblclick: merge this empty into the adjacent content block — same seam as empty center: prefer the left side, fall back to the right, reject when the track has no content at all (J-flip 2026-09-07: R-dblclick, matching the content-head merge and the right-click delete family)
- **left empty head**
  - R-click: unused
  - L-drag: positional drag resizing this empty the same time-preserving (pushed) way as a solid left head — same code
  - R-drag: same destructive way as a solid left head
  - L-dblclick: unused (J-flip 2026-09-07: reserved for keyframing)
  - R-dblclick: merge this empty into the adjacent content block (content survives instead of the empty — the inverse of the solid left-head merge). Prefer/fallback: left-preferred, right fallback, reject on fully-empty track (J-flip 2026-09-07)
- **right empty head**: exact mirror of left empty head (R-dblclick merges, L-dblclick unused)
- **empty center**
  - L-click / R-click: unused (but every interaction still selects the layer first)
  - L-drag: swaps keyframe content exactly like solid center (predictability)
  - R-drag: resize, overwrites whatever it expands over (destructive, victims become empty)
  - L-dblclick: unused (J-flip 2026-09-07: reserved for keyframing)
  - R-dblclick: merge this empty into the adjacent content block — **J-flip 2026-09-07: R-dblclick, not L-dblclick**, so all merges (content ends + empty ends) live on the right button and double-left stays free for keyframe authoring. One seam shared with empty single: find a side with content preferring the left, fall back right, reject when there is no content on either side (fully-empty track).

## rulings from J (2026-09-07, answering the inconsistency list)
1. Empty center merge = **R-dblclick** (J-flip 2026-09-07: was L-dblclick; the right button owns the whole delete/merge family so double-left stays free for keyframe authoring on empties and content ends).
2. Left empty head L-dblclick = **unused** (reserved for keyframing).
3. Empty merges (single + center, both heads mirrored) are **one seam**: find a side with content, prefer left, fall back right, **reject when there is no content on either side**.
4. Content single L-dblclick duplicate = **same as center duplicate**: non-destructive push, fewer distinct outcomes for the user to expect.
4b. Merge button family (J 2026-09-07 second pass): **all merges are R-dblclick** — solid left/right head merges and every empty merge. L-dblclick on solid heads and on empties is unused, reserved for keyframe interpolation (the first real user will be the move row, linear interpolation mode; long term the user toggles each empty's interpolation mode, so empties become authored content).
5. Duplicate at the span end: **place the new block next to it, on the right** — duplicates always land on the right. (Superseded 2026-09-07 by the infinity truth: the duplicate no longer grows the layer span — the viewport is never coupled to editing, and the infinite trailing blank absorbs the room.)
6. Center L-drag swaps the **actual keyframe content** (raster canvas / move value / whatever the property is) — the existing swap seam: spans exchange, content follows each block, so the two keyframes trade time positions. Already in the system.
7. Empty center L-drag swaps the same way — predictability.
8. Right-head mirroring is **total** — direction flips for every interaction.
9. No-adjacent-empties is the new global invariant: "the user should only ever see empties and bars — never a blank property row." Enforced by one normalization in `retiled_property_track` (merge adjacent blanks), which every mutating seam runs.
10. **Selection rule (J-corrected): any interaction on a property row selects that layer first** — left click, right click, double clicks, drags. Select first, then act. (The "unused" branches mean no *additional* action, not no selection.)
11. Left-head merge rejects when the bar is at the left edge / has no left neighbor. With empties everywhere, a non-edge bar always has something to run into — no unbounded fill.
12. Tooltips: the matrix table here is the **source of truth**; the tooltip copy is short and J writes the final wording. Format: title `"property bar <piece>"` + one short line, e.g. left head: "the left bound of a single bar. left drag preserve resizes, right drag destructive resizes, double left duplicates, double right deletes". Operator interpolates drafts for the other pieces; J finalizes later.

## consumers of this truth
- `domain/painter-document/properties/contract.md` — binary span/tiling semantics
- `domain/painter-document/properties/interpolation/contract.md` — gap-fill death, per-row interpolation intent
- `domain/file/storage/contract.md` — schema break, tiling invariant, no migration
- `domain/modules/individuals/layers-panel/contract.md` — piece taxonomy, interaction routing, empty/solid UX split
- `domain/painter-session/timeline-state/contract.md` — auto-key-off rejection now lands on empties explicitly

## keyframe-authoring move drags + document-space drawing (landed 2026-09-07, third pass)
- **Auto-key gates move-drag authoring (J 2026-09-09, reconciliation dictation):** the two drag semantics below both exist, switched by the layers panel's AUTO KEY toggle (was dead until now; default OFF):
  - **Auto-key OFF (editing):** a drag on a bar that already carries movement UPDATES that bar in place — no split, no new bar at the playhead (the 09-09 morning dictation). Empties and valueless born-tiled solids still author.
  - **Auto-key ON (authoring):** a drag strictly inside a valued keyframe splits it — left remainder keeps the old offset, an interpolating empty is carved from the left bar's right end (left keeps ≥1 breath), and the keyframe from the drag breath to the old end carries old offset + delta (the 09-07 shape below). This is the path that makes move interpolation exist at all: with one solid bar there is nothing to interpolate between, which is exactly the regression where "move interpolation just clips".
  - `add_move_offset` is the editing seam, `add_move_offset_keyframe` the authoring seam; `commit_move_offset`/`finish_pointer_stroke` carry the panel's auto-key state down (J 2026-09-09).
- **Move drags author keyframes (J 2026-09-07):** `add_move_offset` is the canvas drag commit seam and it no longer just piles deltas onto whatever block covers the playhead — that produced one constant offset (both drags on the same bar) or two ADJACENT bars (a position-to-position clip), so interpolation never showed. The rules now:
  - **Drag strictly inside a solid keyframe** → the bar splits at the drag breath; the new keyframe (bar from the drag breath to the old bar's end) carries old offset + delta; an interpolating empty opens between the two bars, carved from the LEFT bar's right end (which keeps at least one breath; carve width = half the left remainder, min 1). Two drags at two breaths now produce visible motion by themselves.
  - **Drag inside an empty** → the empty's left part STAYS empty (the interpolation region survives) and a new keyframe lands at the drag breath carrying the resolved offset + delta. Dragging mid-transition no longer swallows the whole empty into one solid.
  - **Drag at a keyframe's own start breath** → accumulates in place; repeated drags at one breath stay one keyframe.
  - **Drag onto a valueless born-tiled solid** → takes the value in place (it is not a real keyframe yet; splitting it would strand a valueless bar that resolves to nothing).
  - With no covering block at all, a keyframe spanning the layer's timing window is created (unchanged).
- **Drawing aims in document space (J 2026-09-07):** the entrypoint's `to_world` seam (and the stamp-hover anchor, the only other cursor→world site) subtracts the active layer's move offset at the current breath, so every tool — strokes, lasso, selection, stamp, text, bounds eligibility — aims at the cell that renders back under the cursor once the render path re-applies the move shift. One seam on purpose: no per-tool fixes. The offset is constant within a frame, so move-drag deltas are unaffected; zero move changes nothing.
- Known deferred edge: a drag landing exactly on an empty's FIRST breath converts that whole empty to a keyframe (pre-existing in-place conversion) rather than splitting — revisit if it bites.
- **Drag past the stored extent (in the infinite region) lands a one-breath keyframe at the drag breath** carrying the resolved offset + delta (J 2026-09-07 bug fix): previously it pushed a span solid over the whole layer window, which the retile trimmed over the existing bars and VAPORIZED later keyframes. The old tail blank merges with the retile's gap fill, and a fresh trailing representative is appended after the new keyframe. Authoring a keyframe inside a loop region strips the stranded loop mode per the edge lock (only first/last blanks carry loop modes).
