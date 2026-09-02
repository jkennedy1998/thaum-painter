# /home/j/Repos/thaum-painter/domain/modules

## purpose
Mirror `thaum-renderer/domain/modules/`'s Module format locally for thaum-painter, hosting painter-only modules on top of the renderer's shared registry/gizmo/chrome logic.

## owns
- painter-only shared module helpers, if any grow beyond what `thaum-renderer/domain/modules/shared/` already provides
- painter's own individual modules: indexed sprite-color picker, RGB color block, drawing-space bounds gizmo, material picker, character picker, toolbar, and similar panels the game never uses

## does not own
- the `Module` contract itself or its registry semantics, owned by `thaum-renderer/domain/modules/`
- generic gizmo/chrome behavior reusable by any consumer, owned by `thaum-renderer/domain/modules/shared/`
- raw input capture or action-binding, owned by `thaum-renderer/domain/controls/`
- painter-document's authored content unit (`domain/painter-document/layers/`) — unrelated concept, see `context/module-concept-audit.md`

## children-encapsulations
- `shared/`
  - default
- `individuals/`
  - default

## contents
- `contract.md`
  - modules contract

## dependencies
- `/home/j/Repos/thaum-renderer/domain/modules/`
- `/home/j/Repos/thaum-renderer/domain/modules/shared/`
- `/home/j/Repos/thaum-renderer/domain/controls/`

## exposed interfaces
- none yet

## interface consumers
- `/home/j/Repos/thaum-painter/orchestration/entrypoint/`

## artifacts
- none

## tests
- none yet

## data
- none

## notes
- `individuals/toolbar/` is the first module built here, implementing `thaum-renderer`'s `Module` trait directly and drawn by `orchestration/entrypoint/` through a `ModuleRegistry` — the same relationship `thaum-renderer` itself uses to own the `Module` contract while any consumer registers concrete panels.
- the next useful painter-only modules now landed too: a painter-bound indexed color-picker wrapper, a painter-bound RGB color block, a painter-only drawing-space bounds gizmo, a material picker, a compact hand-settings panel, and a graphic picker that chooses either glyphs or sprites into the shared hand `graphic` slot.
- old-system precedent for remaining painter-specific modules: `THAUMWORLD-AUTO-STORY-TELLER/src/mono_ui/modules/painter_canvas_module.ts`, `character_editor_module.ts`, and similar — future `individuals/` entries here.
