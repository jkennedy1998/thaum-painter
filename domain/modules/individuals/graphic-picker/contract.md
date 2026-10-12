# thaum-painter/domain/modules/individuals/graphic-picker

## purpose
Own painter's graphic picker panel: one painter-facing module for choosing glyph, sprite, or sided graphics into the live left/right hand state.

## owns
- the `GraphicPickerModule` type
- the picker presentation for sprite choices and glyph choices
- a visible `CHARACTER` → `CLEAR` row that equips the blank glyph for any paint tool
- left/right click assignment from picker hits into live painter `tool-state`
- glyph section ordering sourced from Thaum Mono's supported-character sections; parsing must stay in lockstep with the renderer's sprite-section parser (`glyph_graphic.rs::parse_glyph_sprite_sections`) — a section title may sit on its own line or be glued to its glyph line (`borders:━┃…`), and a stricter parser here silently drops whole sections from the picker
- the space-glyph display placeholder while still assigning the real `' '` glyph
- wheel scrolling of the picker's content: the module renders only the rows visible inside its bounds, `on_wheel` moves a row-offset (clamped at both content edges), and hits exist only for visible rows so cropped content can't be clicked
- vertical layout mirrors the Flat2d HUD orientation (larger local y renders higher on screen): RECENT is pinned at the screen-top end, and the scroll list walks from the screen-top down
- one scrollable category list: the glyph sections (titles above their glyphs) followed by a standard `SPRITES` section with its own title — sprites are not a separated pinned block
- a pinned RECENT section locked to the top of the module (header + one row of the last assigned graphics, most-recent-first, deduped, capped at 10) that stays visible while the glyph list scrolls under it; clicking a recent slot re-assigns that graphic to the clicking hand; fed by draw-time observation of both hands so every assignment path lands there
- the tooling-weight preview rule: every preview cell (glyphs, sprites, sided, clear, recent) shows the weight the hands are actually tooling — exactly one hand targeted on Image shows that hand's weight statically; both hands on Image or neither alternates left→right weights on the renderer's vivid-flash breath so both candidate weights are visible; the frame-breath clock is a shared `Rc<Cell<i32>>` written by the entrypoint each frame before module draw, and the entrypoint's demand gate keeps frames coming every fallback-breath tick while the alternation is visible
- the vertical scrollbar over the scrolling category rows (shared `thaum-renderer/domain/modules/shared/scrollbar/` seam): overlaid on the rightmost content column between the pinned RECENT block and the content bottom, thumb drag + track page-jump, drawn only when the rows overflow; a left press on the bar never assigns a glyph; when the bar is visible the character rows wrap one column short (J) so no character sits under the bar and none becomes unreachable without resizing the module

## does not own
- the renderer's `Module` contract or registry semantics, owned by `thaum-renderer/domain/modules/`
- generic picker chrome or gizmo behavior, owned by `thaum-renderer/domain/modules/shared/`
- the live hand state itself, owned by `domain/painter-session/tool-state/`
- a `sided` category after the sprites section: declaration files under the asset root's `sided/*.json`, parsed once at boot (`load_sided_options`); clicking equips the full `SidedGraphic` into a hand, and the row's preview cell resolves its visible side at render staging like any sided cell
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
  - validates glyph-section parsing plus left/right glyph, clear-character, and sprite assignment
  - validates the tooling-weight preview rule: single-image-hand static weight, both-image and neither-image alternation keyed on the vivid-flash phase
  - validates the vertical scrollbar: bar cells on overflow, bar-aware row wrapping (no glyph or glyph hit on the bar's column while the bar shows), thumb drag walking the category rows to both edges

## data
- none

## notes
- source-of-truth from J: "if only one is targeted on image, we can use that weight for all the characters in the character selectore module. if both left and right hand are targeted on image, then we can alternate weights from the left hand to the right hand in the same blinking time as the selection shader uses. ... if a user has niether left or right hand with image (both on select) then we can simply show it with the alternating same as both on image" — motivator: users did not discover weight editing and could not see the weight they were tooling while the palette was up.
- the old per-assigned-glyph weight preview (max of the hands holding that graphic) was replaced by the tooling-weight rule; per-hand color preview and the hand-settings 3x3 preview blocks still show each hand's actual state.
- source-of-truth from J: "Ideally we see both glyphs and sprites."
- source-of-truth from J: "I like my glyphs sorted by asci section."
- source-of-truth from J: "We have those ASCII sections in thaum mono. Use the supported characters."
- glyphs stay their own section inside this picker even though the live hand state now stores one shared `graphic` slot that can be either glyph or sprite.
- `CLEAR` is a deliberately visible space glyph: it renders as a dot in the picker but equips `' '`, so brush, fill, lasso, and future standard paint tools all remove cells through the shared blank-cell write seam.
- source-of-truth from J: the picker should scroll up and down with its content cropped inside the module bounds, so the panel can be made smaller and the user can still look for characters.
- source-of-truth from J: the horizontal glyph layout must expand and contract with horizontal module resizing (rows re-chunk at the panel's content width).
- source-of-truth from J: the scroll list is one set of standard categories — sprites and glyphs are not separated in this menu, only RECENT stays a pinned section, and it must sit at the top of the module on screen.
- source-of-truth from J (painter-wide, owned by `domain/painter-session/tool-state/`): if any of the three edit channels (graphic/color/weight) is locked, strokes and stamps must not place on empty cells — only an all-unlocked hand authors new cells; locked-channel partial edits still restyle existing cells in place.
- the recent list is module-local and not persisted across restarts; if J wants recall to survive a session, `PersistedModuleUiState` is the seam to extend.
- scroll sits in this module (wheel dispatch itself is renderer-owned via `Module::on_wheel` / `registry.dispatch_wheel_at`); if a second scrolling panel appears, the row-offset/crop logic should graduate into `thaum-renderer/domain/modules/shared/`.
