# /home/j/Repos/thaum-painter/domain/rendering/render-space

## purpose
Own the render-space handoff that converts thaum-painter file or live state into the scene/state payload consumed by `thaum-renderer`.

## owns
- translation from painter file state into renderer-facing render state
- rules for which saved file fields become direct renderer input
- app-side derived render-space state that exists only to feed the renderer cleanly
- normalization of camera/module/cell/data-lane handoff before renderer boot or frame updates
- the module-to-cell-group mapping: exactly one renderer cell-group per authored module
- compositing a module's intra-module groups (local placement, timing/property resolution, overlap) into that module's single cell-group cell field before handoff

## does not own
- the saved file manifest itself
- renderer-internal rendering logic after handoff
- editor session history or tool behavior

## children-encapsulations
- none

## contents
- `contract.md`
  - render-space contract
- `example-render-space-v1.json`
  - first implementation-ready render-space handoff example
- `example-render-space-v1.md`
  - render-space mapping notes
- `schema-render-space-v1.json`
  - first machine-readable render-space schema draft

## dependencies
- `/home/j/Repos/thaum-painter/domain/file/`
- `/home/j/Repos/thaum-renderer/domain/cell/`
- `/home/j/Repos/thaum-renderer/domain/cell-group/`
- `/home/j/Repos/thaum-renderer/domain/composition/`
- `/home/j/Repos/thaum-renderer/domain/coordinate-space/`
- `/home/j/Repos/thaum-renderer/domain/data-lanes/`
- `/home/j/Repos/thaum-renderer/orchestration/boot/`

## exposed interfaces
### render-space handoff
send: thaum-painter file state or live app state
returns: renderer-ready scene/state payload
effects: none
via: contract

## interface consumers
- future painter app boot flow
- future editor preview/runtime surfaces

## artifacts
- `example-render-space-v1.json`
  - first-pass render-space handoff fixture
- `schema-render-space-v1.json`
  - first-pass machine-readable render-space schema

## tests
- future render-space translation tests against `example-render-space-v1.json`
- future schema checks against `schema-render-space-v1.json`

## data
- none

## notes
- this is the seam where the render file or live state sends data to `thaum-renderer`.
- if a field is only needed for storage or authoring, keep it in `domain/file/` and derive renderer-facing state here.
- if a field is directly consumed by the renderer, this boundary should map it through cleanly instead of burying it inside painter-document.
- keep this seam focused on handoff assembly, not long-lived saved truth and not live editor policy.
- if render-space starts absorbing too many concerns, split it into child seams such as scene assembly, timing/data-lane mapping, preview-only derivation, or visible group resolution.
- `thaum-renderer`'s own design truth is that a `CellGroup` is "an entire module and relevant contents" and the renderer does not need to know about a module at all — see `/home/j/Repos/thaum-painter/context/module-concept-audit.md`. This boundary is where that mapping actually happens: one authored module in, one `cell_groups[]` entry out, with intra-module groups already composited.
