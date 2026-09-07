# thaum-painter/domain/modules/individuals/layers-panel

## purpose
Own the one always-attached panel through which J browses, selects, and (over future passes) fully manipulates the document's layers and their animation over time.

## owns
- the layer list: display order, current selection, an "add layer" affordance, per-layer visibility/lock toggles, delete
- the panel's own timeline header: an auto-key toggle plus a scrubbable breath ruler/playhead with visible breath labels, reflecting `painter-session/timeline-state/`
- the layer list row chrome: current selection plus visibility/lock/delete affordances, with per-property animation tracks nested under the selected layer
- the panel's own draw/hit-test/gizmo wiring, implementing `thaum-renderer`'s `Module` trait directly

## does not own
- canonical layer storage (`domain/file/storage/`) or the saved layer schema (`domain/file/file-schema/`)
- the layer document view (`domain/painter-document/layers/`)
- the timeline-state playhead/auto-key resolution logic itself (`painter-session/timeline-state/`) — this module only reads/writes a mirrored breath+auto-key readout and queues actions for the orchestration layer to apply back onto the real `TimelineState`
- gating compositing/visibility by `SharedDocumentLayer` timing fields — those ranges are document data today, but not a first-class visible authoring bar in this panel pass
- destructive edge-drag semantics that consume/rewrite overlapping neighbor blocks and refill exposed gaps with new blanks (the old system's `set_group_property_block_edge_destructive`) — destructive timing now routes through the shared `domain/painter-document/properties/` seam, but only as one commit on pointer-up: the drag itself never mutates the document, the dragged bar previews at its requested span, and the release commit resolves covered victims (truncate/vanish/split) in a single write
- layer rename and drag-based reorder — rename needs a `Module` trait text-input/keyboard hook that does not exist yet (a cross-repo `thaum-renderer` change); reorder is deliberately deferred alongside it
- document-level frames-per-breath — needs a playback-settings seam on the live document schema first, mirroring the `visible`/`locked`/timing gaps that were already closed for layers

## children-encapsulations
- none

## contents
- `contract.md`
  - layers-panel contract
- `layers_panel_module.rs`
  - `LayersPanelModule`, `LayersPanelState`, `LayerRow`, `LayersPanelAction` — the layer list (select, add, delete, visibility/lock toggle), a timeline header (auto-key toggle, scrubbable breath ruler/playhead, visible breath labels), and nested property rows (raster/move) for the selected layer; the caller (orchestration) applies the requested action to the real document/session and syncs `LayersPanelState` back each frame

## dependencies
- `thaum-renderer/domain/modules/`
- `thaum-renderer/domain/modules/shared/`
- `thaum-painter/domain/painter-document/properties/pieces/` — the bar-piece × cell-type classifier the 48-branch interaction matrix routes through

## exposed interfaces
### layer list + timeline header + nested property-track module
send: the current ordered layer rows (id, name, visible, locked, start_breath, length_breaths), selected id, current breath, and auto-key flag (via `LayersPanelState::sync`), plus pointer/wheel events routed by the module registry
returns: at most one pending `LayersPanelAction` per frame (`Select`, `SelectProperty`, `AddRequested`, `ToggleVisible`, `ToggleLocked`, `Delete`, `ToggleAutoKey`, `SetCurrentBreath`, `SetPropertyBlockTiming`, `BlankPropertyBlock`, `SwapPropertyBlocks`, `DuplicatePropertyBlock`, or `MergeEmptyPropertyBlock`), taken via `LayersPanelState::take_pending_action`
effects: none directly — the orchestration layer applies the action to the real document (`SharedDocumentRuntime`) or session (`TimelineState`)
via: `LayersPanelModule`, `LayersPanelState`

## interface consumers
- `thaum-painter/orchestration/entrypoint/`

## artifacts
- none

## tests
- inline `#[cfg(test)]` in `layers_panel_module.rs`
  - light
  - validates the 48-branch piece × cell-type interaction matrix (`context/bars-binary-design-truth.md`, J dictation 2026-09-07), routed off the `properties/pieces/` classifier: every interaction first queues `SelectProperty` (selection rule — "unused" branches mean no additional action, never no selection); drag branches (single move/resize, head pushed/destructive trims, center swaps/slides for both cell types) preview mid-flight without mutating the document and commit exactly once on pointer-up
  - double-click branches: double-left on a solid single or center queues `DuplicatePropertyBlock` (non-destructive right-landing push); double-right on a solid single or center queues `BlankPropertyBlock`; double-right on a solid left/right head merges the adjacent bar in as one `SetPropertyBlockTimingDestructive` (rejected at the span edge, mirrored for right heads); double-right on any empty queues `MergeEmptyPropertyBlock` (left-preferred seam, right heads mirror; rejected on a fully-empty track at the storage seam); double-left on empties and on solid heads are unused — reserved for keyframe interpolation (J-flip 2026-09-07: right-click owns the delete/merge family)

## data
- none

## notes
- closing the schema gaps: `domain/file/storage/`'s `SharedDocumentLayer` now carries `visible`/`locked`/`start_breath`/`length_breaths` fields (all defaulted for backward-compatible deserialization) and `SharedDocumentRuntime` gained `set_layer_visible`, `set_layer_locked`, `rename_layer`, `remove_layer`, and `set_layer_timing`; `composited_canvas_in_layer_order` skips invisible layers.
- the panel is wide enough (`ModuleRect { x0: 25, y0: -24, x1: 70, y1: -3 }` in `main.rs`) to hold a left-hand icon/name column plus a real timeline region, mirroring the old system's `GROUPS_TIMELINE_LEFT_OFFSET` idea of a fixed left offset before the timeline starts. `TIMELINE_START` in `layers_panel_module.rs` is that offset; the header ruler and nested property rows share the same `timeline_bounds()`/`x_for_breath`/`breath_at_x` mapping so they line up column-for-column.
- the visible timeline now leans on the old module's read shape: labeled start/current/end breaths above a scrubbable ruler, while the editable bars live on the nested raster/move rows rather than on the layer title row itself.
- the breath ruler currently maps the panel's visible timeline columns directly to breaths `0..span` with no separate pan/zoom view-window concept (the old system's `get_timeline_view_start`/`get_timeline_view_span`) — scrubbing past the visible span still sets the real breath correctly, it just cannot be seen yet. Adding a pannable/zoomable view window is left for a later pass.
- follows the same `Rc<RefCell<...>>`-shared-state pattern as `tool_state`/`ToolboxModule`: the module never touches `SharedDocumentRuntime` or `TimelineState` directly, it only reads/writes its own small `LayersPanelState`, so it stays unit-testable without any document/session plumbing. The orchestration layer (`orchestration/entrypoint/src/main.rs`'s `apply_layers_panel_action`) is the sole place that touches both.
- the old bottom command-bar "LAYERS" menu (`layer_menu_buttons` in `main.rs`) has been removed now that this module is the real landing zone for layer interaction.
- this module is the eventual home for per-property animation tracks nested under each layer, described in the layers-panel design; those attach to this same module in a later pass rather than becoming a separate module.
- binary-bars interaction pass (26-09-07): `resolve_property_drag_mode` is rewritten piece × cell-type keyed per the 48-branch matrix — solid singles destructively move (left) / destructively resize through either side (right); empty singles are unused on left and destructively resize on right; heads (both cell types) push on left and overwrite destructively on right; solid centers swap on left and destructively slide on right; empty centers swap on left (predictability, J) and are unused on right. The old size-keyed modes die: no `SplitPropertyBlock` action (double-left duplicates instead), no `MergeBlankPropertyBlock`, and the scrub-on-blank-center behavior is gone (scrubbing lives on the ruler row). Hit-testing classifies through `properties/pieces/` (`classify_bar_piece` + `cell_type_of`) instead of the old `PropertyBlockHitMode` enum.
