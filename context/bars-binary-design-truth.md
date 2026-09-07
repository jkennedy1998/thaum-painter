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
- Interim behavior: a playhead over an **empty displays nothing** (stub). Real interpolation is a later pass.
- Future interpolation shape: each property row has its **own custom interpolation** (you cannot interpolate raster like you interpolate move). Empties fire in **one standard way**; the per-row difference lives in how a row's content resolves. The user will eventually set the interpolative mode of empty bars — that UX is not worked out yet.

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
  - L-dblclick: duplicate this single frame to the right (same non-destructive push as center duplicate); lands on the right, growing the layer span by exactly the duplicate's length when it would pass the span end (bounded — never auto-fills)
  - R-dblclick: replace with empty (this is the new delete — empties are voids now)
- **left head**
  - L-click: select the layer
  - R-click: unused
  - L-drag: positional drag preserving the timing of bars around it (pushed ripple, resizes this bar's start edge); dragging left pushes content left, dragging right pulls content in from the left. Preview of every affected bar plus the dragged bar.
  - R-drag: positional drag that does NOT preserve surrounding timing — overwrites blocks under without moving them (crop, or delete if fully encompassed); dragged right it leaves an empty in the wake
  - L-dblclick: the bar on the left merges into this one; the double-clicked bar's content stays — implement as destructive resize to the left bar's start position `(J-confirmed via ruling 11; implemented 2026-09-07 as a destructive timing commit, right heads mirror, span-edge rejects)`
  - R-dblclick: unused
- **right head**: exact mirror of left head
- **center**
  - L-click: select the layer
  - R-click: unused
  - L-drag: swaps the actual keyframe content with the bar the drag lands on (the existing swap seam: spans exchange, each block's content follows it, so the two keyframes trade time positions)
  - R-drag: slides the bar destructively, overwriting content it lands on, leaving an empty in its previous span
  - L-dblclick: duplicate this frame to the right of this one (whole bar span), non-destructive — shifts all other bars right so it has room to land
  - R-dblclick: replace the block span with empties

### blank bar (empty / 0 / null of the binary system)
- Global invariant: empties are **never adjacent to other empties** — they merge into one.
- **empty single**
  - L-click / R-click / L-drag / L-dblclick: unused (but every interaction still selects the layer first — see selection rule)
  - R-drag: resize, overwrites whatever it expands over (destructive, victims become empty)
  - R-dblclick: merge this empty into the adjacent content block — same seam as empty center: prefer the left side, fall back to the right, reject when the track has no content at all
- **left empty head**
  - L-click / R-click / L-dblclick: unused (J-confirmed) — but every interaction still selects the layer first
  - L-drag: positional drag resizing this empty the same time-preserving (pushed) way as a solid left head — same code
  - R-drag: same destructive way as a solid left head
  - R-dblclick: merge this empty into the block on the left, keeping that block's content (content survives instead of the empty — the inverse of the solid left-head merge)
- **right empty head**: exact mirror of left empty head
- **empty center**
  - L-click / R-click / R-drag / L-dblclick: unused (but every interaction still selects the layer first)
  - L-drag: swaps keyframe content exactly like solid center (predictability)
  - R-dblclick: merge this empty into the adjacent content block — **J-corrected 2026-09-07: used, not unused**. One seam shared with empty single: find a side with content preferring the left, fall back right, reject when there is no content on either side (fully-empty track).

## rulings from J (2026-09-07, answering the inconsistency list)
1. Empty center R-dblclick = **merge** (dictation slip; it is used).
2. Left empty head L-dblclick = **unused**.
3. Empty merges (single + center, both heads mirrored) are **one seam**: find a side with content, prefer left, fall back right, **reject when there is no content on either side**.
4. Content single L-dblclick duplicate = **same as center duplicate**: non-destructive push, fewer distinct outcomes for the user to expect.
5. Duplicate at the span end: **place the new block next to it, on the right** — duplicates always land on the right. Span growth is bounded (exactly the duplicate's length, one interaction at a time — never fills to infinity). Timeline length is per-layer `length_breaths` (default 24); the document window is separate.
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
