# /home/j/Repos/thaum-painter/orchestration/entrypoint

## purpose
Own the real, runnable consumer entrypoint that boots `thaum-renderer` for thaum-painter — the local rebuild-and-run loop a human presses to see thaum-painter on screen, mirroring `thaum-renderer-test-user`'s proof-app shape but living inside this repo instead of an ungoverned sibling.

## owns
- the `thaum-painter-entrypoint` binary crate and its `main.rs`
- local rebuild-and-launch scripting (`run.sh`, `run.desktop`) for double-click iteration
- boot-time asset-root resolution and window/boot config for this consumer
- the boot proof scene (registered `Module`s including painter's own `ToolboxModule`, painter-bound `PaintColorPickerModule`, painter-bound `PaintColorBlockModule`, painter-bound `MaterialPickerModule`, painter-bound `GraphicPickerModule`, and `HandSettingsModule`, drawn through `ModuleRegistry`/`Composition`) proving the loop actually renders something
- per-frame pointer dispatch: converting the renderer's clip-space left/right clicks and cursor into world/cell points and routing them into `ModuleRegistry::dispatch_pointer_event_at`/`dispatch_captured_pointer_move`/`dispatch_captured_pointer_up` (plus `remove_closed_modules` each frame) so registered modules react to real clicks and real held-button drag sessions (a module's move/resize gizmo), not just discrete clicks
- the render-space boot proof: parsing the bundled `example-thaum-painter-file-v1.json` once at startup and calling `domain/rendering/render-space/`'s `build_composition` every frame, driven by the renderer's own auto-ticking fallback breath clock (`state.data_lanes.breath()`), so `render-space`'s timing/move-block resolution is provably exercised live, not only under `cargo test`
- the first real hand-based paint loop: left/right clicks and drags over the paint canvas apply the left-hand/right-hand tools from `domain/painter-session/tool-state/`, so toolbox selection immediately affects live painting
- live scene-space canvas + selection proof rendering: the paint canvas cells and plane-selection border are emitted as regular composition groups in the renderer's world/3D intake path, while toolbox/indexed-color-picker/color-block/material-picker/graphic-picker/hand-settings remain flat 2D modules
- registry-backed hotkey dispatch: physical key presses resolve through the TAI registry's `painter_bindings()` (`domain/tai/`) by binding name before any live-only match arm runs; registry-owned keys (P/B tool select) never fall through to the hand-written arms, and wheel-bound registry actions (zoom) stay wheel-handled
- typing-mode input focus: while a text-tool typing session is active the renderer's `TypingMode` gate (`thaum-renderer` `domain/controls/typing-mode/`) routes every input — held and just-pressed keys, pointer clicks/drags, and wheel — with the current camera/depth bindings (from the effective binding map, so remaps move the reserved set) as the only inputs that stay live outside the session

## does not own
- renderer domain truth or boot internals, owned by `thaum-renderer`
- a real, editable painter document/session in this window — the render-space proof only replays one bundled, read-only example manifest; live document authoring is deferred until `domain/painter-document/`, `domain/painter-session/`, and `domain/painter-operations/` exist
- painter's own modules, owned by `domain/modules/individuals/`
- app runtime logic beyond boot/launch, owned by `domain/`

## children-encapsulations
- none

## contents
- `Cargo.toml`
  - `thaum-painter-entrypoint` binary crate manifest
- `src/main.rs`
  - boot + proof-scene entrypoint
- `src/painter_modules.rs`
  - the one place that assembles the module registry; the event loop consumes shared handles and never touches registration order
- `run.sh`
  - rebuilds (with `CARGO_TARGET_DIR` redirected into this encapsulation's `artifacts/`) and launches the binary
- `run.desktop`
  - double-click launcher invoking `run.sh` with a visible terminal
- `.gitignore`
  - excludes regenerated `artifacts/*` build output from version control

## dependencies
- `/home/j/Repos/thaum-painter/domain/`
- `/home/j/Repos/thaum-painter/domain/modules/individuals/toolbar/`
- `/home/j/Repos/thaum-renderer/orchestration/boot/`
- `/home/j/Repos/thaum-renderer/domain/`
- `/home/j/Repos/thaum-renderer/domain/modules/`
- `/home/j/Repos/thaum-renderer/domain/modules/individuals/color-picker/`
- `/home/j/Repos/thaum-renderer/domain/modules/shared/ui-palette/`
- `/home/j/Repos/thaum-renderer/domain/modules/shared/panel-chrome/`
- `/home/j/Repos/thaum-renderer/domain/modules/shared/module-gizmos/`

## exposed interfaces
- none

## interface consumers
- humans and operators running thaum-painter locally

## artifacts
- `artifacts/target/`
  - cargo build output for this crate, redirected here via `CARGO_TARGET_DIR` so build accumulation stays owned by this encapsulation instead of the shared workspace `target/`; regenerated on every `run.sh` invocation and git-ignored

## tests
- `src/main.rs` inline `#[cfg(test)]` module (light): cell-path interpolation, file-root resolution, dialog-path normalization, and the registry/live hotkey agreement tests — every registry key binding resolves to live dispatch, every action the dispatch fires exists in `painter_bindings()`, and wheel-bound actions never resolve through the key path

## data
- none

## notes
- this is the first real "press button, see it run" surface for thaum-painter; before this pass `orchestration/` was intentionally empty.
- draws painter's own `ToolboxModule` (`domain/modules/individuals/toolbox/`), `PaintColorPickerModule` (`domain/modules/individuals/paint-color-picker/`), `PaintColorBlockModule` (`domain/modules/individuals/paint-color-block/`), `MaterialPickerModule` (`domain/modules/individuals/material-picker/`), `GraphicPickerModule` (`domain/modules/individuals/graphic-picker/`), and `HandSettingsModule` (`domain/modules/individuals/hand-settings/`) alongside the generic renderer proof modules, demonstrating the intended relationship: `thaum-renderer` owns the `Module` trait and `ModuleRegistry`, painter owns concrete module implementations, and this entrypoint composes both into one window.
- click handling is real end-to-end, not simulated: `tools/window-surface` captures OS mouse events, `orchestration/boot` exposes them per-frame plus a zoom-aware clip-size helper, `domain/camera/screen-world-remap/` converts clip-space to a world/cell point, and this entrypoint's frame-provider closure dispatches that point into the registry every frame before rebuilding the composition — all of that is renderer-owned machinery this entrypoint only calls, per the same ownership split as the rest of the boot seam.
- source-of-truth from J: the old painter's toolbox UX mattered, specifically assigning left-hand and right-hand tools with left and right mouse buttons. This entrypoint and `domain/modules/individuals/toolbox/` are the first rebuild of that habit.
- source-of-truth from J: left/right color and weight state, plus masking and lock behavior, should come back cleanly. This entrypoint now wires indexed sprite colors, arbitrary RGB picking, and material picking through shared `tool-state` plus painter-owned modules.
- source-of-truth from J: the ASCII painter canvas should render in the 3D layer even when the focus lane visually lines up with the 2D UI layer. The current proof app now does that for the live canvas cells and plane-selection border instead of drawing them as flat panel chrome.
- gizmo drag sessions (move/resize) are real too, not simulated: the frame loop reads `WindowSurfaceInput.pointer_down` (persists across frames while the button is held, unlike `just_clicked`) alongside `cursor_position` and, once a gizmo click made a module request capture, calls `ModuleRegistry::dispatch_captured_pointer_move` every held frame and `dispatch_captured_pointer_up` on release — so a color-picker (or any future gizmo-enabled module) actually moves/resizes on screen in real time as the mouse drags, and `remove_closed_modules` is called every frame so a clicked close gizmo actually removes its module.
- the render-space boot proof (`RENDER_SPACE_PROOF_MANIFEST_JSON`, `render_space_proof_manifest`/`render_space_proof_composition` in `main.rs`) mirrors `ProofLabelModule`'s existing "boot proof only" pattern: it is not one of painter's real modules and is not wired to any editing UI, it exists to prove `build_composition` renders a saved document end-to-end in a real window. Its `CellGroup`s are appended directly onto the per-frame `groups` vec alongside the drawn `Module`s, not registered with `ModuleRegistry`, since this content isn't interactive yet. A real, editable document is the natural next step once `domain/painter-document/`/`domain/painter-session/` land.
- cross-repo path dependencies use absolute `/home/j/Repos/thaum-renderer/...` paths, matching the existing convention already used by `domain/Cargo.toml`.
- `thaum-renderer-test-user` remains the canonical proof app for the renderer itself (`/home/j/Repos/thaum-renderer/tests/contract.md`); this encapsulation is painter's own equivalent and should not be confused with it or replace it.
- run via `./run.sh` from a terminal, or double-click `run.desktop` in a file manager (may need "allow launching" / mark-as-trusted the first time, depending on desktop environment).
