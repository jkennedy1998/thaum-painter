# thaum-painter/orchestration/build-commands

## purpose
Own the public build commands and entrypoint binary crate for thaum-painter. One lane layout is shared by local and GitHub builds, producing replaceable per-environment folders and deliberate public releases.

## source-of-truth (from J)
- `orchestration/builds/<environment>/thaum painter/` is the only place builds land. One build path per environment; more environments (android etc.) extend the same shape later.
- Each lane root holds the exe directly (`thaum painter` / `thaum painter.exe`) plus `renderer-assets/` and both LICENSE files. The lane folder is the whole app: zip it and send it.
- A lane folder is deleted and rebuilt in full on every build — no version churn, the folder is always current.
- `build-all-environments` builds lanes and never runs them. Default: every known environment; Linux and Windows build locally, while Mac is built by GitHub Actions from exact pushed painter and renderer commits, then downloaded into the mac lane.
- `run-and-build-linux` rebuilds the linux lane through the same shared pipeline and boots the built lane exe. There is exactly one linux build path — no separate dev lane.
- The manual GitHub release workflow packages exact painter and renderer commits into a three-platform GitHub Release. `release-thaum-painter` is the maintainer-local public-release command: it tags the matching app version, waits for that workflow, verifies all three published assets, then updates only the canonical website page's displayed version, generated GitHub release notes, and release manifest. Its three permanent `releases/latest/download` links remain stable but are verified on every publication.
- The linux lane is the source of truth: what run-and-build-linux boots is byte-identical to what gets sent to someone.
- The renderer never builds standalone: it is a library, open-sourced later, packaged along inside every painter lane (as `renderer-assets/` plus the linked static code). Renderer "releases" are source tags only.
- All cargo build output lives under `orchestration/builds/.cargo-cache/` (via repo-root `.cargo/config.toml`), never in the repo root and never nested inside other encapsulations.

## owns
- the `thaum-painter-entrypoint` binary crate and its `main.rs`, including the passive command-bar release-status check and system-browser download-page action
- `build-all-environments` — build every (or selected) environment lane, never runs
- `run-and-build-linux` — rebuild linux lane + boot the built exe
- GitHub Actions workflows for remote Mac builds and public three-platform releases
- `release-thaum-painter` — maintainer-local tag/workflow/asset-verification orchestration plus canonical website release-fact publication
- the per-environment lane layout and replacement behavior
- the lane-assembly tail shared by both commands (exe + renderer-assets + licenses)

## does not own
- renderer domain truth or boot internals, owned by `thaum-renderer`
- website presentation/content beyond the exact current-version, generated release-notes, and manifest facts `release-thaum-painter` publishes after a verified release
- painter domain modules, owned by `domain/modules/individuals/`
- GitHub Release hosting semantics beyond uploading the three built archives

## children-encapsulations
- none

## contents
- `Cargo.toml` — `thaum-painter-entrypoint` binary crate manifest
- `src/main.rs` — boot + app entrypoint
- `src/painter_modules.rs` — the one place that assembles the module registry
- `src/run_log.rs` — per-boot `artifacts/debug-logging/<UTC-stamp>[-NN]/` run.log + perf.jsonl wiring, 5-folder culling, panic hook
- `truth.md` — durable J source-of-truth decisions for the build and release workflow
- `build-all-environments` — build lanes under `orchestration/builds/<env>/thaum painter/`; shared pipeline, replace-per-build
- `run-and-build-linux` — build linux lane (same pipeline) + exec the lane exe
- `release-thaum-painter` — tag/publish a matching app version, then publish its verified version, generated release notes, and manifest to the canonical website page
- `.github/workflows/build-mac.yml` — exact-commit remote Mac lane workflow for build-all
- `.github/workflows/release-thaum-painter.yml` — exact-commit three-platform public-release workflow
- repo-root `.cargo/config.toml` — redirect all workspace cargo output to `orchestration/builds/.cargo-cache/`

## dependencies
- `thaum-painter/domain/`
- `thaum-painter/domain/modules/individuals/` (toolbox, paint-color-picker, paint-color-block, material-picker, graphic-picker, hand-settings)
- `thaum-painter/workers/`
- `thaum-renderer/orchestration/boot/`
- `thaum-renderer/domain/`
- `thaum-renderer/orchestration/renderer-assets/` (copied into each lane at build time)

## exposed interfaces
- none (human-facing commands, not consumed by other encapsulations)

## interface consumers
- humans and operators building/running thaum-painter

## artifacts
- `orchestration/builds/<environment>/thaum painter/` — the shippable lanes (git-ignored), replaced in full on every build
- `orchestration/builds/.cargo-cache/` — regenerable cargo build cache for the whole workspace (git-ignored)
- GitHub Actions artifacts — temporary remote Mac lanes and release-job archives
- GitHub Releases — public stable Linux, Windows, and Mac archives
- `jartanddesign.com/thaum-painter/release.json` — static public version/page manifest, changed with the rendered release notes only after the matching GitHub Release assets are verified

## tests
- `src/main.rs` inline `#[cfg(test)]` module (light): cell-path interpolation, file-root resolution, dialog-path normalization, registry/live hotkey agreement
- `src/run_log.rs` inline tests (light): boot-stamp formatting, culling-to-cap behavior

## data
- none

## notes
- Environments: `linux` (host build), `windows` (cross `x86_64-pc-windows-gnu`, needs the mingw toolchain installed once), `mac` (built on GitHub `macos-latest`, Apple Silicon). `build-all-environments` validates that both painter and renderer are clean and exactly equal to their pushed `origin/main` commits before dispatching and downloading the Mac lane. On jobo it uses `tools/github/` with the canonical `GITHUB_TOKEN` runtime secret rather than persistent GitHub CLI credentials.
- Runtime paths stay portable: `painter_root()` in `main.rs` resolves writes as `THAUM_PAINTER_ROOT` env → compiled repo root when on disk (dev runs keep the repo layout) → the exe's own folder. So the linux lane exe run on jobo still writes into the repo dev layout, and a lane sent to another machine is self-contained next to its exe.
- Asset resolution: `THAUM_RENDERER_ASSET_ROOT` env → `renderer-assets/` next to the exe → compiled repo path.
- Source-of-truth from J (2026-09-10): SAVE on a never-saved document triggers SAVE AS — the user always picks the location, so a file never lands somewhere unexpected. Quick save (no-dialog auto-named folders under painter-files) is retired. The typed-path terminal fallback keeps SAVE survivable where the native dialog stack fails (XDG portal backend failed instantly on jobo). `artifacts/painter-files/` remains the save/open dialogs' default start directory.
- Source-of-truth from J: files save relative to wherever the painter lives, same painter-shaped structure, just relocated under the exe's folder when deployed.
- Document autosave (J 2026-09-10 power-loss recovery): an unsaved document snapshots into the vault `artifacts/autosave/<slug>-autosave-<UTC-stamp>/` every 10 minutes; once a manual save lands, the document's own root snapshots every 5 minutes via the guarded snapshot seam. Mirrors Adobe's AutoRecover shape — recovery copies live in a painter-owned vault, never next to user-chosen files. The vault copy is pruned when save/save-as/open/new retires it (kept if the user opened the vault copy itself). Session clients never autosave — the host owns saves while multiplayer is live; hosts autosave through the guarded seam whose guards are suspended by the hosting flag. Ticked every frame BEFORE the demand gate so an idle painter still snapshots (`tick_document_autosave`).
- Source-of-truth from J (2026-09-10): `context/` is design-truth notes only; runtime output (autosave vault, debug run logs) lives under `artifacts/`. Run logs moved from `context/debug-logging/` to `artifacts/debug-logging/`.
- Cross-repo path deps use relative sibling paths (`../../../thaum-renderer/...` from here); no compiled-in absolute machine paths.
- `run-and-build-linux` replaced the old `run.sh`; `build-all-environments` replaced `package.sh`. The old `orchestration/entrypoint/` name is retired — this encapsulation now owns building, not just booting.
- The GitHub Mac workflow reuses this lane layout (exe + `renderer-assets/` + both LICENSE files, zipped lane folder) and accepts immutable painter and renderer commit IDs. The release workflow uses those same immutable inputs to assemble and publish all three platform archives. Ordinary builds never publish a release.
- The manual release workflow consumes explicit immutable painter and renderer commit IDs and its version tag is the deliberate public-facing boundary. It never publishes ordinary commits. `release-thaum-painter` refuses a tag that does not match `thaum-painter-entrypoint`'s package version and refuses website publication unless all three named GitHub Release assets exist.
- The mac lane binary is unsigned; recipients must bypass Gatekeeper once (`xattr -cr "thaum painter"` or right-click → Open on first launch).
- Session-bridge borrow discipline: never pass `timeline_state.borrow().<field>` (or any `RefCell.borrow()` temporary) directly in a call whose body drains the layers-panel queue or `borrow_mut()`s the same state — argument temporaries live until the end of the full call statement, so the held `Ref` turns the inner `borrow_mut()` into a `RefCell already borrowed` panic (the scrub-then-release timeline crash: `finish_canvas_release` drains queued `SetCurrentBreath` actions). Read the field into a binding first.
