# thaum-painter/domain/modules/individuals/paint-color-picker

## purpose
Own the palette module: painter's single interface for editing the file's index colors (J 2026-10-10). It shows the live file palette as swatches, writes swatch clicks into left/right hand color state, and owns the edit strip inside the panel — a linked hex entry and rgb entry plus `+` to commit the entry color into the file palette and `−` to remove the selected swatch without rewriting any painted cell.

## owns
- the `PaintColorPickerModule` type
- drawing and re-syncing the swatch list from tool-state's live file palette each pass (a palette edit re-seeds the swatches on the next draw)
- the edit strip inside the panel's lower rows: `−` removes the selected swatch (refused on the last remaining color) and carries the selection to the color that takes the removed swatch's display slot, so repeated `−` wipes in sorted order, `+` commits the entry color (the linked hex/rgb fields resolve to one color; with no entry open it adds the active hand's color), the hex field opens the in-place `ColorEntryEdit` on the hex side, the rgb field opens it on the rgb side, and the preview swatch mirrors the color the entry currently resolves to
- the linked-entry click flow while typing owns the input surface: a click on the palette module reaches the module so `+` can commit a typed color, and any other click closes the entry
- the bridge from left/right swatch clicks into left/right hand flat-RGB color state
- honoring per-hand color locks while a swatch is chosen

## does not own
- generic swatch-grid drawing, gizmo behavior, or display ordering, owned by `thaum-renderer/domain/modules/individuals/color-picker/`
- the hex/rgb parse rules and the file palette's shape, owned by `thaum-painter/domain/painter-session/paint-color/`
- the live file palette state itself, owned by `domain/painter-session/tool-state/`
- the host typing seam that routes keys into an open color entry, owned by `orchestration/build-commands/`
- boot placement in the app window, owned by `orchestration/build-commands/`

## children-encapsulations
- none

## contents
- `paint_color_picker_module.rs`
  - painter-bound wrapper over renderer `ColorPickerModule`: live file-palette swatches, the in-panel edit strip (`−` / `+` / linked hex + rgb entry / preview), and the left/right hand click bridge

## dependencies
- `thaum-painter/domain/painter-session/paint-color/`
- `thaum-painter/domain/painter-session/tool-state/`
- `thaum-renderer/domain/modules/individuals/color-picker/`

## exposed interfaces
- `PaintColorPickerModule::new(id, rect, tool_state, palette, color_entry)` — build one palette module over shared live tool-state and the shared color-entry edit slot
- `PaintColorPickerModule::color_entry_is_open()` — whether an in-place color entry is open right now

## interface consumers
- `thaum-painter/orchestration/build-commands/`

## artifacts
- none

## tests
- inline `#[cfg(test)]` in `paint_color_picker_module.rs`
  - light
  - validates left/right clicks write into the matching hand color through the derived swatch order and color locks are respected
  - validates the swatch list re-syncs from the live file palette after a palette edit
  - validates `+` commits the open entry's color (hex or rgb side), refuses an incomplete entry without losing it, adds the hand color with no entry open (deduped), and `−` removes the selected swatch without mutating any painted cell color, carries the selection to the color taking the removed slot (repeated `−` wipes), and is refused on the last remaining color
  - validates the hex/rgb fields open (or switch focus of) the shared entry seeded from the hand color, a non-strip click closes an open entry, and the strip rows draw inside the module bounds

## data
- none

## notes
- source-of-truth from J: the old painter had left/right mouse button color ownership and that single source of truth should come back in the new repo.
- source-of-truth from J 2026-10-10: the palette module should really be the one interface users change the file's index colors through — hex/RGB input plus `+` adds, minus removes without rewriting cells, and the color block auto add/subtracts its banding as the palette changes.
- source-of-truth from J 2026-10-11: the palette module owns adding colors from typed input — the user cannot be relied on to already hold the color in a hand, so both a hex field and an rgb field live inside the panel, linked (changing one rewrites the other), and `+` adds the color they resolve to; the buttons live inside the module, not in a floating row below it.
- source-of-truth from J 2026-10-10: "lets make sure this is built in a standard way and we dont hardcode the bottom into the graphic. we should simply render the content maybe responsibly on the bottom of the module" — the strip renders as content in the bottom rows of the panel's single chrome; the bottom border is chrome-owned, never hand-drawn into the strip graphic.
- `−` on a palette color deliberately does NOT replace or rewrite painted cells (out of scope by decision); painted cells keep raw RGB.
- this wrapper exists so painter keeps authoring semantics local instead of teaching the renderer's generic color picker about painter session state.
- the strip is the bottom two content rows of the panel's single chrome: the inner picker owns the border/background/title (one bottom border, no hand-drawn duplicate) and reserves those rows from its swatch grid via `with_footer_rows`, while the wrapper draws only the strip's own glyphs there. The wrapper no longer draws its own bottom border or shifts the inner rect — that older shape produced two visible bottoms (the inner panel's bottom border peeking through at the hex row plus the wrapper's hand-drawn one below the strip).
- wipe flow from J 2026-10-11: "make deleting colors easier... what if the color selected when we deleted went to the first color? then users can wipe easier. bonus lopoints if it goes to the index of the deleted color instead so i can wipe with more controll" — the carry goes to the removed swatch's display position (the bonus form), so `−` walks the sorted list one color per press.
- the palette never empties: `tool-state/` refuses to remove the last remaining color, so nearest-indexed resolution and the saved file's palette section always hold at least one color.
