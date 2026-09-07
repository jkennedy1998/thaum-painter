# thaum painter file v2 example

## intent
This is the current-generation saved file example for `domain/file/file-schema/`, pinned to schema version 2 (the binary-bars generation).

## top-level sections
Identical structure to the v1 example (`kind`, `version`, `metadata`, `document`, `time_assets`, `saved_camera_defaults`, `import_export_bookkeeping`).

## what changed from v1
- `version` is 2. The schema version only changes when the shape breaks; the number alone describes the file's generation.
- The break is the binary-bars redesign: property tracks are binary (empty/solid bars tiling the full layer span, no voids). The interchange shape itself is unchanged — blocks are solid content only, and the breaths between blocks are empties by definition. A v1 file's saved shape cannot be trusted to carry that invariant, so v1 files are not imported or migrated; the load-time gate rejects them with the explicit found-vs-supported message.
- Old v1 files are handled case by case, manually, by J (`context/bars-binary-design-truth.md`).

## consumers
- parse/gate tests in `file_schema.rs` pin this file as the current-generation parse baseline.
- the v1 example stays in this folder as a historical generation marker; parsing it now rejects with the version-difference message by design.
