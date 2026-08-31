# /home/j/Repos/thaum-painter/domain/rendering/camera

## purpose
Own app-side camera intent and focus-resolution rules before state is handed to `thaum-renderer`.

## owns
- live editor and preview camera intent
- camera subject resolution from document, selection, text cursor, or tool anchors
- painter-side camera state that is later mapped into renderer-facing values
- policy for how saved camera defaults are combined with live camera overrides

## does not own
- renderer camera internals
- canonical saved file manifest ownership
- general render-space translation

## children-encapsulations
- none

## contents
- `contract.md`
  - camera contract

## dependencies
- `/home/j/Repos/thaum-painter/domain/painter-document/`
- `/home/j/Repos/thaum-painter/domain/painter-session/selection/`
- `/home/j/Repos/thaum-renderer/domain/camera/`

## exposed interfaces
- none

## interface consumers
- render-space
- future preview/editor surfaces

## artifacts
- future camera fixtures

## tests
- future camera resolution tests

## data
- none

## notes
- treat saved camera defaults as file-owned preference data and live camera motion/focus as rendering-owned state.
- the current concrete baseline for saved defaults is `domain/file/manifest/example-thaum-painter-file-v1.json`, and the current concrete handoff target is `domain/rendering/render-space/example-render-space-v1.json`.
- this seam should stay small enough that it can feed render-space without becoming a second broad view/runtime boundary.
