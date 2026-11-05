# thaum-painter/domain/modules/individuals/display-profile

## purpose
Own painter's compact display-profile panel: live editing of the open file's cell width/height, plus a left/right list switcher each for the active typeface and spriteset (project 04n3fq3e phase-9).

## owns
- the `DisplayProfileModule` type
- the `cell_size` number row (width/height, click to type, scroll to nudge, clamped to a sane editable range)
- the `typeface` and `spriteset` left/right list-switch rows, cycling through the renderer's installed typeface/spriteset ids with wraparound
- the `none` no-spriteset-override choice, prepended ahead of the discovered spriteset ids (the discovery helpers never emit it themselves — it is this module's own UI-layer addition)
- direct edits into the shared live `SharedDocumentDisplayProfile` handle it is constructed with

## does not own
- the saved/live document schema for the profile, owned by `thaum-painter/domain/file/storage/` (`SharedDocumentDisplayProfile`) and `thaum-painter/domain/file/file-schema/` (`DisplayProfile`)
- discovering which typeface/spriteset ids exist on disk, owned by `thaum-renderer/domain/cell-graphic/typeface-intake/` and `thaum-renderer/domain/atlas-intake/` (`list_available_typeface_ids`, `list_available_glyph_spriteset_ids`)
- applying the live profile onto `BootConfig` each frame, owned by `thaum-painter/orchestration/build-commands/`
- the `none` sentinel's actual effect on glyph loading, owned by `thaum-renderer/orchestration/boot/` (`NO_SPRITESET_OVERRIDE_ID`)
- generic renderer module seams, panel chrome, or the shared `PropertyRows`/`ListSwitchRow` primitive, owned by `thaum-renderer/domain/modules/`

## children-encapsulations
- none

## contents
- `display_profile_module.rs`
  - module UI for the live cell width/height fields and the typeface/spriteset switchers

## dependencies
- `thaum-painter/domain/file/storage/` (`SharedDocumentDisplayProfile`)
- `thaum-renderer/domain/modules/shared/property-rows/` (`PropertyRow::NumberRow`, `PropertyRow::ListSwitchRow`)
- `thaum-renderer/domain/modules/shared/panel-chrome/`
- `thaum-renderer/domain/modules/shared/module-gizmos/`

## exposed interfaces
- `DisplayProfileModule::new(id, rect, profile, typeface_options, spriteset_options)` — build one compact panel bound to a shared live `SharedDocumentDisplayProfile` handle and the caller-discovered typeface/spriteset id lists (spriteset ids only; `none` is added internally)
- `DisplayProfileModule::number_edit(&self)` / `number_edit_active(&self)` — the shared in-place number-field edit handle, routed by the host's typing seam (same `NumberFieldEdit` pattern as `hand-settings`)
- `MIN_CELL_DIMENSION`, `MAX_CELL_DIMENSION` — the row's editable clamp range, re-exported so the host's commit path clamps identically

## interaction consumers
- `thaum-painter/orchestration/build-commands/`

## artifacts
- none

## tests
- inline `#[cfg(test)]` in `display_profile_module.rs`
  - light
  - validates row content against the live profile and option lists, left/right switch cycling (with wraparound and a graceful fallback when the live value is not among the discovered options), the `none` sentinel's position, and number-field click-to-edit opening

## data
- none

## notes
- Matches J's confirmed design (2026-11, recorded in `thaum-renderer/truth.md`): "it should be tied to the file for the cell width and cell height, and maybe a switcher (left right toggle of a list) to keep the ui easy for a selected font, and same with the selected spriteset ideally."
- Reuses the shared `NumberFieldEdit`/`PropertyRows::number_field_at` pattern already established by `hand-settings` rather than inventing a second typing seam; the host's `handle_number_field_edit_key` branches on `row_id == "cell_size"` to commit into this module's profile handle instead of `tool_state`.
- This panel intentionally stays compact (three rows, no scrolling) — it is not a second toolbox.
