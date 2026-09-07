//! Session→document bridge: how live editing session state (staged strokes,
//! undo/redo, selection edits) becomes persisted shared-document actions.

use std::collections::BTreeSet;

use anyhow::Result;

use thaum_renderer_domain::{CameraViewOrientation, CellPoint, WorldPoint};

use crate::brush::{Canvas, PaintedCell};
use crate::fill::CanvasBounds;
use crate::layers_runtime::resolved_active_layer_id;
use crate::selection_state::{PainterSelection, SelectionMode};
use crate::storage::{
    append_action_record, save_shared_document_snapshot, SharedCellPatch,
    SharedDocumentActionRecord, SharedDocumentPaths, SharedDocumentRuntime,
    SharedSelectionWriteMode, DEFAULT_SELECTION_CHANNEL_ID,
};
use crate::tool_state::{PaintHand, ToolState};

pub fn action_timestamp_string() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .to_string()
}

/// Mints one globally-unique action id. The `user_id` is baked in (Figma's
/// client-id-in-object-id rule): two sessions minting offline can never
/// collide, and revert records referencing an action id stay unambiguous.
pub fn next_action_id(counter: &mut u64, user_id: &str) -> String {
    *counter += 1;
    format!("action-{user_id}-{}-{counter}", action_timestamp_string())
}

pub fn collect_canvas_patches(before: &Canvas, after: &Canvas) -> Vec<SharedCellPatch> {
    let positions: BTreeSet<_> = before.keys().chain(after.keys()).copied().collect();
    positions
        .into_iter()
        .filter_map(|position| {
            let before_cell = before.get(&position);
            let after_cell = after.get(&position);
            if before_cell == after_cell {
                None
            } else {
                Some(SharedCellPatch::new(position, before_cell, after_cell))
            }
        })
        .collect()
}

pub fn append_and_apply_shared_action(
    runtime: &mut SharedDocumentRuntime,
    paths: &SharedDocumentPaths,
    action: SharedDocumentActionRecord,
) -> Result<()> {
    append_action_record(&paths.actions_file_path, &action)?;
    runtime.apply_action_record(action);
    Ok(())
}

pub fn sync_canvas_from_active_layer(
    runtime: &SharedDocumentRuntime,
    active_layer_id: &str,
    current_breath: u32,
    canvas: &mut Canvas,
) {
    *canvas = runtime
        .canvas_for_layer(active_layer_id, current_breath)
        .cloned()
        .unwrap_or_default();
}

/// Stages one paint chunk onto the live canvases WITHOUT creating an action record.
/// Strokes commit once at release (one undo per stroke); the runtime canvas is staged
/// so per-frame compositing shows the work in progress.
#[allow(clippy::too_many_arguments)] // session-bridge seam: one fn carries the live session state; struct-izing touches the entrypoint
pub fn stage_image_edit_chunk(
    runtime: &mut SharedDocumentRuntime,
    canvas: &mut Canvas,
    tool_state: &mut ToolState,
    selection: &mut crate::selection_state::PainterSelection,
    positions: impl IntoIterator<Item = CellPoint>,
    hand: PaintHand,
    bounds: CanvasBounds,
    orientation: CameraViewOrientation,
    layer_id: &str,
    block_id: &str,
) {
    let before = canvas.clone();
    let mut candidate = before.clone();
    for position in positions {
        tool_state.apply_at_for_hand(
            &mut candidate,
            selection,
            position,
            hand,
            bounds,
            orientation,
        );
    }
    let patches = collect_canvas_patches(&before, &candidate);
    if patches.is_empty() {
        return;
    }
    runtime.stage_canvas_patches(layer_id, block_id, &patches);
    // Adopt the candidate instead of re-cloning from the runtime: the staged
    // canvas now holds exactly this content.
    *canvas = candidate;
}

/// Stages resolved painted-cell changes onto the live canvases WITHOUT
/// creating an action record — live preview, exactly like a tool painting
/// into the grid. Pending changes commit as one record through
/// [`commit_staged_paint_stroke`] on release (one undo per stroke).
pub fn stage_painted_cells_chunk(
    runtime: &mut SharedDocumentRuntime,
    canvas: &mut Canvas,
    changes: impl IntoIterator<Item = (CellPoint, Option<PaintedCell>)>,
    layer_id: &str,
    block_id: &str,
) {
    let before = canvas.clone();
    let mut candidate = before.clone();
    for (point, change) in changes {
        match change {
            Some(cell) => candidate.insert(point, cell),
            None => candidate.remove(&point),
        };
    }
    let patches = collect_canvas_patches(&before, &candidate);
    if patches.is_empty() {
        return;
    }
    runtime.stage_canvas_patches(layer_id, block_id, &patches);
    *canvas = candidate;
}

/// Stages one text-entry cell change (char insert or erase) onto the live
/// canvases WITHOUT creating an action record — per-keystroke live preview,
/// exactly like the old tool painting into the grid as the user types. Pending
/// changes commit as one 'Type Text' record through
/// [`commit_staged_paint_stroke`] at Enter/exit.
pub fn stage_text_entry_change(
    runtime: &mut SharedDocumentRuntime,
    canvas: &mut Canvas,
    change: (CellPoint, Option<PaintedCell>),
    layer_id: &str,
    block_id: &str,
) {
    stage_painted_cells_chunk(runtime, canvas, [change], layer_id, block_id);
}

/// Commits one finished stroke as a single `CellPatchSet` record — one undo per
/// stroke. The runtime canvas already holds the staged content, so applying the
/// record is idempotent; it only registers the undo bookkeeping.
#[allow(clippy::too_many_arguments)] // session-bridge seam: one fn carries the live session state; struct-izing touches the entrypoint
pub fn commit_staged_paint_stroke(
    runtime: &mut SharedDocumentRuntime,
    paths: &SharedDocumentPaths,
    action_counter: &mut u64,
    user_id: &str,
    active_layer_id: &str,
    canvas: &mut Canvas,
    stroke_start: Option<(Canvas, String)>,
) -> Result<()> {
    let Some((start_canvas, block_id)) = stroke_start else {
        return Ok(());
    };
    let patches = collect_canvas_patches(&start_canvas, canvas);
    if patches.is_empty() {
        return Ok(());
    }
    let record = SharedDocumentActionRecord::cell_patch_set(
        next_action_id(action_counter, user_id),
        runtime.document.document_id.clone(),
        active_layer_id,
        user_id,
        action_timestamp_string(),
        patches,
        Some(block_id),
    );
    append_action_record(&paths.actions_file_path, &record)?;
    runtime.apply_action_record(record);
    Ok(())
}

/// Recovers from a snapshot-save conflict (any error mentioning `changed on disk`):
/// the other writer's on-disk truth wins, so the runtime reloads from disk and every
/// live mirror of document state is resynced — active layer, canvas, the selection
/// channel cache, and the action counter. Local unsaved edits are discarded by
/// design; they were refused anyway. Other errors are just logged. Returns whether
/// a reload happened.
#[allow(clippy::too_many_arguments)] // session-bridge seam: one fn carries the live session state; struct-izing touches the entrypoint
pub fn recover_snapshot_conflict(
    error: &anyhow::Error,
    runtime: &mut SharedDocumentRuntime,
    paths: &SharedDocumentPaths,
    active_layer_id: &mut String,
    current_breath: u32,
    canvas: &mut Canvas,
    selection: &std::rc::Rc<std::cell::RefCell<PainterSelection>>,
    action_counter: &mut u64,
) -> bool {
    if !error.to_string().contains("changed on disk") {
        crate::debug_log::error(
            "storage",
            &format!("failed to save document snapshot: {error}"),
        );
        return false;
    }
    crate::debug_log::warn(
        "storage",
        "shared document changed on disk; reloading the other writer's version",
    );
    if let Err(reload_error) = runtime.reload_from_disk(paths) {
        crate::debug_log::error(
            "storage",
            &format!("failed to reload shared document: {reload_error}"),
        );
        return false;
    }
    crate::debug_log::info(
        "storage",
        "reloaded the shared document from disk; local unsaved edits were discarded",
    );
    *active_layer_id = resolved_active_layer_id(runtime, Some(active_layer_id));
    sync_canvas_from_active_layer(runtime, active_layer_id, current_breath, canvas);
    let channel_points = runtime.selection_points(DEFAULT_SELECTION_CHANNEL_ID);
    selection.borrow_mut().replace_points(channel_points);
    *action_counter = runtime.actions.len() as u64;
    true
}

/// Mirrors a committed selection change into the document's selection channel and
/// persists it. Selection is document-owned (per file, one shared 3D bitmap on the
/// canvas coordinate system), so the plane cache inside `PainterSelection` is only
/// the interaction surface; the channel is the truth. No-ops skip the snapshot save.
#[allow(clippy::too_many_arguments)] // session-bridge seam: one fn carries the live session state; struct-izing touches the entrypoint
pub fn commit_selection_channel<I>(
    runtime: &mut SharedDocumentRuntime,
    paths: &SharedDocumentPaths,
    points: I,
    mode: SelectionMode,
    active_layer_id: &mut String,
    current_breath: u32,
    canvas: &mut Canvas,
    selection: &std::rc::Rc<std::cell::RefCell<PainterSelection>>,
    action_counter: &mut u64,
) where
    I: IntoIterator<Item = CellPoint>,
{
    let write_mode = match mode {
        SelectionMode::Replace => SharedSelectionWriteMode::Replace,
        SelectionMode::Additive => SharedSelectionWriteMode::Additive,
        SelectionMode::Subtract => SharedSelectionWriteMode::Subtract,
        SelectionMode::Intersect => SharedSelectionWriteMode::Intersect,
    };
    if runtime.apply_selection_points(DEFAULT_SELECTION_CHANNEL_ID, points, write_mode) {
        if let Err(error) = save_shared_document_snapshot(paths, runtime) {
            recover_snapshot_conflict(
                &error,
                runtime,
                paths,
                active_layer_id,
                current_breath,
                canvas,
                selection,
                action_counter,
            );
        }
    }
}

/// Writes one vector move commit: the drag's world delta added to the
/// active layer's move block covering `current_breath` (auto-creating the
/// move track and a block spanning the layer's timing window when missing),
/// then the snapshot save. Property-offset edits persist like the other
/// property-track metadata edits — through the document snapshot, not the
/// per-cell undo log.
pub fn commit_move_offset(
    runtime: &mut SharedDocumentRuntime,
    paths: &SharedDocumentPaths,
    active_layer_id: &str,
    delta: WorldPoint,
    current_breath: u32,
) -> Result<()> {
    if runtime.add_move_offset(active_layer_id, current_breath, delta) {
        save_shared_document_snapshot(paths, runtime)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)] // session-bridge seam: one fn carries the live session state; struct-izing touches the entrypoint
pub fn apply_shared_history_action(
    runtime: &mut SharedDocumentRuntime,
    paths: &SharedDocumentPaths,
    action_counter: &mut u64,
    user_id: &str,
    active_layer_id: &str,
    canvas: &mut Canvas,
    undo: bool,
    current_breath: u32,
) -> Result<()> {
    // Undo/redo are persisted as passive revert records (normal CellPatchSets that
    // paint content but skip the undo stacks) — the all-forward-edits log keeps
    // squash safe and matches the undo-as-operation multiplayer model.
    let revert = if undo {
        runtime.undo_top_action(active_layer_id)
    } else {
        runtime.redo_top_action(active_layer_id)
    };
    if let Some(revert) = revert {
        let record = SharedDocumentActionRecord::revert_patch_set(
            next_action_id(action_counter, user_id),
            runtime.document.document_id.clone(),
            active_layer_id,
            user_id,
            action_timestamp_string(),
            revert.patches,
            Some(revert.block_id),
            revert.action_id,
        );
        append_action_record(&paths.actions_file_path, &record)?;
        // The undo/redo already mutated the canvases; the record is history-only,
        // so push without re-applying (the canonical in-session flow the storage
        // tests validate).
        runtime.push_history_record(record);
    }
    sync_canvas_from_active_layer(runtime, active_layer_id, current_breath, canvas);
    Ok(())
}
