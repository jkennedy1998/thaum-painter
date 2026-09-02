# /home/j/Repos/thaum-painter/domain/painter-session/paint-color

## purpose
Own painter's live color choice shape so hand state and painted cells can carry either a direct RGB color or a material selection without leaking renderer cell-color details everywhere.

## owns
- the `PaintColor` type
- conversion from painter color choice into renderer `CellColor`
- painter-side preview rgb for UI swatches and labels

## does not own
- live tool-state ownership, owned by `/home/j/Repos/thaum-painter/domain/painter-session/tool-state/`
- renderer material definitions, owned by `/home/j/Repos/thaum-renderer/domain/cell-materials/`
- renderer cell color resolution, owned by `/home/j/Repos/thaum-renderer/domain/cell-color/`

## children-encapsulations
- none

## contents
- `paint_color.rs`
  - `PaintColor` and conversion helpers

## dependencies
- `/home/j/Repos/thaum-renderer/domain/cell-color/`
- `/home/j/Repos/thaum-renderer/domain/cell-materials/`

## exposed interfaces
- `PaintColor::flat_rgb(red, green, blue)`
- `PaintColor::material(material)`
- `PaintColor::to_cell_color()`
- `PaintColor::preview_rgb()`
- `PaintColor::label()`

## interface consumers
- `/home/j/Repos/thaum-painter/domain/painter-session/tool-state/`
- `/home/j/Repos/thaum-painter/domain/painter-operations/`
- painter UI modules needing a live color preview

## artifacts
- none

## tests
- inline `#[cfg(test)]` in `paint_color.rs`
  - light
  - validates flat and material conversion behavior

## data
- none

## notes
- source-of-truth from J: painter color choice should grow cleanly to cover both indexed colors and material palettes.
