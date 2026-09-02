# /home/j/Repos/thaum-painter/domain/modules/individuals/layers-panel

## purpose
Own the one always-attached panel through which J browses, selects, and (over future passes) fully manipulates the document's layers and their animation over time.

## owns
- the layer list: display order, current selection, an "add layer" affordance, per-layer visibility/lock toggles, delete
- the panel's own timeline header: an auto-key toggle plus a scrubbable breath ruler/playhead with visible breath labels, reflecting `painter-session/timeline-state/`
- the layer list row chrome: current selection plus visibility/lock/delete affordances, with per-property animation tracks nested under the selected layer
- the panel's own draw/hit-test/gizmo wiring, implementing `thaum-renderer`'s `Module` trait directly

## does not own
- canonical layer storage (`domain/file/storage/`) or the saved layer schema (`domain/file/manifest/`)
- the layer document view (`domain/painter-document/layers/`)
- the timeline-state playhead/auto-key resolution logic itself (`painter-session/timeline-state/`) — this module only reads/writes a mirrored breath+auto-key readout and queues actions for the orchestration layer to apply back onto the real `TimelineState`
- gating compositing/visibility by `SharedDocumentLayer` timing fields — those ranges are document data today, but not a first-class visible authoring bar in this panel pass
- destructive edge-drag semantics that consume/rewrite overlapping neighbor blocks and refill exposed gaps with new blanks (the old system's `set_group_property_block_edge_destructive`) — this pass's drag/trim/dynamic-resize only reshapes the one dragged block via `set_property_block_timing`, it never touches neighbors, so overlapping a neighbor by dragging into it is not yet prevented or resolved
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
- `/home/j/Repos/thaum-renderer/domain/modules/`
- `/home/j/Repos/thaum-renderer/domain/modules/shared/`

## exposed interfaces
### layer list + timeline header + nested property-track module
send: the current ordered layer rows (id, name, visible, locked, start_breath, length_breaths), selected id, current breath, and auto-key flag (via `LayersPanelState::sync`), plus pointer/wheel events routed by the module registry
returns: at most one pending `LayersPanelAction` per frame (`Select`, `SelectProperty`, `AddRequested`, `ToggleVisible`, `ToggleLocked`, `Delete`, `ToggleAutoKey`, `SetCurrentBreath`, `SetPropertyBlockTiming`, `SplitPropertyBlock`, `BlankPropertyBlock`, `MergeBlankPropertyBlock`, or `SwapPropertyBlocks`), taken via `LayersPanelState::take_pending_action`
effects: none directly — the orchestration layer applies the action to the real document (`SharedDocumentRuntime`) or session (`TimelineState`)
via: `LayersPanelModule`, `LayersPanelState`

## interface consumers
- `/home/j/Repos/thaum-painter/orchestration/entrypoint/`

## artifacts
- none

## tests
- inline `#[cfg(test)]` in `layers_panel_module.rs`
  - light
  - validates: clicking a layer name queues `Select`; clicking add queues `AddRequested`; clicking the visibility/lock icons queues `ToggleVisible`/`ToggleLocked`; clicking the delete icon queues `Delete`; clicking the auto-key row queues `ToggleAutoKey`; clicking the ruler queues `SetCurrentBreath` and starts a scrub-drag whose subsequent `Move` events keep queuing updated breaths until `Up`; clicking a property row queues `SelectProperty`; raster/move blocks render and edit in-place (move, trim, split); clicking timeline space on the layer row still selects that layer; clicking outside any row queues nothing; the gizmo close click hides the panel
  - right double-clicking a content block queues `BlankPropertyBlock`; right double-clicking a blank's left/right half queues `MergeBlankPropertyBlock` toward the clicked side; left-click-dragging a blank's center scrubs the timeline instead of starting a block drag; right-click-dragging a block's body previews a swap against whatever block the pointer is over and commits `SwapPropertyBlocks` on release (or queues nothing if released over empty space); right-click-dragging a block's left edge can flip from trimming the start to growing the end, depending on which side of the press the pointer moves to

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
- old-system click/drag parity pass (26-09-02): `resolve_property_drag_mode` mirrors the old `resolve_groups_raster_drag_mode` — content blocks move/trim on left-click and, on right-click, a single-breath block or the left edge does a "dynamic resize" that grows/shrinks from whichever side the pointer moves toward (flipping through the block if dragged past the opposite side), the right edge always just trims (no dynamic flip on that side, matching the old system as authored, not by design intent), and the body does a swap-drag against whatever block the pointer ends up over on release. Blanks only drag via a right-click swap on their center; left-click-dragging a blank scrubs the timeline instead. Right double-click blanks a content block (`BlankPropertyBlock`) or merges a blank into its left/right content neighbor (`MergeBlankPropertyBlock`), split by which half of the blank was clicked (mirrors `getBlankCompactDirection`). Left double-click on a content body still splits it. Known gap vs. the old system: edge/dynamic-resize drags reshape only the dragged block (`set_property_block_timing`) and do not consume/rewrite an overlapping neighbor or refill any exposed gap with a new blank, unlike the old system's "destructive" edge semantics — see "does not own".
