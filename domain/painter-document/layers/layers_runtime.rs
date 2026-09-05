//! Layer session runtime: active-layer resolution, layer creation, layers-panel
//! property-row projection, and application of pending layers-panel actions to
//! the real document/session.

use std::cell::RefCell;
use std::rc::Rc;

use crate::brush::Canvas;
use crate::document_locations::INITIAL_LAYER_ID;
use crate::layers_panel_module::{
    LayerPropertyKind, LayersPanelAction, MergeDirection, PropertyTrackBlock, PropertyTrackRow,
};
use crate::selection_state::PainterSelection;
use crate::session_document::{
    action_timestamp_string, append_and_apply_shared_action, next_action_id,
    recover_snapshot_conflict, sync_canvas_from_active_layer,
};
use crate::storage::{
    save_shared_document_snapshot, PropertyBlockMergeDirection, SharedDocumentPaths,
    SharedDocumentRuntime,
};
use crate::timeline_state::TimelineState;

pub fn resolved_active_layer_id(
    runtime: &SharedDocumentRuntime,
    preferred_layer_id: Option<&str>,
) -> String {
    if let Some(layer_id) =
        preferred_layer_id.filter(|layer_id| runtime.document.has_layer(layer_id))
    {
        return layer_id.to_string();
    }
    runtime
        .document
        .first_layer_id()
        .unwrap_or(INITIAL_LAYER_ID)
        .to_string()
}

pub fn next_layer_number(runtime: &SharedDocumentRuntime) -> usize {
    let mut number = 1;
    loop {
        if !runtime.document.has_layer(&format!("layer-{number}")) {
            return number;
        }
        number += 1;
    }
}

pub fn create_layer(runtime: &mut SharedDocumentRuntime) -> String {
    let number = next_layer_number(runtime);
    let layer_id = format!("layer-{number}");
    let layer_name = format!("Layer {number}");
    runtime.add_layer(layer_id.clone(), layer_name);
    layer_id
}

pub fn build_selected_layer_property_rows(
    shared_document: &SharedDocumentRuntime,
    active_layer_id: &str,
) -> Vec<PropertyTrackRow> {
    let Some(layer) = shared_document
        .layers()
        .iter()
        .find(|layer| layer.layer_id == active_layer_id)
    else {
        return Vec::new();
    };
    let raster_blocks = shared_document
        .property_track(active_layer_id, "raster")
        .map(|track| {
            track
                .blocks
                .iter()
                .map(|block| PropertyTrackBlock {
                    id: block.id.clone(),
                    start_breath: block.start_breath,
                    length_breaths: block.length_breaths,
                    is_blank: block.is_blank,
                })
                .collect()
        })
        .unwrap_or_else(|| {
            vec![PropertyTrackBlock {
                id: format!("{}:raster:0", layer.layer_id),
                start_breath: layer.start_breath,
                length_breaths: layer.length_breaths,
                is_blank: false,
            }]
        });
    let move_blocks = shared_document
        .property_track(active_layer_id, "move")
        .map(|track| {
            track
                .blocks
                .iter()
                .map(|block| PropertyTrackBlock {
                    id: block.id.clone(),
                    start_breath: block.start_breath,
                    length_breaths: block.length_breaths,
                    is_blank: block.is_blank,
                })
                .collect()
        })
        .unwrap_or_default();

    vec![
        PropertyTrackRow {
            layer_id: layer.layer_id.clone(),
            property_id: "raster".to_string(),
            label: "RASTER".to_string(),
            kind: LayerPropertyKind::Raster,
            blocks: raster_blocks,
        },
        PropertyTrackRow {
            layer_id: layer.layer_id.clone(),
            property_id: "move".to_string(),
            label: "MOVE".to_string(),
            kind: LayerPropertyKind::Move,
            blocks: move_blocks,
        },
    ]
}

/// Applies one pending `LayersPanelModule` action (if any) to the real document/session. Called
/// both after a click and after every captured drag-move frame, since ruler scrubbing queues a
/// new action on each frame it is dragged.
#[allow(clippy::too_many_arguments)] // session-bridge seam: one fn carries the live session state; struct-izing touches the entrypoint
pub fn apply_layers_panel_action(
    action: Option<LayersPanelAction>,
    shared_document: &mut SharedDocumentRuntime,
    shared_document_paths: &SharedDocumentPaths,
    shared_action_counter: &mut u64,
    session_user_id: &str,
    active_layer_id: &mut String,
    selected_property_id: &mut Option<String>,
    canvas: &mut Canvas,
    timeline_state: &Rc<RefCell<TimelineState>>,
    selection: &Rc<RefCell<PainterSelection>>,
) {
    let Some(action) = action else { return };
    let current_breath = timeline_state.borrow().current_breath;
    // Pure-UI actions (selection, playhead, auto-key) never touch the document;
    // everything else mutates document metadata, so the snapshot is rewritten right
    // after applying. Without this, block/layer edits only reached disk on an explicit
    // file:save and were lost whenever the app closed first.
    let document_mutated = !matches!(
        action,
        LayersPanelAction::Select(_)
            | LayersPanelAction::SelectProperty(..)
            | LayersPanelAction::ToggleAutoKey
            | LayersPanelAction::SetCurrentBreath(_)
            | LayersPanelAction::TogglePlay
            | LayersPanelAction::ToggleLoop
    );
    match action {
        LayersPanelAction::Select(layer_id) => {
            *active_layer_id = resolved_active_layer_id(shared_document, Some(&layer_id));
            *selected_property_id = None;
            sync_canvas_from_active_layer(shared_document, active_layer_id, current_breath, canvas);
        }
        LayersPanelAction::SelectProperty(layer_id, property_id) => {
            *active_layer_id = resolved_active_layer_id(shared_document, Some(&layer_id));
            *selected_property_id = Some(property_id);
            sync_canvas_from_active_layer(shared_document, active_layer_id, current_breath, canvas);
        }
        LayersPanelAction::AddRequested => {
            *active_layer_id = create_layer(shared_document);
            *selected_property_id = None;
            sync_canvas_from_active_layer(shared_document, active_layer_id, current_breath, canvas);
        }
        LayersPanelAction::ToggleVisible(layer_id) => {
            if let Some(layer) = shared_document
                .layers()
                .iter()
                .find(|layer| layer.layer_id == layer_id)
            {
                let next_visible = !layer.visible;
                shared_document.set_layer_visible(&layer_id, next_visible);
            }
        }
        LayersPanelAction::ToggleLocked(layer_id) => {
            if let Some(layer) = shared_document
                .layers()
                .iter()
                .find(|layer| layer.layer_id == layer_id)
            {
                let next_locked = !layer.locked;
                shared_document.set_layer_locked(&layer_id, next_locked);
            }
        }
        LayersPanelAction::Delete(layer_id) => {
            if shared_document.layers().len() > 1 && shared_document.remove_layer(&layer_id) {
                if *active_layer_id == layer_id {
                    *active_layer_id = resolved_active_layer_id(shared_document, None);
                    *selected_property_id = None;
                }
                sync_canvas_from_active_layer(
                    shared_document,
                    active_layer_id,
                    current_breath,
                    canvas,
                );
            }
        }
        LayersPanelAction::ToggleAutoKey => {
            timeline_state.borrow_mut().toggle_auto_key();
        }
        LayersPanelAction::TogglePlay => {
            let window = shared_document.document_window();
            let mut timeline = timeline_state.borrow_mut();
            timeline.toggle_play();
            // Starting playback from outside the window snaps to its start.
            if timeline.playing
                && (timeline.current_breath < window.start_breath
                    || timeline.current_breath > window.end_breath)
            {
                timeline.set_current_breath(window.start_breath);
                drop(timeline);
                sync_canvas_from_active_layer(
                    shared_document,
                    active_layer_id,
                    window.start_breath,
                    canvas,
                );
            }
        }
        LayersPanelAction::ToggleLoop => {
            timeline_state.borrow_mut().toggle_loop();
        }
        LayersPanelAction::SetCurrentBreath(breath) => {
            timeline_state.borrow_mut().set_current_breath(breath);
            // Scrubbing the playhead switches which raster block the edit surface shows.
            sync_canvas_from_active_layer(shared_document, active_layer_id, breath, canvas);
        }
        LayersPanelAction::SetLayerTiming(layer_id, start_breath, length_breaths) => {
            shared_document.set_layer_timing(&layer_id, start_breath, length_breaths);
        }
        LayersPanelAction::SetLoopWindow(start_breath, end_breath) => {
            shared_document.set_document_window(start_breath, end_breath);
        }
        LayersPanelAction::SetPropertyBlockTiming(
            layer_id,
            property_id,
            block_id,
            start_breath,
            length_breaths,
        ) => {
            shared_document.set_property_block_timing(
                &layer_id,
                &property_id,
                &block_id,
                start_breath,
                length_breaths,
            );
        }
        LayersPanelAction::SetPropertyBlockTimingPushed(
            layer_id,
            property_id,
            block_id,
            start_breath,
            length_breaths,
        ) => {
            shared_document.set_property_block_timing_pushed(
                &layer_id,
                &property_id,
                &block_id,
                start_breath,
                length_breaths,
            );
        }
        LayersPanelAction::SetPropertyBlockTimingDestructive(
            layer_id,
            property_id,
            block_id,
            start_breath,
            length_breaths,
        ) => {
            shared_document.set_property_block_timing_destructive(
                &layer_id,
                &property_id,
                &block_id,
                start_breath,
                length_breaths,
            );
        }
        LayersPanelAction::SplitPropertyBlock(layer_id, property_id, block_id, split_breath) => {
            let Some(new_block_id) = shared_document.split_property_block(
                &layer_id,
                &property_id,
                &block_id,
                split_breath,
            ) else {
                return;
            };
            *selected_property_id = Some(property_id.clone());
            // Propagate the split block's channel data onto the new half as a recorded
            // patch: both halves start as identical copies and replay rebuilds the copy.
            if let Some(record) = shared_document.split_data_propagation_record(
                &layer_id,
                &block_id,
                &new_block_id,
                next_action_id(shared_action_counter),
                session_user_id,
                action_timestamp_string(),
            ) {
                let _ =
                    append_and_apply_shared_action(shared_document, shared_document_paths, record);
            }
            sync_canvas_from_active_layer(shared_document, active_layer_id, current_breath, canvas);
        }
        LayersPanelAction::BlankPropertyBlock(layer_id, property_id, block_id) => {
            shared_document.blank_property_block(&layer_id, &property_id, &block_id);
        }
        LayersPanelAction::MergeBlankPropertyBlock(layer_id, property_id, block_id, direction) => {
            let direction = match direction {
                MergeDirection::Left => PropertyBlockMergeDirection::Left,
                MergeDirection::Right => PropertyBlockMergeDirection::Right,
            };
            shared_document.merge_blank_property_block(
                &layer_id,
                &property_id,
                &block_id,
                direction,
            );
        }
        LayersPanelAction::SwapPropertyBlocks(
            layer_id,
            property_id,
            source_block_id,
            target_block_id,
        ) => {
            shared_document.swap_property_blocks(
                &layer_id,
                &property_id,
                &source_block_id,
                &target_block_id,
            );
        }
    }
    if document_mutated {
        if let Err(error) = save_shared_document_snapshot(shared_document_paths, shared_document) {
            recover_snapshot_conflict(
                &error,
                shared_document,
                shared_document_paths,
                active_layer_id,
                current_breath,
                canvas,
                selection,
                shared_action_counter,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::SharedDocumentSelection;
    use crate::storage::{
        DocumentWindow, SharedDocumentFile, SharedDocumentLayer, SHARED_DOCUMENT_KIND,
        SHARED_DOCUMENT_SCHEMA_VERSION,
    };

    fn document_with_layers(layer_ids: &[&str]) -> SharedDocumentFile {
        SharedDocumentFile {
            file_kind: SHARED_DOCUMENT_KIND.to_string(),
            schema_version: SHARED_DOCUMENT_SCHEMA_VERSION,
            document_id: "doc-1".to_string(),
            title: "Doc".to_string(),
            layers: layer_ids
                .iter()
                .map(|id| SharedDocumentLayer {
                    layer_id: id.to_string(),
                    name: format!("Layer {id}"),
                    visible: true,
                    locked: false,
                    start_breath: 0,
                    length_breaths: 24,
                    property_tracks: vec![],
                })
                .collect(),
            revision: 0,
            selection: SharedDocumentSelection::default(),
            document_window: DocumentWindow::default(),
        }
    }

    #[test]
    fn resolved_active_layer_id_falls_back_to_first_document_layer() {
        let runtime = SharedDocumentRuntime::new(document_with_layers(&["layer-a", "layer-b"]));

        assert_eq!(
            resolved_active_layer_id(&runtime, Some("missing")),
            "layer-a"
        );
    }

    #[test]
    fn create_layer_picks_the_next_open_layer_number() {
        let mut runtime = SharedDocumentRuntime::new(document_with_layers(&["layer-1", "layer-3"]));

        assert_eq!(create_layer(&mut runtime), "layer-2");
    }
}
