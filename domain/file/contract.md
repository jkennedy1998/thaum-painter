# thaum-painter/domain/file

## purpose
Own the canonical thaum-painter saved file shape, including the file schema and every piece of authoring state the renderer does not own.

## owns
- the app-owned file schema and versioning
- saved asset structure for painter-to-painter and painter-to-game transfer
- authoring metadata, naming, tags, and workflow state not directly consumed by `thaum-renderer`
- storage rules for timing, layer/group organization, import bookkeeping, and other painter-only state
- the split between what is embedded in the file versus derived later for rendering

## does not own
- direct renderer boot or window lifecycle
- low-level renderer cell/group/composition semantics once handed off
- live editing session policy

## children-encapsulations
- `file-schema/`
  - default
- `storage/`
  - default
- `import-export/`
  - default

## contents
- `contract.md`
  - file boundary contract

## dependencies
- `thaum-painter/`
- `thaum-renderer/`

## exposed interfaces
### thaum-painter file shape
send: saved app asset state plus painter-only metadata
returns: normalized file records ready for rendering translation or editing consumption
effects: none
via: contract

## interface consumers
- `thaum-painter/domain/painter-document/`
- `thaum-painter/domain/rendering/render-space/`
- future importer/exporter seams

## artifacts
- `thaum-painter/domain/file/file-schema/example-thaum-painter-file-v1.json`
  - first concrete saved file example for downstream seams

## tests
- future file-shape validation and normalization tests anchored to `domain/file/file-schema/example-thaum-painter-file-v1.json`

## data
- none

## notes
- this boundary should manifest and store all the things the renderer does not have.
- if a field exists only because humans author, revise, transfer, or catalog work, it likely belongs here.
- this seam should stay more encapsulated than the old broad painter-document design.
- sub-boundaries should separate file shape, persistence surfaces, import/export wrappers, ect.
- the current concrete saved-truth reference is `domain/file/file-schema/example-thaum-painter-file-v1.json`.
