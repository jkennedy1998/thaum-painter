# thaum painter contract-first architecture project plan

## current-state
Thaum painter is still mostly empty, while the old ASCII painter had a wide feature set spread across document runtime, timing, selection, clipboard, tools, import/export, and preview adapters. Before rebuilding behavior, this repo should sort most of the core encapsulations and contracts so implementation can land piece by piece with clean ownership.

## affected-encapsulations
- `/home/j/Repos/thaum-painter/domain/file/`: edit
- `/home/j/Repos/thaum-painter/domain/rendering/`: edit
- `/home/j/Repos/thaum-painter/domain/painter-document/`: edit
- `/home/j/Repos/thaum-painter/domain/painter-document/modules/`: add
- `/home/j/Repos/thaum-painter/domain/painter-session/`: edit
- `/home/j/Repos/thaum-painter/domain/painter-operations/`: edit
- `/home/j/Repos/thaum-painter/context/`: edit

## encapsulation-plans
- `/home/j/Repos/thaum-painter/domain/file/`: `plans/home-j-repos-thaum-painter-domain-file-plan.md`
- `/home/j/Repos/thaum-painter/domain/rendering/render-space/`: `plans/home-j-repos-thaum-painter-domain-rendering-render-space-plan.md`
- `/home/j/Repos/thaum-painter/domain/painter-document/`: `plans/home-j-repos-thaum-painter-domain-painter-document-plan.md`
- `/home/j/Repos/thaum-painter/domain/painter-session/`: `plans/home-j-repos-thaum-painter-domain-painter-session-plan.md`
- `/home/j/Repos/thaum-painter/domain/painter-operations/`: `plans/home-j-repos-thaum-painter-domain-painter-operations-plan.md`

## encapsulation-details
### `/home/j/Repos/thaum-painter/domain/file/`
- action: edit
- intent: break saved file ownership into manifest, storage, import-export, and migration seams
- interface delta: add clearer sub-boundaries before implementation
- dependencies: `thaum-renderer` alignment for renderer-consumed fields only
- consumers: painter-document, render-space, future file UI
- artifacts: future sample files and migration fixtures
- tests: future file shape and migration tests
- data: none

### `/home/j/Repos/thaum-painter/domain/rendering/`
- action: edit
- intent: separate app camera intent from render-space handoff
- interface delta: add rendering camera seam beside render-space
- dependencies: painter-document, painter-session selection, thaum-renderer camera/domain seams
- consumers: future preview/editor boot
- artifacts: future render fixtures
- tests: future translation tests
- data: none

### `/home/j/Repos/thaum-painter/domain/painter-document/`
- action: edit
- intent: narrow painter-document into editing-facing group/timing/property views
- interface delta: add child seams for groups, timing, and properties
- dependencies: domain/file
- consumers: painter-session, rendering camera, render-space
- artifacts: none
- tests: future document-view tests
- data: none

### `/home/j/Repos/thaum-painter/domain/painter-session/`
- action: edit
- intent: split live authoring state into commands, history, selection, clipboard, and tool-state seams
- interface delta: add child session boundaries matching major old runtime features
- dependencies: painter-document, painter-operations
- consumers: future app/editor surfaces
- artifacts: future session fixtures
- tests: future reducer/history/selection tests
- data: none

### `/home/j/Repos/thaum-painter/domain/painter-operations/`
- action: edit
- intent: split pure operations into brush, shapes, fill, text, and image-import seams
- interface delta: add pure operation boundaries matching the old tool surface
- dependencies: painter-document and session seams as needed
- consumers: painter-session and import flows
- artifacts: future op fixtures
- tests: future operation tests
- data: none

### `/home/j/Repos/thaum-painter/context/`
- action: edit
- intent: record the old feature audit and the new architecture direction before coding
- interface delta: add architecture notes and roadmap updates
- dependencies: old repo inspection
- consumers: humans and future retrieval
- artifacts: feature audit notes
- tests: none
- data: none

## phases
### phase-1
- [x] audit old painter features and map them into bounded responsibilities
- [x] align the repo around the stronger encapsulation split
- [x] capture contract-first direction in context docs

### phase-2
- [x] sort the saved-file ownership seams
- [x] sort the rendering camera and render-space seams
- [x] sort the editing-facing document seams

### phase-3
- [x] sort the live session seams
- [x] sort the pure operation seams
- [x] leave each seam with a crisp ownership boundary before code

### phase-4
- [x] review the whole contract graph for gaps or fused concerns
- [x] prune or rename seams if any are still too broad
- [x] leave the repo ready for piecewise implementation planning

### phase-5
- [ ] validation sweep across contracts and plans
- [ ] update checklist states for whatever actually landed
- [ ] git commit if approved

## post-implementation-notes
- 26-08-29: phase-4's "review the whole contract graph for gaps" surfaced a real missing concept, not just wording drift: `module` (from `thaum-renderer`'s own design truths and the old `module_position_storage.ts`) had no owner. `domain/painter-document/modules/` was added, `domain/file/manifest/`'s saved shape now nests `document.modules[*].groups[]` instead of a flat `document.groups[]`, and `domain/rendering/render-space/` now emits one `cell_groups[]` entry per module with intra-module groups composited in, matching the renderer's truth that a `CellGroup` is "an entire module and relevant contents" and never needs to know about a module itself. Both the manifest and render-space v1 examples/schemas were updated and re-validated (`jsonschema` passes on both), and `context/module-concept-audit.md` records the full reasoning. `context/ascii-painter-feature-audit.md`, `ascii-painter-feature-ownership-matrix.md`, `third-pass-encapsulation-review.md`, `example-alignment-review.md`, and `roadmap.md` were all updated to reflect this rather than left stale.
- 26-08-29: phases 1-4 checkboxes were previously unchecked even though the underlying work was done (third-pass review, example-alignment review, and the audit/matrix docs already existed). Corrected the checklist to match real repo state rather than leaving it stale.
- there is still no git repository for `thaum-painter` (`git init` has not been run here), so none of this work is committed anywhere; treat the working tree itself as the only copy until that's set up.
- 26-08-30: implementation language decided — Rust, not TypeScript. The deciding factor was live rendered preview while editing (a raster-engine-editor requirement): embedding `thaum-renderer` in-process avoids per-frame IPC/serialization latency that a separate TS process talking to the Rust renderer would carry. Scaffolded the Cargo workspace (`Cargo.toml`, `rust-toolchain.toml`, `domain/Cargo.toml`, `domain/lib.rs`) mirroring `thaum-renderer`'s own conventions exactly, and landed the first real implementation: `domain/file/manifest/manifest.rs`, a `serde_json::Value` + `anyhow::Context` manual parser (matching the renderer ecosystem's established style, e.g. `thaum-renderer-test-user`'s scene parser, rather than `serde` derive macros), with inline `#[cfg(test)]` tests against the pinned example fixture. Not compiler-verified in this environment (no `cargo`/`rustc` available here) — needs `cargo test -p thaum-painter-domain` run locally.
- plan-finished: false
