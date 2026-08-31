# module concept audit

## purpose
Capture why `module` is a missing structural concept in the current `domain/file/` and `domain/rendering/render-space/` shapes, sourced from `thaum-renderer`'s own design truths and the old painter's `module_position_storage.ts`, and record the correction made to the saved-file and render-space examples.

## source evidence
- `/home/j/Repos/thaum-renderer/context/statement-truths.md` (question-2/truth-2, and the two later truths about module panning and cell-group-local coordinates)
- old file: `src/ascii_painter/module_position_storage.ts` (tracked module placement as its own concern, separate from `painter_document.ts`)

## what the renderer truths actually say
- truth-2: a `CellGroup` is "a render space layer / grouping. intended for an entire module and relevant contents, one menu piece, one encapsulated display module ... the game / ascii painter will handle their own grouping of intra module groups. this is mostly just a rendering piece."
- "modules are what will be used to link a cell group. i dont think the renderer has to know about a module."
- "local panning is owned by the general module features encapsulation which should outline a repeatable module usecase."
- cell-group has its own local coordinate system, "more used by local users of the cell groups / modules," distinct from global world coordinates.

## the gap this exposed
`domain/file/manifest/`'s saved shape (v1 example) modeled one flat `document.groups[]` list with no module wrapper, and each group carried its own top-level `placement`. `domain/rendering/render-space/`'s v1 example mapped each group 1:1 to its own top-level renderer `cell_group` — i.e. three groups produced three separate cell-groups.

That contradicts the renderer's own truth: **one module should become one renderer `CellGroup`**, and the groups inside a module are intra-module content the painter composites before handoff, not independent top-level renderer groups. The old painter also tracked module position as its own concern (`module_position_storage.ts`), separate from document/group storage — confirming module identity and module-level placement were never the same thing as a group.

## correction landed
- `domain/file/manifest/`: `document.groups[]` is now nested one level deeper, under `document.modules[]`. Each module owns identity (`id`, `name`), visibility/lock/opacity, module-level `placement` (position on the shared board — what `module_position_storage.ts` used to own), and its own `group_order`/`groups[]`.
- groups keep everything they already owned (raster segments, properties, per-group timing, visibility/lock/opacity) but their `placement` is renamed `local_placement` and is now explicitly relative to the owning module's local coordinate space, mirroring `thaum-renderer/domain/cell-group/local-coordinate-space/`.
- `domain/rendering/render-space/`: one `cell_groups[]` entry is now emitted per **module**, not per group. Its `placement` comes from the module's global placement; its `cells` are the flattened, composited result of that module's intra-module groups after local-placement offset and timing/property resolution — matching "the game / ascii painter will handle their own grouping of intra module groups."
- the v1 example now carries two modules (`module_cavern_sign`, `module_torch_marker`) to prove the shape actually supports more than one independently-positioned module on the same board, not just a single-module document wearing a new wrapper key.

## ownership after this correction
- `domain/file/manifest/`: canonical saved module identity, module order, and module-level (global) placement.
- `domain/painter-document/modules/`: new editing-facing sibling to `groups/`/`timing/`/`properties/` — module lookup/ordering/placement views over saved truth, same pattern as the existing document children.
- `domain/rendering/render-space/`: module-to-cell-group mapping and intra-module group compositing.
- `thaum-renderer/domain/cell-group/`: unchanged — it still only has to know about `CellGroup`, never "module." That boundary holds exactly as truth-2 says it should.

## still open
- whether module-level `placement` should be authored directly in thaum-painter, or mostly assigned by whatever composes multiple modules onto a shared board (possibly a future game-side concern) is not settled. For now the saved file keeps it as authored, editable state, since the old painter authored it directly (`module_position_storage.ts` lived inside the painter, not the game).
- "general module features encapsulation" (the truth's phrase) as a cross-repo shared concept is not built. `domain/painter-document/modules/` covers thaum-painter's own editing-facing need; whether a shared module-usecase seam should live somewhere both painter and game can consume is future work, not decided here.
