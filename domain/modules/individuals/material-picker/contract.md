# /home/j/Repos/thaum-painter/domain/modules/individuals/material-picker

## purpose
Own painter's material palette panel so left and right hands can choose material-backed colors separately from direct sprite RGB colors.

## owns
- the `MaterialPickerModule` type
- painter-facing material list layout and hit-testing
- left/right click assignment from a chosen material into hand color state
- material-band preview swatches for each material row

## does not own
- live hand state, owned by `/home/j/Repos/thaum-painter/domain/painter-session/tool-state/`
- renderer material definitions or band resolution, owned by `/home/j/Repos/thaum-renderer/domain/cell-materials/`
- generic panel chrome or gizmo behavior, owned by `/home/j/Repos/thaum-renderer/domain/modules/shared/`

## children-encapsulations
- none

## contents
- `material_picker_module.rs`
  - `MaterialPickerModule` and painter material assignment UI

## dependencies
- `/home/j/Repos/thaum-painter/domain/painter-session/paint-color/`
- `/home/j/Repos/thaum-painter/domain/painter-session/tool-state/`
- `/home/j/Repos/thaum-renderer/domain/cell-materials/`
- `/home/j/Repos/thaum-renderer/domain/modules/`

## exposed interfaces
- `MaterialPickerModule::new(id, rect, tool_state)`

## interface consumers
- `/home/j/Repos/thaum-painter/orchestration/entrypoint/`

## artifacts
- none

## tests
- inline `#[cfg(test)]` in `material_picker_module.rs`
  - light
  - validates left/right material assignment

## data
- none
