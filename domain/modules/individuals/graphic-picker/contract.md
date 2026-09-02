# /home/j/Repos/thaum-painter/domain/modules/individuals/graphic-picker

## purpose
Own painter's graphic picker panel: one painter-facing module for choosing either glyph graphics or sprite graphics into the live left/right hand state.

## owns
- the `GraphicPickerModule` type
- the picker presentation for sprite choices and glyph choices
- left/right click assignment from picker hits into live painter `tool-state`
- glyph section ordering sourced from Thaum Mono's supported-character sections
- the space-glyph display placeholder while still assigning the real `' '` glyph

## does not own
- the renderer's `Module` contract or registry semantics, owned by `thaum-renderer/domain/modules/`
- generic picker chrome or gizmo behavior, owned by `thaum-renderer/domain/modules/shared/`
- the live hand state itself, owned by `domain/painter-session/tool-state/`
- sprite atlas decode/render behavior, owned by `thaum-renderer/domain/cell-graphic/` and `domain/atlas-intake/`

## children-encapsulations
- none

## contents
- `graphic_picker_module.rs`
  - `GraphicPickerModule`, supported glyph-section parsing, and picker draw/hit behavior

## dependencies
- `/home/j/Repos/thaum-painter/domain/painter-session/tool-state/`
- `/home/j/Repos/thaum-renderer/domain/modules/`
- `/home/j/Repos/thaum-renderer/domain/cell-graphic/`
- `/home/j/Repos/thaum-renderer/orchestration/renderer-assets/cell-sprites/monothaum-atlas-v3/sections.txt`

## exposed interfaces
- `GraphicPickerModule::new(id, rect, tool_state)`
  - build one painter-bound graphic picker over shared live tool-state

## interface consumers
- `/home/j/Repos/thaum-painter/orchestration/entrypoint/`

## artifacts
- none

## tests
- inline `#[cfg(test)]` in `graphic_picker_module.rs`
  - light
  - validates glyph-section parsing plus left/right glyph and sprite assignment

## data
- none

## notes
- source-of-truth from J: "Ideally we see both glyphs and sprites."
- source-of-truth from J: "I like my glyphs sorted by asci section."
- source-of-truth from J: "We have those ASCII sections in thaum mono. Use the supported characters."
- glyphs stay their own section inside this picker even though the live hand state now stores one shared `graphic` slot that can be either glyph or sprite.
