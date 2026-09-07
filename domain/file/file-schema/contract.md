# thaum-painter/domain/file/file-schema

## purpose
Own the canonical top-level saved file schema and versioned shape for thaum-painter assets. The schema version only changes when the shape breaks; the version number alone describes the file's generation, which keeps future migrational content easy to store per version.

## owns
- file kind/version markers (`FILE_SCHEMA_KIND`, `FILE_SCHEMA_VERSION`)
- top-level file sections and required keys
- the load-time schema gate truth: a file's kind and version are checked before its body is trusted (gate mechanism wired in `storage/`, owned jointly — see notes)
- canonical placement of painter-only metadata versus renderable content sections
- the stable saved split between document content, time assets, authoring metadata, saved camera defaults, and other painter-only sections
- canonical group identity, group order, and group-level (global-board) placement

## does not own
- persistence transports
- where files live on disk (that is `storage/`)
- legacy migration logic (there is none, by decision — old files reject cleanly)
- renderer handoff mapping

## children-encapsulations
- none

## contents
- `contract.md`
  - file-schema contract
- `file_schema.rs`
  - rust file-schema parsing/validation shapes, owned by this encapsulation (currently still parses the pre-reversal module-wrapped shape; needs an implementation pass, see notes)
- `example-thaum-painter-file-v1.json`
  - v1 generation example (historical: now rejected by the gate — kept as a generation marker)
- `example-thaum-painter-file-v1.md`
  - v1 saved file shape notes
- `schema-thaum-painter-file-v1.json`
  - v1 machine-readable file schema draft (historical)
- `example-thaum-painter-file-v2.json`
  - current-generation saved file example (binary-bars generation)
- `example-thaum-painter-file-v2.md`
  - v2 saved file shape notes (what changed from v1 and why)
- `schema-thaum-painter-file-v2.json`
  - current machine-readable file schema draft

## dependencies
- `thaum-painter/domain/file/`

## exposed interfaces
### parse_file_schema — parse one saved thaum-painter file schema header from an already-decoded JSON value
send: `&serde_json::Value`
returns: `anyhow::Result<FileSchema>`
effects: none
via: rust fn

### parse_file_schema_from_str — parse one saved thaum-painter file schema header directly from its raw JSON text
send: `&str`
returns: `anyhow::Result<FileSchema>`
effects: none
via: rust fn

## interface consumers
- file storage
- file import/export
- painter-document
- render-space

## artifacts
- `example-thaum-painter-file-v2.json`
  - current-generation saved file fixture
- `schema-thaum-painter-file-v2.json`
  - current machine-readable file schema
- `example-thaum-painter-file-v1.json` + `schema-thaum-painter-file-v1.json`
  - v1 generation records (historical; the gate rejects v1 with the explicit version message)

## tests
- inline `#[cfg(test)]` in `file_schema.rs`
  - v2 example parses; v1 example rejects as an unsupported generation; unsupported version and wrong kind reject with explicit messages

## data
- none

## notes
- renamed from `manifest/` (2026-09-07, J-approved): "what's the current schema version" is the sentence the code should be able to say; "manifest" was vague.
- schema version bumped 1 -> 2 (2026-09-07, binary-bars redesign): the break is binary tiling — property tracks tile the full layer span with empty/solid bars, no voids. The interchange shape is unchanged; v1 files are not imported or migrated, they reject cleanly at load (`storage/`'s typed gate). Cross-reference: `plans/project-thaum-painter-binary-bars-plan.md`.
- the schema version changes ONLY when the shape breaks. No importer, no migration pass — old files are recognized as unsupported and rejected cleanly at load, never a crash. J fixes friend files case-by-case.
- load-time gate ownership: `parse_file_schema` provides kind/version validation; `storage/`'s `load_document_file` wires it into the actual load seam so rejection happens before any body deserialization.
