# project-plan — file-index-palette (file truth vs user truth)

<!-- project home: thaum-painter repo. canonical file location: plans/project-<slug>-plan.md -->

## implementation-rules
- update checklist states for this plan directly while working.
- use tests/ in the encapsulation we are working on to put testing
- the rules of the general encapsulation design can be found in jobo/domains/encapsulations/
- a plan's bounds end at development done within other encapsulations
- a plan should not feature development of a child encapsulation, the child encapsulation should own its own plans
- phase-1 is always project architecture alignment, touched-encapsulation declaration, and work ordering
- before implementation engagement, the project-manager and user should refine the project plan and linked encapsulation plans until approved
- middle phases should be one phase per touched encapsulation, ordered by dependency
- each encapsulation phase should build the planning/handoff shape for that encapsulation
- the last phase is always final spec-built verification, final iterative encapsulation-checker or equivalent repo-rule review across touched encapsulations, project-scope tests, and git commit if approved
- if work intentionally stops before commit, leave a compact note in the active plan describing what landed, what is still dirty, and what should happen before git.

## status key
- `[ ]` not done
- `[+]` implemented
- `[#]` tested

## pre-implementation-note
The painter currently hardcodes the legacy 37-color indexed palette in `domain/painter-session/paint-color/indexed_palette.rs`; the palette picker wrapper and color-block wrapper both pass that constant to the renderer's generic modules, and indexed raster interpolation resolves through the hardcoded `nearest_indexed_rgb()`. J settled the design truths (recorded in repo `truth.md`): the split is **file truth vs user truth** — not "dynamic vs static". Cells keep storing raw RGB (never palette indices) so future depth like tolerance fill stays possible. The file's index palette becomes a saved ordered color list, defaulting to the legacy 37, and the palette module becomes the single interface users edit it through (hex/RGB input + plus button; minus removes without rewriting cells). The color block auto add/subtracts its banding as the palette changes. Palette switching and extra presets are explicitly later — the shipped default palette is everyone's starting point. Per-file drawing background color is out of scope for this plan (separate work).

## affected-encapsulations
- `domain/file/file-schema/`: edit
- `domain/file/storage/`: edit
- `domain/painter-session/paint-color/`: edit
- `domain/painter-session/tool-state/`: edit
- `domain/painter-document/properties/`: edit
- `domain/modules/individuals/paint-color-picker/`: edit
- `domain/modules/individuals/paint-color-block/`: edit

## encapsulation-plans
- `domain/file/file-schema/`: `plans/home-j-repos-thaum-painter-domain-file-plan.md` (extend)
- `domain/file/storage/`: `plans/home-j-repos-thaum-painter-domain-file-storage-plan.md` (extend)
- `domain/painter-session/paint-color/`: `plans/home-j-repos-thaum-painter-domain-painter-session-plan.md` (extend)
- `domain/painter-session/tool-state/`: `plans/home-j-repos-thaum-painter-domain-painter-session-plan.md` (extend)
- `domain/painter-document/properties/`: `plans/home-j-repos-thaum-painter-domain-painter-document-properties-plan.md` (extend)
- `domain/modules/individuals/paint-color-picker/`: `plans/project-thaum-painter-session-ui-plan.md` (extend)
- `domain/modules/individuals/paint-color-block/`: `plans/project-thaum-painter-session-ui-plan.md` (extend)

## encapsulation-details
### `domain/file/file-schema/`
- action: edit
- intent: Own the saved file-truth palette shape: an optional top-level `palette` section (ordered list of RGB colors) on the v3 schema. Absent section → default palette (legacy 37). File version stays v3 — same optional-section precedent as `facing`.
- interface delta: `palette` section parse/validation accepted by the schema gate; palette is an ordered rgb color list (raw `[r,g,b]` triples, no indices, no named roles).
- dependencies: `thaum-painter/domain/painter-session/paint-color/` — default palette constant lives in paint-color; schema references it rather than duplicating the 37.
- consumers: `domain/file/storage/` loads/saves the section; painter-session consumes the parsed list as the live file palette.
- artifacts: none
- tests: schema parse tests in `file_schema.rs` inline tests — palette present/absent, malformed palette rejected, empty list rejected (default applies instead)
- data: none

### `domain/file/storage/`
- action: edit
- intent: Persist the palette section on save and hand the parsed palette to painter-session state on load. Cells keep storing raw RGB — storage does not remap cell colors when the palette changes.
- interface delta: save includes the file palette when it differs from/was edited away from default; load exposes the palette to session state.
- dependencies: `thaum-painter/domain/file/file-schema/` — schema gate for the section.
- consumers: painter-session state building.
- artifacts: none
- tests: storage round-trip test — save a file with a user-edited palette, reload, palette matches; cell RGB untouched by palette edits
- data: none

### `domain/painter-session/paint-color/`
- action: edit
- intent: Reposition the palette as the per-file color list instead of a global constant. `LEGACY_INDEXED_PALETTE` survives as the shipped default palette preset. `nearest_indexed_rgb()` takes the palette as a parameter — no global hidden palette.
- interface delta: `nearest_indexed_rgb(rgb, palette)`; default palette accessor; palette add/remove helpers (add appends unless duplicate, remove drops the entry without touching cells).
- dependencies: unchanged (`thaum-renderer/domain/cell-color/`, `thaum-renderer/domain/cell-materials/`)
- consumers: tool-state (live palette), `painter-document/properties/` (interp), both picker wrappers
- artifacts: none
- tests: inline tests — parameterized nearest resolution, add dedupes, remove leaves cell colors alone
- data: none

### `domain/painter-session/tool-state/`
- action: edit
- intent: Hold the live file palette in session state: loaded from the file on open, edited through the palette module, written back on save. Hand colors stay raw RGB.
- interface delta: `file_palette()` read access, `add_file_palette_color(rgb)`, `remove_file_palette_color(rgb)`; palette change is the single event the color block reacts to.
- dependencies: `thaum-painter/domain/painter-session/paint-color/`
- consumers: `paint-color-picker/`, `paint-color-block/`, storage save path
- artifacts: none
- tests: inline tests — load-from-file, add/remove round trips, palette survives session continuity
- data: none

### `domain/painter-document/properties/`
- action: edit
- intent: `interp_raster`'s indexed color blend resolves against the file's live palette (via parameterized `nearest_indexed_rgb`) instead of the hardcoded 37.
- interface delta: none (consumer-side call-site update)
- dependencies: `thaum-painter/domain/painter-session/paint-color/`
- consumers: none new
- artifacts: none
- tests: existing interp tests updated to run against a supplied palette, including a non-default palette
- data: none

### `domain/modules/individuals/paint-color-picker/`
- action: edit
- intent: The palette module becomes the single interface for editing the file's index colors. It reads the live file palette for swatches and gains an entry row: an RGB or hex input plus a `+` gizmo that commits the entered color to the file palette. A `−` gizmo removes the selected swatch from the palette — it does NOT replace or rewrite painted cells (out of scope by decision).
- interface delta: `PaintColorPickerModule` constructor takes the tool-state live palette (via tool-state) instead of hardcoding `legacy_indexed_palette()`; new entry-row gizmos (`+` add from hex/rgb field, `−` remove selected).
- dependencies: `thaum-painter/domain/painter-session/tool-state/`, `thaum-renderer/domain/modules/individuals/color-picker/`
- consumers: `orchestration/entrypoint/`
- artifacts: none
- tests: inline tests — swatches reflect live file palette, `+` appends entered hex/rgb, `+` dedupes, `−` removes selected without mutating any cell color
- data: none

### `domain/modules/individuals/paint-color-block/`
- action: edit
- intent: The color block consumes the live file palette for its banding and automatically adds/subtracts banded colors as the user changes the palette through the palette module. Free wheel/RGB selection UX is unchanged; UI (user-truth) colors keep their current behavior and users can still enter new free colors there.
- interface delta: constructor consumes tool-state's live file palette instead of `legacy_indexed_palette()`; re-reads palette each draw so banding follows palette edits with no extra wiring.
- dependencies: `thaum-painter/domain/painter-session/tool-state/`, `thaum-renderer/domain/modules/individuals/color-block/`
- consumers: `orchestration/entrypoint/`
- artifacts: none
- tests: inline tests — banding follows a palette add and a palette remove; hand-color sync unchanged
- data: none

## phases
### phase-1 — architecture alignment + encapsulation ordering
- [x] identify the touched encapsulations
- [x] confirm each touched encapsulation action is add, edit, delete, or new
- [x] record exposed-interface deltas for touched encapsulations
- [x] record dependencies and consumer expectations for touched encapsulations
- [x] note which touched encapsulations need a dedicated encapsulation-local plan
- [x] order the project work in the smallest correct sequence
- ordering: paint-color first (palette shape + parameterized nearest) → tool-state (live palette state) → file-schema + storage (save/load) → paint-color-picker (editing UI) → paint-color-block (auto banding) → properties (interp call-site) → final sweep

### phase-2 — plan encapsulation: `domain/painter-session/paint-color/`
- [x] confirm the encapsulation path/name and action are correct for this phase
- [x] confirm the encapsulation-local plan file path is declared
- [x] record the encapsulation intent and interface delta for handoff
- [x] record dependency, consumer, artifact, test, and data expectations for handoff
- [x] record why this encapsulation is ordered here relative to the other touched encapsulations
- rationale: the palette shape and parameterized resolution are the foundation every other touched boundary consumes

### phase-3 — plan encapsulation: `domain/painter-session/tool-state/`
- [x] confirm the encapsulation path/name and action are correct for this phase
- [x] confirm the encapsulation-local plan file path is declared
- [x] record the encapsulation intent and interface delta for handoff
- [x] record dependency, consumer, artifact, test, and data expectations for handoff
- [x] record why this encapsulation is ordered here relative to the other touched encapsulations
- rationale: live palette session state is the seam both UI wrappers and storage read/write

### phase-4 — plan encapsulation: `domain/file/file-schema/` + `domain/file/storage/`
- [x] confirm the encapsulation path/name and action are correct for this phase
- [x] confirm the encapsulation-local plan file path is declared
- [x] record the encapsulation intent and interface delta for handoff
- [x] record dependency, consumer, artifact, test, and data expectations for handoff
- [x] record why this encapsulation is ordered here relative to the other touched encapsulations
- rationale: file truth lands once the in-memory shape exists; schema stays v3 with an optional section

### phase-5 — plan encapsulation: `domain/modules/individuals/paint-color-picker/`
- [x] confirm the encapsulation path/name and action are correct for this phase
- [x] confirm the encapsulation-local plan file path is declared
- [x] record the encapsulation intent and interface delta for handoff
- [x] record dependency, consumer, artifact, test, and data expectations for handoff
- [x] record why this encapsulation is ordered here relative to the other touched encapsulations
- rationale: the palette module is the single editing interface; UI lands after file truth exists

### phase-6 — plan encapsulation: `domain/modules/individuals/paint-color-block/` + `domain/painter-document/properties/`
- [x] confirm the encapsulation path/name and action are correct for this phase
- [x] confirm the encapsulation-local plan file path is declared
- [x] record the encapsulation intent and interface delta for handoff
- [x] record dependency, consumer, artifact, test, and data expectations for handoff
- [x] record why this encapsulation is ordered here relative to the other touched encapsulations
- rationale: block banding and interp are consumers of the now-live file palette

### phase-7 — final verification + repo-rule sweep + git commit
- [x] verify every touched encapsulation has a linked encapsulation plan
- [x] verify every touched encapsulation detail block includes action, intent, interface delta, dependencies, consumers, artifacts, tests, and data
- [x] verify the built project matches the planned specs
- [x] run final iterative encapsulation-checker or equivalent repo-rule verification for each touched encapsulation
- [x] run standard tests for the project scope (cargo test across touched encapsulations)
- [ ] git commit if approved (awaiting J)

## post-implementation-notes
- plan-finished: true
- encapsulation-git-commit: false
- landed: parameterized `nearest_indexed_rgb(rgb, palette)` + file-truth palette shape (`default_file_palette`, add/remove bookkeeping, hex parse/format, `HexColorEdit`) in `paint-color/`; live file palette in `tool-state/` (`file_palette()`/`set_file_palette`/add/remove, empty falls back to default); document palette section in `storage/` (`SharedDocumentPalette` on the runtime document, skip-while-default, cells keep raw RGB, schema v3 untouched — the optional-section precedent, palette rides `document.json` instead of the canonical gate); palette module edit strip in `paint-color-picker/` (swatches from live palette, `−`/`+`, in-place hex entry committed by the host typing seam); draw-time re-banding in `paint-color-block/`; interp + smear + layers runtime thread the file palette.
- tests added late in the build: tool-state palette round trips, block re-band after add/remove (stale vs re-banded pick contrast), interp non-default-palette blend. Workspace green: painter domain 518, renderer domain 404, entrypoint 28, workers 58.
- renderer-side runtime seam (uncommitted in thaum-renderer): `ColorPickerModule::set_indexed_palette` + `indexed_palette()` read view, `ColorBlockModule::set_indexed_palette` (selection-preserving); both contracts updated. Commit both repos together.
- dirty before git: both repos hold the full palette pass, contracts + plan updated, truth.md carries the three 2026-10-10 palette truths. Next step: J-approved commits for thaum-painter and thaum-renderer.

## follow-up-phase — 2026-10-11 palette module usability pass (J feedback)
J's first live pass showed: removing colors worked, adding colors did not (the `+` only re-added the hand's color, which comes from existing swatches — a user cannot add a color they do not hold), the edit strip floated below the panel, and the click-to-front swatch reordering made the list annoying. This pass is folded into the same touched encapsulations (`paint-color/`, `paint-color-picker/`, renderer `color-picker/`) plus the entrypoint typing seam.
- [+] typed entry: linked hex field + rgb field (changing one rewrites the other), `+` commits the entry color; `HexColorEdit` superseded by `ColorEntryEdit` + `ColorEntryField` in `paint-color/color_entry.rs`, `parse_rgb_color` / `rgb_color_string` alongside the hex helpers
- [+] wipe flow: `−` carries the selection to the color taking the removed swatch's display slot (clamped; `ColorPickerModule::select_display_position` in the renderer picker), so repeated `−` wipes in sorted order; the palette never empties — `tool-state/` refuses to remove the last remaining color
- [+] clamp widget on the color block: a `#`/`*` toggle cell on the block's hex readout row flips index clamping (`ColorBlockModule::clamped`/`set_clamped`); unclamped, a raw off-palette pick carries straight to the hand, so the palette module's `+` adds any custom color and users can paint off-palette naturally
- [+] buttons move inside the module: the strip is the panel's lower rows (rgb row + hex row over the inner picker's bottom border, wrapper-drawn bottom border); the floating row below the module is gone
- [+] typing seam: while a color entry is open, a click hitting the palette module is delivered to it (so `+` can commit a typed color) and any other click closes the entry; `Enter` still commits. Second pass (J: typing never worked): the left-click branch never began typing mode — only the right-click branch had the `typing_mode.begin` check — so the entry opened but keys routed nowhere; the check now runs on both click paths, a self-closing edit (Enter/Escape) ends typing mode with it, and `Space`/`,` route into the rgb field as channel separators
- [+] renderer picker: click-to-front MRU order removed, display order derived from the colors (value descending, hue ascending, ties keep palette order) via `display_order()`; selection keyed by raw RGB so palette swaps and re-sorts keep it; overflow swatches clip to the content instead of drawing past the border; `set_rect` / `is_seamless` / `set_seamless` exposed for the wrapper's persisted-state surgery
- [#] tested: `color_entry.rs` (seed, mirror both directions, last-valid preview, caps, replace-on-first-typing, refusal), `indexed_palette.rs` rgb parse/format, renderer picker (derived order, no reorder, selection survival, clipping), wrapper (`+` commits hex and rgb entries, incomplete entry refuses and stays open, field switching, entry closes on non-strip click, strip draws inside the module bounds). Workspaces green: painter 529 + 28 + 58, renderer 407 + 50 + 4 + 3 + 2
- [+] contracts updated: renderer `color-picker/`, painter `paint-color-picker/`, `paint-color/`; truths recorded in both repos' truth.md (2026-10-11)
- [ ] git commit if approved (both repos, awaiting J)

## notes
- naming rule: file truth (saved with the file) vs user truth (per-user UI, hard-set stable RGB). "Dynamic/static" rejected as a name.
- cells always store raw RGB, never palette indices — keeps depth like fill-tolerance open.
- palette switching between lists and additional presets are explicitly later work; the default palette (legacy 37) ships as everyone's starting point.
- minus on a palette color does not replace or rewrite painted cells (out of scope by decision).
- per-file drawing background color + background gizmo is separate work, not in this plan.
