# thaum-painter/domain/painter-session/paint-color

## purpose
Own painter's portable live color choice shape so hand state and painted cells carry direct RGB, filename-backed material, or A/B/C slot values without leaking renderer cell-color details everywhere.

## owns
- the `PaintColor` type
- the file-truth index palette: an ordered color list that ships as the legacy 37 default and is the single palette every consumer threads (J 2026-10-10 file-truth vs user-truth split)
- palette bookkeeping helpers: append-unless-duplicate add, remove-without-touching-cells, default-vs-edited detection
- nearest-indexed-color resolution, parameterized over the caller's palette — no hidden global palette
- hex and rgb color entry: `RRGGBB` parse (optional leading `#`), canonical `#RRGGBB` display form, decimal rgb parse (`R G B` / `R,G,B` / nine raw digits), canonical `RRR GGG BBB` display form, and the in-place `ColorEntryEdit` typing state whose linked hex and rgb fields mirror each other
- conversion from painter color choice into renderer `CellColor`
- painter-side preview rgb for UI swatches and labels

## does not own
- live tool-state ownership, owned by `thaum-painter/domain/painter-session/tool-state/`
- renderer material definitions, owned by `thaum-renderer/domain/cell-materials/`
- renderer cell color resolution, owned by `thaum-renderer/domain/cell-color/`

## children-encapsulations
- none

## contents
- `paint_color.rs`
  - `PaintColor` and conversion helpers
- `indexed_palette.rs`
  - the file-truth palette shape: `LEGACY_INDEXED_PALETTE` (the shipped default 37), `default_file_palette` / `is_default_file_palette`, parameterized `nearest_indexed_rgb(rgb, palette)`, `palette_push_color` / `palette_remove_color`, and the hex/rgb parse and format helpers
- `color_entry.rs`
  - `ColorEntryEdit` + `ColorEntryField`, the in-place typing state over the palette module's linked hex and rgb entry fields

## dependencies
- `thaum-renderer/domain/cell-color/`
- `thaum-renderer/domain/cell-materials/`

## exposed interfaces
- `PaintColor::flat_rgb(red, green, blue)`
- `legacy_indexed_palette()`
- `default_file_palette()` / `is_default_file_palette(palette)`
- `nearest_indexed_rgb(rgb, palette)` — nearest resolution over the caller-supplied file palette, ties keep palette order
- `palette_push_color(palette, rgb)` / `palette_remove_color(palette, rgb)` — add appends unless duplicate; remove drops the entry without touching painted cells
- `parse_hex_color(text)` / `hex_color_string(rgb)`
- `parse_rgb_color(text)` / `rgb_color_string(rgb)`
- `ColorEntryEdit::begin/push/backspace/commit` + `hex_text` / `rgb_text` / `preview_rgb` — the in-place entry state with linked hex/rgb fields
- `PaintColor::material(material)` / `PaintColor::material_asset(asset_file)`
- `PaintColor::slots(a, b, c)` and `PaintColorSlot`
- `PaintColor::to_cell_color()`
- `PaintColor::preview_rgb()`
- `PaintColor::label()`

## interface consumers
- `thaum-painter/domain/painter-session/tool-state/`
- `thaum-painter/domain/painter-operations/`
- painter UI modules needing a live color preview
- `thaum-painter/domain/painter-document/properties/` for indexed raster interpolation

## artifacts
- none

## tests
- inline `#[cfg(test)]` in `paint_color.rs`
  - light
  - validates flat and material conversion behavior
- inline `#[cfg(test)]` in `indexed_palette.rs`
  - light
  - validates parameterized nearest resolution, add dedupe, remove leaves cell colors alone, default-palette detection, hex parse/format round trips, and rgb parse/format round trips
- inline `#[cfg(test)]` in `color_entry.rs`
  - light
  - validates both-fields seeding, hex→rgb and rgb→hex mirroring, last-valid preview during half-typed entries, buffer caps, replace-on-first-typing, and refusal of incomplete commits

## data
- none

## notes
- source-of-truth from J 2026-10-10 (file truth vs user truth): cells always store raw RGB, never palette indices — palette membership is bookkeeping, so depth like tolerance fill stays open. "Dynamic vs static" was rejected as the name for this split.
- source-of-truth from J 2026-10-10: the shipped default palette is the hardcoded legacy 37 — every user's starting point. Palette switching between lists and extra presets are explicitly later work.
- source-of-truth from J: painter color choice should grow cleanly to cover both indexed colors and material palettes.
- source-of-truth from J 2026-10-11: typed color entry cannot depend on the user already holding the color — the entry takes either a hex value or an rgb value, the two stay linked (changing one changes the other), and `+` adds the color they resolve to into the indexed palette.
- Asset-backed values retain relative filenames in painter state. `to_cell_color()` is a temporary built-in preview bridge until the renderer owns general filename resolution.
