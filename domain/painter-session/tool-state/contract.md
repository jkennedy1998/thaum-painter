# thaum-painter/domain/painter-session/tool-state

## purpose
Own live tool settings and active authoring-hand state for the painter session.

## owns
- active tool selections
- left/right hand brush state
- per-hand color, glyph, weight, brush-size, and fill-connectivity source-of-truth state
- the rule that hand color can be either direct RGB or a material-backed choice
- edit-channel masks
- per-hand image-versus-selection target state
- fill's flood-select channel matching as a fill-specific opt-in (`fill_match_channels`): off matches on every channel, on follows the hand's Select row
- picker session behavior: click-only cell sampling into a hand's graphic/color/weight, gated by the picking hand's edit-channel toggles, with the per-hand `pick_opposite_hand` property routing the sample to the opposite hand
- the `PaintTool` enum and per-tool session behavior match arms (edit/select semantics) — the thin enum <-> registry bridge and the property manifest delegation now read from `domain/painter-tools/`
- lasso session behavior: in-progress bound accumulation (`lasso_stroke`), release-time enclosed-region fill through the channel mask, and release-time selection-surface application
- text-entry and paste option state that belongs to the live session

## does not own
- persistent file schema ownership
- pure tool operations
- history ownership

## children-encapsulations
- none

## contents
- `contract.md`
  - tool-state contract
- `tool_state.rs`
  - live left/right hand tool-state, selection-gated image editing, masked paint application, lasso release fill/selection application, including blank-glyph preservation on empty cells
- stamp seams: `stamp_changes_for_hand` places the hand user's copied world cells at an anchor with the copied cells' own graphic/color/weight (enabled edit channels paste the copied value, locked channels keep the target cell's existing value, selection-gated, any-locked-channel stamps skipping empty target cells entirely under the unified empty-cell rule), and `stamp_preview_cells` pairs current cells with the upcoming stamp for the two-phase flash
- source-of-truth from J (void marker): in the as-it-is phase of the lasso/stamp/selection flash, an empty cell (absent or authored blank) displays as a vivid `●` glyph at weight 0 instead of an invisible space, so coverage over voids stays readable. The other flash phase is unchanged.
- `lasso_stroke.rs`
  - in-progress lasso bound accumulation (`LassoStroke`) and its overlay preview cell group (path only, never the interior)

## dependencies
- `thaum-painter/domain/painter-operations/`
- `thaum-painter/domain/painter-session/paint-color/`
- `thaum-painter/domain/painter-session/clipboard/` (stamp consumes `WorldCopyData` payload shapes)
- `thaum-painter/domain/painter-tools/`
- `thaum-painter/domain/file/storage/`
## exposed interfaces
- none

## interface consumers
- painter-session

## artifacts
- none

## tests
- inline `#[cfg(test)]` in `tool_state.rs`
  - light
  - validates per-hand tool selection, shared brush/erase properties, selection-gated image editing, fill masking, lasso enclosed-region fill/selection, fill's opt-in flood channel matching, and the per-tool property-row manifest
- inline `#[cfg(test)]` in `lasso_stroke.rs`
  - light
  - validates lasso stroke accumulation

## data
- none

## notes
- source-of-truth from J: left and right mouse buttons should map to separately assignable painter tools, matching the old painter's hand-based workflow.
- source-of-truth from J: the color and weight selected for the left and right mouse button should live as a single source of truth and support tool-side shenanigans.
- this now stores painter-facing color choice through `paint-color/` so flat sprite colors and material palettes share one hand-state seam.
- source-of-truth from J: left/right masking between image editing and selection editing was useful and should come back.
- source-of-truth from J: once a selection exists, image-editing tools should have one common seam that limits edits to the selected cells so brush, erase, fill, rect tools, and future delete actions all obey the same rule.
- source-of-truth from J: left/right channel locks for character, weight, or color were useful for in-depth illustration where only some layers should change. (Later superseded: J called locks bloat and they were removed entirely — see below.)
- source-of-truth from J: tools use properties; tools that share the same property UI reuse the same panel pieces, all tool properties stay tracked per tool, and the properties panel hides rows neither the left nor right hand's tool uses so panels stay thin as tools gain properties.
- registration truth (id, label, icon, property rows, hotkey) moved to the `domain/painter-tools/` registry; this encapsulation keeps session behavior match arms until per-tool behavior migration and owns the small enum <-> registry bridge that drift-tests against the registry.
- cross-tool rules are not re-decided here: the erase-forces-subtract selection rule and its future siblings live in `domain/painter-tools/shared/` and this encapsulation consumes them.
- when graphic editing is disabled and the user edits an empty cell, painter now preserves that cell's implicit blank glyph (`' '`) rather than silently swapping in the hand's current graphic.
- source-of-truth from J: the picker tool's channel gating reads the toggles for what a tool is editing (the picking hand's edit channels); with opposite hand on, a picking hand without gfx toggled hands only color and weight to the opposite hand.
- source-of-truth from J: channel locks were bloat and largely not used and have been removed entirely (row, `ChannelLocks`, setter guards, persistence field); nothing gates pick-panel or weight-stepper updates anymore.
- source-of-truth from J: fill's use of the Select row to decide flood-match channels is not standard select behavior — it is now a fill-specific opt-in (`fill_match_channels`, per hand, persisted); with it off, flood select matches on every channel and the Select row does not gate it.
- source-of-truth from J: the opposite-hand picker property toggles independently per hand.
- source-of-truth from J: the lasso tool equips per hand, records its bound with press-drag-release, and on release fills the enclosed cells with the selected character — which may be an empty cell and may be channel-locked (character-only or color-only) — so the fill flows through the same hand-state seams as brush/fill, and it only edits within the selected tiles.
- source-of-truth from J (empty-cell placement rule): if ANY of the three edit channels (graphic/color/weight) is locked, strokes and stamps must not place on empty cells — a locked channel has no existing value to keep there, so a partial edit cannot resolve. Only an all-unlocked hand authors new cells; locked-channel edits still restyle existing cells in place. Implemented in `resolved_painted_cell` (brush/fill/lasso seam) and the `stamp_changes_for_hand` empty-target guard.
