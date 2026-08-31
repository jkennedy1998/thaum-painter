# /home/j/Repos/thaum-painter/domain/painter-document

## purpose
Own the painter editing-facing document view over saved file state without taking ownership of the saved file manifest or the renderer handoff seam.

## owns
- editing-facing document semantics over saved file state
- document-level conveniences for browsing and manipulating painter content during authoring
- document normalization and validation rules that help editor/session flows
- painter-side document views or projections that should not become the canonical saved file manifest

## does not own
- canonical saved file manifest ownership
- renderer composition semantics
- renderer camera semantics
- renderer handoff shaping owned by `domain/rendering/render-space/`
- UI interaction behavior

## children-encapsulations
- `modules/`
  - default
- `groups/`
  - default
- `timing/`
  - default
- `properties/`
  - default

## contents
- `contract.md`
  - painter-document contract

## dependencies
- `/home/j/Repos/thaum-painter/domain/file/`

## exposed interfaces
### painter document view
send: saved file state plus authoring context
returns: normalized editing-facing document records
effects: none
via: contract

## interface consumers
- future painter session/runtime
- future editor surfaces

## artifacts
- future document-view fixtures derived from `domain/file/manifest/example-thaum-painter-file-v1.json`

## tests
- future document-view tests derived from `domain/file/manifest/example-thaum-painter-file-v1.json`

## data
- none

## notes
- `domain/file/` should own the saved manifest and painter-only stored state.
- `domain/rendering/render-space/` should own how file or live state sends data to `thaum-renderer`.
- painter-document should stay narrower: it is the editing-facing document view, not the save-file owner and not the renderer bridge owner.
- the current saved-truth input reference for this seam is `domain/file/manifest/example-thaum-painter-file-v1.json`.
- module lookup, group lookup, property lookup, and timing lookup should likely land as child seams rather than one monolithic document file.
- `modules/` was added after the module concept audit (`context/module-concept-audit.md`) showed one saved module maps to exactly one renderer cell-group, and groups are intra-module content, not top-level document content.
