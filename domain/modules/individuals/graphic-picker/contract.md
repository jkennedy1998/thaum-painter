# thaum-painter/domain/modules/individuals/graphic-picker

## purpose
Own painter's graphic picker panel: one painter-facing module for choosing either glyph graphics or sprite graphics into the live left/right hand state.

## owns
- the `GraphicPickerModule` type
- the picker presentation for sprite choices and glyph choices
- left/right click assignment from picker hits into live painter `tool-state`
- glyph section ordering sourced from Thaum Mono's supported-character sections; parsing must stay in lockstep with the renderer's sprite-section parser (`glyph_graphic.rs::parse_glyph_sprite_sections`) — a section title may sit on its own line or be glued to its glyph line (`borders:━┃…`), and a stricter parser here silently drops whole sections from the picker
- the space-glyph display placeholder while still assigning the real `' '` glyph
- wheel scrolling of the picker's content: the module renders only the rows visible inside its bounds, `on_wheel` moves a row-offset (clamped at both content edges), and hits exist only for visible rows so cropped content can't be clicked
- vertical layout mirrors the Flat2d HUD orientation (larger local y renders higher on screen): RECENT is pinned at the screen-top end, and the scroll list walks from the screen-top down
- one scrollable category list: the glyph sections (titles above their glyphs) followed by a standard `SPRITES` section with its own title — sprites are not a separated pinned block
- a pinned RECENT section locked to the top of the module (header + one row of the last assigned graphics, most-recent-first, deduped, capped at 10) that stays visible while the glyph list scrolls under it; clicking a recent slot re-assigns that graphic to the clicking hand; fed by draw-time observation of both hands so every assignment path lands there

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
- `thaum-painter/domain/painter-session/tool-state/`
- `thaum-renderer/domain/modules/`
- `thaum-renderer/domain/cell-graphic/`
- `thaum-renderer/orchestration/renderer-assets/cell-sprites/monothaum-atlas-v3/sections.txt`

## exposed interfaces
- `GraphicPickerModule::new(id, rect, tool_state)`
  - build one painter-bound graphic picker over shared live tool-state

## interface consumers
- `thaum-painter/orchestration/entrypoint/`

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
- source-of-truth from J: the picker should scroll up and down with its content cropped inside the module bounds, so the panel can be made smaller and the user can still look for characters.
- source-of-truth from J: the horizontal glyph layout must expand and contract with horizontal module resizing (rows re-chunk at the panel's content width).
- source-of-truth from J: the scroll list is one set of standard categories — sprites and glyphs are not separated in this menu, only RECENT stays a pinned section, and it must sit at the top of the module on screen.
- source-of-truth from J (painter-wide, owned by `domain/painter-session/tool-state/`): if any of the three edit channels (graphic/color/weight) is locked, strokes and stamps must not place on empty cells — only an all-unlocked hand authors new cells; locked-channel partial edits still restyle existing cells in place.
- the recent list is module-local and not persisted across restarts; if J wants recall to survive a session, `PersistedModuleUiState` is the seam to extend.
- scroll sits in this module (wheel dispatch itself is renderer-owned via `Module::on_wheel` / `registry.dispatch_wheel_at`); if a second scrolling panel appears, the row-offset/crop logic should graduate into `thaum-renderer/domain/modules/shared/`.
