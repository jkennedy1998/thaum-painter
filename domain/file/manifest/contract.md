# /home/j/Repos/thaum-painter/domain/file/manifest

## purpose
Own the canonical top-level saved file manifest and versioned shape for thaum-painter assets.

## owns
- file kind/version markers
- top-level file sections and required keys
- canonical placement of painter-only metadata versus renderable content sections
- the stable saved split between document content, time assets, authoring metadata, saved camera defaults, and other painter-only sections
- canonical module identity, module order, and module-level (global-board) placement
- canonical intra-module group storage, nested one level under its owning module

## does not own
- persistence transports
- legacy migration logic
- renderer handoff mapping

## children-encapsulations
- none

## contents
- `contract.md`
  - manifest contract
- `manifest.rs`
  - rust manifest parsing/validation shapes, owned by this encapsulation
- `example-thaum-painter-file-v1.json`
  - first implementation-ready saved file example
- `example-thaum-painter-file-v1.md`
  - saved file shape notes
- `schema-thaum-painter-file-v1.json`
  - first machine-readable manifest schema draft

## dependencies
- `/home/j/Repos/thaum-painter/domain/file/`

## exposed interfaces
### parse_manifest — parse one saved thaum-painter file manifest from an already-decoded JSON value
send: `&serde_json::Value`
returns: `anyhow::Result<Manifest>`
effects: none
via: rust fn

### parse_manifest_from_str — parse one saved thaum-painter file manifest directly from its raw JSON text
send: `&str`
returns: `anyhow::Result<Manifest>`
effects: none
via: rust fn

## interface consumers
- file storage
- file import/export
- painter-document
- render-space

## artifacts
- `example-thaum-painter-file-v1.json`
  - first-pass saved file fixture
- `schema-thaum-painter-file-v1.json`
  - first-pass machine-readable manifest schema

## tests
- `manifest.rs`'s inline `#[cfg(test)]` module
  - light
  - parses `example-thaum-painter-file-v1.json` (two modules, intra-module groups, raster segments, property blocks, time assets, saved camera defaults, import/export bookkeeping) and validates kind/version/required-field rejection.

## data
- none

## notes
- the manifest should stay explicit about which sections are saved truth versus which states are derived later for rendering.
- preferred direction: keep saved camera defaults here, but keep live camera motion/focus outside the file.
- if a future field exists mainly to help humans save, catalog, migrate, or reopen work, it likely belongs in this boundary rather than render-space.
- `document.groups[]` moved to `document.modules[*].groups[]`: one saved module maps to exactly one renderer cell-group, and groups are intra-module content, not top-level document content; see `context/module-concept-audit.md`.
- a group's `placement` field is now `local_placement`, relative to its owning module's local coordinate space; the module itself carries the global-board `placement`.
- implemented in Rust (`serde_json::Value` + manual field-by-field parsing with `anyhow::Context`, matching `thaum-renderer`'s own convention, e.g. its `thaum-renderer-test-user/src/main.rs` scene parser) rather than `serde` derive macros, for consistency with the sibling renderer repo.
- this was written without a working `cargo`/`rustc` in the authoring environment, so it has not been compiler-verified here; run `cargo test -p thaum-painter-domain` locally to confirm before relying on it.
