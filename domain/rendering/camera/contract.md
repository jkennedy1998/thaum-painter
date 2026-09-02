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
- which physical keys/mouse gestures move the camera, or the action-binding/remap system that resolves them — owned by `thaum-renderer/domain/controls/`

## children-encapsulations
- none

## contents
- `contract.md`
  - camera contract
- `camera_viewport.rs`
  - app-side camera→plane mapping and viewport-scroll intent (`CanvasBounds` derivation from the live camera, HUD/drawing-space wheel handling, viewport-centered swing/roll)

## dependencies
- `/home/j/Repos/thaum-painter/domain/painter-document/`
- `/home/j/Repos/thaum-painter/domain/painter-session/selection/`
- `/home/j/Repos/thaum-renderer/domain/camera/`
- `/home/j/Repos/thaum-renderer/domain/controls/`

## exposed interfaces
- none

## interface consumers
- render-space
- future preview/editor surfaces

## artifacts
- future camera fixtures

## tests
- future camera resolution tests
- `camera_viewport` inline tests: bounds follow HUD pan offset, swing/roll keep the viewport-center world point under the viewport center, HUD and drawing-space scroll directions

## data
- none

## notes
- treat saved camera defaults as file-owned preference data and live camera motion/focus as rendering-owned state.
- the current concrete baseline for saved defaults is `domain/file/manifest/example-thaum-painter-file-v1.json`, and the current concrete handoff target is `domain/rendering/render-space/example-render-space-v1.json`.
- this seam should stay small enough that it can feed render-space without becoming a second broad view/runtime boundary.
- camera has one source of truth in the renderer already (position/focus target/swing/roll/zoom); what changed by this seam is *how a program moves it*. Painter should declare its own named actions (e.g. `camera_pan_left`) against `thaum-renderer/domain/controls/`'s `ActionBindingMap` and bind them to whatever keys/scroll/drag gesture painter wants (remappable), then call `thaum-renderer/interfaces/camera/`'s movement operations when those actions fire — this seam does not hardcode keys itself.
