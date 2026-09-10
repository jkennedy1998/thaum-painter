# encapsulation plan — renderer command bar release-status support

linked project plan: `plans/project-thaum-painter-release-status-and-download-page-plan.md` (phase-3)

## implementation-rules
- Update checklist states while working.
- This plan changes generic command-bar presentation only; it must not grow Painter-specific release/network logic.
- Reuse `Hotspot` and the existing tooltip dwell/card renderer rather than make a second tooltip path.

## status key
- `[ ]` not done
- `[+]` implemented
- `[#]` tested

## pre-implementation-note
`command_bar.rs` currently gives every visible button the same idle Medium color and has no tooltip declaration surface, even though it already calculates precise button geometry and tracks the hovered button id. Painter needs one status-bearing direct button. The smallest reusable change is opt-in metadata on `CommandBarButton`, defaulting exactly to current behavior, and an accessor that turns the current visible layout into existing `Hotspot` values.

## target-encapsulation
- `/home/j/Repos/thaum-renderer/domain/command-bar/`: edit

## artifacts
- none

## tests
- `domain/command-bar/command_bar.rs` inline tests — default button rendering, explicit palette-role rendering, and visible button hotspot geometry.

## data
- none

## phases
### phase-1 — docs + contract + truth alignment
- [+] inspect the parent renderer/module/tooltip contracts and retain command bar’s generic ownership.
- [+] document the opt-in/default-preserving presentation and tooltip contract beside the command-bar implementation if the local boundary lacks this truth.
- [+] record that tooltip timing/card visuals remain owned by `domain/modules/shared/tooltip/`.
- [+] record consumers: existing command bars unchanged; Painter supplies one configured status button.

### phase-2 — generic button presentation metadata
- [+] extend `CommandBarButton` with optional idle `UiColorRole` and optional tooltip copy while keeping `new(id, label)` visually/source compatible.
- [+] add fluent construction or an equally minimal setter shape for the configured button path.
- [#] render an opted-in button in its requested palette role when idle; preserve existing vivid hover treatment unless a deliberate configured-hover policy is needed.
- [#] prove ordinary `FILE`/`MODULES` buttons retain their current color/weight behavior.

### phase-3 — command-bar hotspot exposure
- [#] expose one `Hotspot` per currently visible configured button from the command bar’s actual `button_layout()` geometry.
- [#] ensure hidden, clipped, non-tooltip, nested-off-path, and scrolled-off buttons yield no active hotspot.
- [+] let Painter’s existing frame-loop tooltip selection include command-bar hotspots without duplicating dwell, placement, or card draw code.
- [#] prove hover geometry exactly matches click geometry.

### phase-4 — validation + repo-rule sweep + git commit
- [#] run renderer command-bar tests and format/lint checks used by this repository.
- [ ] manually verify default and configured button visuals in a Painter build.
- [+] review no Painter release URL, HTTP, browser, or status state leaked into Renderer.
- [ ] run applicable encapsulation/repo-rule review.
- [ ] git commit if approved.

## post-implementation-notes
- audit-2026-09-10: generic metadata/hotspots remain free of Painter URL, HTTP, browser, and release-state logic. Renderer unit tests cover default compatibility, custom idle color, and visible hotspot/click-layout agreement. A live Painter visual pass and an explicit repo-health pass remain deferred.
- plan-finished: false
- encapsulation-git-commit: false
