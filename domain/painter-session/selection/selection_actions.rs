//! Painter selection actions: one dispatch seam for the registry's
//! selection-mode and selection-shape actions. Every action that reshapes
//! the selection commits it to the shared document's selection channel
//! here, so the entrypoint never hand-wires the commit sequence.

use std::cell::RefCell;
use std::rc::Rc;

use anyhow::Result;
use thaum_renderer_domain::CellPoint;

use crate::brush::Canvas;
use crate::selection_state::{PainterSelection, SelectionMode};
use crate::session_document::commit_selection_channel;
use crate::storage::{SharedDocumentPaths, SharedDocumentRuntime};

/// Applies one selection action. Returns whether the action was consumed;
/// unknown names are left for the caller's remaining dispatch.
#[allow(clippy::too_many_arguments)] // session-bridge seam: one fn carries the live session state; struct-izing touches the entrypoint
pub fn apply_painter_selection_action(
    action: &str,
    selection: &Rc<RefCell<PainterSelection>>,
    runtime: &mut SharedDocumentRuntime,
    paths: &SharedDocumentPaths,
    action_counter: &mut u64,
    active_layer_id: &mut String,
    current_breath: u32,
    canvas: &mut Canvas,
) -> Result<bool> {
    let mode_action = match action {
        "painter_selection_mode_replace" => Some(SelectionMode::Replace),
        "painter_selection_mode_additive" => Some(SelectionMode::Additive),
        "painter_selection_mode_subtract" => Some(SelectionMode::Subtract),
        "painter_selection_mode_intersect" => Some(SelectionMode::Intersect),
        _ => None,
    };
    if let Some(mode) = mode_action {
        selection.borrow_mut().set_mode(mode);
        return Ok(true);
    }
    match action {
        "painter_selection_clear" => {
            selection.borrow_mut().clear_plane();
            commit_selection_channel(
                runtime,
                paths,
                std::iter::empty(),
                SelectionMode::Replace,
                active_layer_id,
                current_breath,
                canvas,
                selection,
                action_counter,
            );
        }
        "painter_selection_invert" => {
            selection.borrow_mut().invert_plane();
            commit_plane_points(
                runtime,
                paths,
                selection,
                active_layer_id,
                current_breath,
                canvas,
                action_counter,
            );
        }
        "painter_selection_all" => {
            selection.borrow_mut().select_all_plane();
            commit_plane_points(
                runtime,
                paths,
                selection,
                active_layer_id,
                current_breath,
                canvas,
                action_counter,
            );
        }
        _ => return Ok(false),
    }
    Ok(true)
}

/// Shape actions (invert/all) reshape the plane in place and re-commit the
/// full plane point set as one Replace write.
#[allow(clippy::too_many_arguments)]
fn commit_plane_points(
    runtime: &mut SharedDocumentRuntime,
    paths: &SharedDocumentPaths,
    selection: &Rc<RefCell<PainterSelection>>,
    active_layer_id: &mut String,
    current_breath: u32,
    canvas: &mut Canvas,
    action_counter: &mut u64,
) {
    let points: Vec<CellPoint> = selection.borrow().plane().iter().collect();
    commit_selection_channel(
        runtime,
        paths,
        points,
        SelectionMode::Replace,
        active_layer_id,
        current_breath,
        canvas,
        selection,
        action_counter,
    );
}
