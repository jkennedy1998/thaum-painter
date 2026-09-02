# /home/j/Repos/thaum-painter/domain/modules/individuals/paint-color-block

## purpose
Own painter's binding for the generic renderer color-block picker so the old 37-color indexed palette drives the visible RGB field and wheel-scrolled hue selection writes into left/right hand color state.

## owns
- the `PaintColorBlockModule` type
- left/right hand routing for color-block interactions
- syncing the renderer color block from the active hand's current color preview
- opting the renderer color block into painter's old 37-color indexed palette so the field is visibly banded
- committing dragged or wheel-scrolled RGB updates into live painter tool-state

## does not own
- generic HSV color-block drawing or drag math, owned by `/home/j/Repos/thaum-renderer/domain/modules/individuals/color-block/`
- live hand-state ownership, owned by `/home/j/Repos/thaum-painter/domain/painter-session/tool-state/`

## children-encapsulations
- none

## contents
- `paint_color_block_module.rs`
  - painter-bound wrapper over renderer `ColorBlockModule`

## dependencies
- `/home/j/Repos/thaum-painter/domain/painter-session/paint-color/`
- `/home/j/Repos/thaum-painter/domain/painter-session/tool-state/`
- `/home/j/Repos/thaum-renderer/domain/modules/individuals/color-block/`

## exposed interfaces
- `PaintColorBlockModule::new(id, rect, tool_state, palette)`

## interface consumers
- `/home/j/Repos/thaum-painter/orchestration/entrypoint/`

## artifacts
- none

## tests
- inline `#[cfg(test)]` in `paint_color_block_module.rs`
  - light
  - validates click and drag assignment into painter hand color state

## data
- none
