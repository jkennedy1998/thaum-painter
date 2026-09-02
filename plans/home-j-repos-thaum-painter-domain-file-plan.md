# file encapsulation plan

## current-state
The first pass spread saved asset thinking across painter-document and renderer-bridge ideas. This repo now needs a dedicated `domain/file/` seam so the saved app asset manifest and all non-renderer state have one owner.

## target-encapsulation
- `/home/j/Repos/thaum-painter/domain/file/`: add

## artifacts
- future example saved painter asset files

## tests
- future file-shape validation and normalization tests

## data
- none

## phases
### phase-1
- [ ] align contract notes with file ownership of painter-only stored state
- [ ] note the old broad painter-document responsibilities that should move here

### phase-2
- [x] define first-pass file top-level sections
- [x] define manifest/versioning fields
- [x] define stored authoring metadata and transfer bookkeeping

### phase-3
- [x] define which fields are embedded canonical data versus derived later for rendering
- [x] define normalization notes for future parser/serializer work

### phase-4
- [x] validate the shape against painter-to-painter and painter-to-game use cases
- [x] leave implementation-ready notes for render-space and document consumption

### phase-5
- [ ] final validation sweep
- [x] tests/proofs as available — `manifest.rs` inline `#[cfg(test)]` tests now exist for `domain/file/manifest/`; `storage/`, `import-export/`, and `migration/` still have no implementation
- [ ] git commit if approved

## post-implementation-notes
- 26-08-30: `domain/file/manifest/` got its first real implementation — `manifest.rs`, a Rust parser/validator (`parse_manifest`/`parse_manifest_from_str`) covering the full saved-file shape including the two-level module/group nesting, with inline tests against the pinned `example-thaum-painter-file-v1.json` fixture. `domain/file/storage/`, `domain/file/import-export/`, and `domain/file/migration/` remain contract-only. This was written without a working `cargo`/`rustc` in the authoring environment — not compiler-verified here, needs `cargo test -p thaum-painter-domain` run locally to confirm.
- 26-08-31: the two-level module/group nesting from 26-08-30 is reversed — see `context/module-concept-audit.md`'s "superseded" section. `document.groups[]` is flat again, one group per renderer `CellGroup` directly; `domain/painter-document/modules/` is removed. Contract updated to describe the flat shape; `manifest.rs` and the v1 example/schema JSON still implement the old wrapper and need a code pass.
