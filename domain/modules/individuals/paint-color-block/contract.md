# thaum-painter/domain/modules/individuals/paint-color-block

## purpose
Own painter's binding for the generic renderer color-block picker so the old 37-color indexed palette drives the visible RGB field and wheel-scrolled hue selection writes into left/right hand color state.

## owns
- the `PaintColorBlockModule` type
- left/right hand routing for color-block interactions
- syncing the renderer color block from the active hand's current color preview
- draw-time selection highlighting: color cells matching either hand's live color draw at weight 3, all other color cells at weight 1, resolved from tool state each draw so the highlight never goes stale
- opting the renderer color block into painter's old 37-color indexed palette so the field is visibly banded
- committing dragged or wheel-scrolled RGB updates into live painter tool-state

## does not own
- generic HSV color-block drawing or drag math, owned by `thaum-renderer/domain/modules/individuals/color-block/`
- live hand-state ownership, owned by `thaum-painter/domain/painter-session/tool-state/`

## children-encapsulations
- none

## contents
- `paint_color_block_module.rs`
  - painter-bound wrapper over renderer `ColorBlockModule`

## dependencies
- `thaum-painter/domain/painter-session/paint-color/`
- `thaum-painter/domain/painter-session/tool-state/`
- `thaum-renderer/domain/modules/individuals/color-block/`

## exposed interfaces
- `PaintColorBlockModule::new(id, rect, tool_state, palette)`

## interface consumers
- `thaum-painter/orchestration/entrypoint/`

## artifacts
- none

## tests
- inline `#[cfg(test)]` in `paint_color_block_module.rs`
  - light
  - validates click and drag assignment into painter hand color state
  - validates weight-3 highlighting of hand-matched color cells and weight-1 for the rest

## data
- none
