# thaum-painter/domain/modules/individuals/paint-color-picker

## purpose
Own painter's indexed color picker binding: a painter-facing module that reuses the renderer's generic `ColorPickerModule`, swaps in painter's old 37-color indexed palette, and writes clicks into live painter `tool-state` so left/right mouse buttons actually get separate flat RGB colors.

## owns
- the `PaintColorPickerModule` type
- the bridge from left/right swatch clicks into left/right hand flat-RGB color state
- honoring per-hand color locks while a swatch is chosen

## does not own
- generic swatch-grid drawing, gizmo behavior, or palette ordering, owned by `thaum-renderer/domain/modules/individuals/color-picker/`
- live hand settings ownership, owned by `domain/painter-session/tool-state/`
- boot placement in the app window, owned by `orchestration/entrypoint/`

## children-encapsulations
- none

## contents
- `paint_color_picker_module.rs`
  - painter-bound wrapper over renderer `ColorPickerModule`

## dependencies
- `thaum-painter/domain/painter-session/tool-state/`
- `thaum-renderer/domain/modules/individuals/color-picker/`

## exposed interfaces
- `PaintColorPickerModule::new(id, rect, tool_state, palette)` — build one painter-bound color picker over shared live tool-state

## interface consumers
- `thaum-painter/orchestration/entrypoint/`

## artifacts
- none

## tests
- inline `#[cfg(test)]` in `paint_color_picker_module.rs`
  - light
  - validates left/right clicks write into the matching hand color and color locks are respected

## data
- none

## notes
- source-of-truth from J: the old painter had left/right mouse button color ownership and that single source of truth should come back in the new repo.
- this wrapper now specifically owns the indexed sprite-color side of that, while material picking and free RGB picking live in separate painter modules.
- this wrapper exists so painter keeps authoring semantics local instead of teaching the renderer's generic color picker about painter session state.
