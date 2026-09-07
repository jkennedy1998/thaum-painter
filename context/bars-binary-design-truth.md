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

## consumers of this truth
- `domain/painter-document/properties/contract.md` — binary span/tiling semantics
- `domain/painter-document/properties/interpolation/contract.md` — gap-fill death, per-row interpolation intent
- `domain/file/storage/contract.md` — schema break, tiling invariant, no migration
- `domain/modules/individuals/layers-panel/contract.md` — piece taxonomy, interaction routing, empty/solid UX split
- `domain/painter-session/timeline-state/contract.md` — auto-key-off rejection now lands on empties explicitly
