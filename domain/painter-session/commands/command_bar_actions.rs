//! Command-bar menu buttons and their session actions: module visibility
//! toggles and the file:new/open/save/save-as document flows.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use anyhow::Result;

use thaum_renderer_domain::{Canvas, CommandBar, CommandBarButton, ModuleRegistry};

use thaum_painter_file_dialogs::{prompt_open_document_root, prompt_save_document_root};

use crate::document_locations::{
    load_document_from_root, new_unsaved_document, resolve_painter_file_root,
};
use crate::layers_runtime::resolved_active_layer_id;
use crate::selection_state::PainterSelection;
use crate::session_document::sync_canvas_from_active_layer;
use crate::storage::{
    save_shared_document_snapshot, SharedDocumentPaths, SharedDocumentRuntime,
};

pub fn file_menu_buttons() -> Vec<CommandBarButton> {
    vec![
        CommandBarButton::new("file:new", "NEW"),
        CommandBarButton::new("file:open", "OPEN"),
        CommandBarButton::new("file:save", "SAVE"),
        CommandBarButton::new("file:save-as", "SAVE AS"),
    ]
}

pub fn module_menu_buttons(modules: &ModuleRegistry) -> Vec<CommandBarButton> {
    [
        ("paint_canvas_bounds", "DRAWING SPACE"),
        ("painter_toolbox", "TOOLS"),
        ("painter_color_picker", "COLOR PICKER"),
        ("painter_color_block", "COLOR BLOCK"),
        ("painter_material_picker", "MATERIALS"),
        ("painter_graphic_picker", "GRAPHICS"),
        ("painter_hand_settings", "PROPS"),
        ("painter_ui_customization", "UI COLORS"),
    ]
    .into_iter()
    .map(|(module_id, label)| {
        let prefix = if modules.is_hidden(module_id).unwrap_or(false) {
            "+"
        } else {
            "-"
        };
        CommandBarButton::new(format!("module:{module_id}"), format!("{prefix} {label}"))
    })
    .collect()
}

pub fn handle_command_bar_button(
    button_id: &str,
    modules: &mut ModuleRegistry,
    shared_document: &mut SharedDocumentRuntime,
    shared_document_paths: &mut SharedDocumentPaths,
    repo_root: &std::path::Path,
    artifacts_root: &std::path::Path,
    current_document_root: &mut Option<PathBuf>,
    active_layer_id: &mut String,
    selected_property_id: &mut Option<String>,
    shared_action_counter: &mut u64,
    current_breath: u32,
    canvas: &mut Canvas,
    selection: &Rc<RefCell<PainterSelection>>,
) -> Result<()> {
    if let Some(module_id) = button_id.strip_prefix("module:") {
        if let Some(hidden) = modules.is_hidden(module_id) {
            modules.set_hidden(module_id, !hidden);
        }
        return Ok(());
    }
    let file_root = resolve_painter_file_root(repo_root);
    match button_id {
        "file:new" => {
            *shared_document = new_unsaved_document();
            *shared_document_paths = crate::document_locations::painter_shared_document_paths(
                artifacts_root,
                &shared_document.document.document_id,
            );
            *current_document_root = None;
            *active_layer_id = resolved_active_layer_id(shared_document, None);
            *selected_property_id = None;
            *shared_action_counter = 0;
            sync_canvas_from_active_layer(shared_document, active_layer_id, current_breath, canvas);
            selection.borrow_mut().clear_plane();
        }
        "file:open" => {
            if let Some(next_root) = prompt_open_document_root(&file_root) {
                let (next_paths, next_document) = load_document_from_root(&next_root)?;
                *shared_document_paths = next_paths;
                *shared_document = next_document;
                *current_document_root = Some(next_root);
                *active_layer_id = resolved_active_layer_id(shared_document, Some(active_layer_id));
                *selected_property_id = None;
                *shared_action_counter = shared_document.actions.len() as u64;
                sync_canvas_from_active_layer(shared_document, active_layer_id, current_breath, canvas);
                selection.borrow_mut().clear_plane();
            }
        }
        "file:save" => {
            if current_document_root.is_none() {
                if let Some(next_root) = prompt_save_document_root(&file_root, &shared_document.document.title) {
                    *shared_document_paths = SharedDocumentPaths::new(next_root.clone());
                    *current_document_root = Some(next_root);
                } else {
                    return Ok(());
                }
            }
            save_shared_document_snapshot(shared_document_paths, shared_document)?;
        }
        "file:save-as" => {
            if let Some(next_root) = prompt_save_document_root(&file_root, &shared_document.document.title) {
                let next_paths = SharedDocumentPaths::new(next_root.clone());
                save_shared_document_snapshot(&next_paths, shared_document)?;
                *shared_document_paths = next_paths;
                *current_document_root = Some(next_root);
            }
        }
        _ => {}
    }
    Ok(())
}
