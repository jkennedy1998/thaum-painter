# thaum-painter/domain/modules/individuals/paint-color-block

## purpose
Own painter's binding for the generic renderer color-block picker so the live file palette drives the visible RGB field's banding, and dragged or wheel-scrolled selection writes into left/right hand color state.

## owns
- the `PaintColorBlockModule` type
- left/right hand routing for color-block interactions
- syncing the renderer color block from the active hand's current color preview
- draw-time selection highlighting: color cells matching either hand's live color draw at weight 3, all other color cells at weight 1, resolved from tool state each draw so the highlight never goes stale
- draw-time palette re-banding: each pass re-reads tool-state's live file palette and re-bands the field when it moved, so banding auto add/subtracts as the user edits the palette through the palette module (J 2026-10-10) — the user's wheel-scrolled color stays where they left it
- the clamp widget flow (J 2026-10-11): the block's own clamp widget toggles index clamping; with clamping off, a raw off-palette pick carries straight to the hand, so the palette module's `+` (add hand color) can commit a custom color and users can paint colors not on the palette
- opting the renderer color block into tool-state's live file palette so the field is visibly banded
- committing dragged or wheel-scrolled RGB updates into live painter tool-state

## does not own
- generic HSV color-block drawing or drag math, owned by `thaum-renderer/domain/modules/individuals/color-block/`
- the live file palette state and its edit seams, owned by `thaum-painter/domain/painter-session/tool-state/`

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
- `PaintColorBlockModule::clamping()` — whether the block currently snaps picks to the indexed palette (the toggle itself lives on the block's clamp widget)

## interface consumers
- `thaum-painter/orchestration/entrypoint/`

## artifacts
- none

## tests
- inline `#[cfg(test)]` in `paint_color_block_module.rs`
  - light
  - validates click and drag assignment into painter hand color state
  - validates weight-3 highlighting of hand-matched color cells and weight-1 for the rest
  - validates draw re-bands the block after a file palette add and a file palette remove (a stale block cannot band onto the entry; the re-banded block holds the pick exactly on it; a removed entry stops banding)

## data
- none

## notes
- source-of-truth from J 2026-10-10: the color block automatically adds and subtracts its banded colors as the user changes the file palette through the palette module; the palette module is the single editing interface.
- source-of-truth from J 2026-10-11 (clamp widget): the color block gains a widget toggling index clamping on and off — unclamped, the user can pick colors outside the palette, paint with them naturally, and press `+` to add the custom color; the free-pick flow runs through the color block because it is the module getting ongoing UX polish.
- the wrapper re-bands in `draw()` (the `Module` trait fixes `draw(&self)`, so the re-band rides the pass instead of a separate event).
