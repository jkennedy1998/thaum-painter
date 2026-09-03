# /home/j/Repos/thaum-painter/domain/tai

## purpose
Own the painter's tool-assisted input (TAI) tests: painter-declared action bindings, the growable individuals script list, and the registry-driven harness that replays them through the renderer's TAI runner.

## owns
- painter action bindings for TAI assertion (`painter_bindings()`), intended to become the painter's one hotkey/binding declaration when the entrypoint adopts it
- the growable `individuals/` list: one folder + `script.json` per painter TAI
- `individuals/registry.json`, the id -> path/status/purpose list
- the registry-driven harness that loads every registered script and runs it against painter bindings

## does not own
- the script format or replay runner — consumed from `/home/j/Repos/thaum-renderer/domain/controls/tai/`
- raw input shapes or `ActionBindingMap` — consumed from renderer `domain/controls/`
- app-level UI assertion helpers beyond action fires (add on top of the fired-log only when a script genuinely needs them)
- live-window input injection

## children-encapsulations
- none

## contents
- `tai.rs`
  - painter bindings, registry loader, inline tests
- `contract.md`
  - this contract
- `individuals/registry.json`
  - growable registry: id -> path, status, purpose, notes
- `individuals/taiNN_name/script.json`
  - one folder + one script per painter TAI

## script format
Renderer-owned. See `/home/j/Repos/thaum-renderer/domain/controls/tai/contract.md`; copy `template/script.json` from there.

## adding a painter TAI (growth path)
1. copy the renderer template into `individuals/taiNN_name/`, rename the script `id`
2. fill in actions + expectations using painter action names declared in `painter_bindings()`
3. add one line to `individuals/registry.json`
4. only touch `tai.rs` if the script needs a painter binding that isn't declared yet

## dependencies
- `/home/j/Repos/thaum-renderer/domain/controls/tai/` (TaiScript runner)
- `/home/j/Repos/thaum-painter/domain/`

## exposed interfaces
- painter tai seam
  - describes `painter_bindings()`, `load_registered_scripts()`, `TAI_TOTAL_BREATHS` consumed by painter tests and, later, by entrypoint input handling

## interface consumers
- `tai.rs` inline tests
- `/home/j/Repos/thaum-painter/orchestration/entrypoint/src/main.rs` — the entrypoint's live key dispatch resolves physical key presses through `painter_bindings()` and fires named actions (registry-owned keys never fall through to live-only match arms); wheel-bound actions stay wheel-handled

## artifacts
- none

## tests
- `tai.rs` inline `#[cfg(test)]` module
  - light
  - validates registry loading, that every registered painter TAI parses and passes against painter bindings, and that the smoke TAI fires exactly its expected actions.

## data
- none

## notes
- Ownership split (J): the renderer owns TAI infra, consumers own the actual tests. This is the painter consumer seam; the infra seam lives in renderer `domain/controls/tai/`.
- Registry statuses follow the old system's convention: `canonical` for baseline TAIs, `secondary` for extras.
- Old-system painter TAIs (`tai03` canonical all-tools, text focus regression, wheel depth zoom/pan, etc.) are candidates to port here one by one as painter actions get declared.
- `painter_bindings()` is now consumed by the entrypoint for live hotkeys (adoption done), so a binding change here changes live behavior; keep names in sync with tests that assert the live dispatch.
- Live dispatch resolves through the renderer's controls-profile seam: `effective_bindings(painter_bindings(), ControlsProfile)` is the one map the entrypoint and the controls panel read. The per-user profile rides `PainterUserSessionState.controls_profile` (overrides only, persisted in the user session file); the panel's click-to-rebind writes overrides through `set_override` and recomputes the effective map. Registry defaults are never restated, so newly declared actions pick up their declared binding until the user overrides them.
- Known naming drift: the registry still carries old-system tool names (`painter_select_pencil` -> live Brush, `painter_select_bucket` -> live Fill). Renaming would change TAI script semantics, so the entrypoint maps the old names explicitly; collapse to live tool names only together with the registered scripts that use them.
