use std::{
    cell::RefCell,
    env, fs,
    io::{self, Write},
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use rfd::FileDialog;
use thaum_renderer_domain::NumberFieldEdit;
use thaum_painter_domain::{
    camera_viewport::{
        apply_drawing_space_scroll, apply_hud_scroll, pan_hud_and_focus_right,
        pan_hud_and_focus_up, reorient_camera_around_viewport_center, sync_canvas_bounds_to_camera,
        INITIAL_PAINT_CANVAS_BOUNDS, INITIAL_PAINT_CANVAS_VIEWPORT,
    }, document_locations::{
        load_document_from_root, new_unsaved_document, resolve_painter_file_root,
    }, layers_runtime::{
        apply_layers_panel_action, build_selected_layer_property_rows, resolved_active_layer_id,
    }, render_space::build_document_layer_cell_groups, save_shared_document_snapshot, selection_stroke::{
        build_plane_selection_cell_groups, interpolate_cell_path, SelectionStroke,
    }, lasso_stroke::{build_lasso_path_cell_group, build_lasso_preview_cell_groups, LassoStroke}, session_document::{
        commit_selection_channel, commit_staged_paint_stroke,
        apply_shared_history_action, recover_snapshot_conflict, stage_image_edit_chunk,
        stage_text_entry_change, sync_canvas_from_active_layer,
    }, text_entry::{cursor_overlay_group, TextEntryKey, TextEntryOutcome, TextEntryState}, Canvas, DrawingSpaceWheelMode, GraphicPickerModule, HandSettingsModule, LayerRow, LayersPanelModule, LayersPanelState,
    MaterialPickerModule, PaintCanvasBoundsModule, PaintColorBlockModule,
    PaintColorPickerModule, PaintHand,
    PaintTarget, PaintTool, PainterSelection, PainterUserSessionState, PersistedPainterUiState, SelectionMode, SharedDocumentPaths,
    SharedDocumentRuntime, TimelineState,
    ToolDef, ToolState, ToolboxModule, DEFAULT_SELECTION_CHANNEL_ID,
};
use thaum_renderer_boot::{
    boot_renderer, cell_clip_size_for_state, run_renderer_window_with_state_frame_provider,
    BootConfig, BootState,
};
use thaum_renderer_domain::{
    camera_view_orientation_for_camera, remap_surface_units_to_active_plane_world,
    remap_surface_units_to_flat_2d_local, ActionBindingMap, ActionName, CellPoint, CommandBar,
    CommandBarButton, CommandBarClickOutcome, Composition, ControlActionRow, ControlsPanelModule,
    ControlsProfile, conflicting_actions, effective_bindings, format_raw_input,
    ModulePointerButton, ModulePointerEvent, ModuleRect, ModuleRegistry,
    PersistedRendererUiSessionState, RawInput, TypingMode, TypingRoute, UiColorRole,
    UiCustomizationModule, UiPalette,
};
use winit::keyboard::KeyCode;

fn development_asset_root() -> PathBuf {
    if let Ok(path) = env::var("THAUM_RENDERER_ASSET_ROOT") {
        return PathBuf::from(path);
    }

    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../thaum-renderer/orchestration/renderer-assets")
}

fn session_user_id() -> String {
    env::var("THAUM_SESSION_USER_ID")
        .or_else(|_| env::var("USER"))
        .unwrap_or_else(|_| "local-user".to_string())
}

fn painter_session_state_path(user_id: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../artifacts/user-session-state")
        .join(format!("{user_id}.json"))
}

fn painter_shared_document_paths(document_id: &str) -> SharedDocumentPaths {
    SharedDocumentPaths::new(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../artifacts/shared-documents")
            .join(document_id),
    )
}

fn painter_repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn painter_file_root() -> PathBuf {
    resolve_painter_file_root(&painter_repo_root())
}

fn slugify_file_stem(text: &str) -> String {
    let mut slug = String::new();
    let mut last_was_dash = false;
    for ch in text.chars() {
        let lower = ch.to_ascii_lowercase();
        if lower.is_ascii_alphanumeric() {
            slug.push(lower);
            last_was_dash = false;
        } else if !last_was_dash {
            slug.push('-');
            last_was_dash = true;
        }
    }
    slug.trim_matches('-').to_string()
}

fn native_file_dialog_backend_name() -> &'static str {
    #[cfg(target_os = "linux")]
    {
        "XDG desktop portal"
    }
    #[cfg(target_os = "windows")]
    {
        "Windows native dialog"
    }
    #[cfg(target_os = "macos")]
    {
        "macOS native dialog"
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        "native dialog"
    }
}

fn display_path(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
}

fn log_missing_dialog_selection(action: &str, file_root: &Path) {
    thaum_painter_domain::debug_log::warn(
        "dialogs",
        &format!(
            "{action} dialog returned no selection. backend={}. root={}",
            native_file_dialog_backend_name(),
            display_path(file_root)
        ),
    );
    #[cfg(target_os = "linux")]
    thaum_painter_domain::debug_log::warn(
        "dialogs",
        "on Linux this backend depends on a live desktop-portal session; if no dialog appears, check xdg-desktop-portal / DBus availability in the current desktop session.",
    );
}

fn prompt_path_in_terminal(prompt: &str, file_root: &Path) -> Option<PathBuf> {
    eprintln!("thaum-painter: {prompt}");
    eprintln!("thaum-painter: press Enter to cancel");
    eprint!("thaum-painter path [{}]: ", display_path(file_root));
    let _ = io::stderr().flush();
    let _ = io::stdout().flush();
    let mut line = String::new();
    if io::stdin().read_line(&mut line).ok()? == 0 {
        return None;
    }
    let trimmed = line.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(PathBuf::from(trimmed))
    }
}

fn normalize_open_document_root(path: &Path) -> PathBuf {
    if path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
    {
        return path.parent().unwrap_or(path).to_path_buf();
    }
    path.to_path_buf()
}

fn prompt_open_document_root(file_root: &Path) -> Option<PathBuf> {
    let selected = FileDialog::new()
        .set_directory(file_root)
        .set_title("Open thaum-painter document")
        .add_filter("Thaum painter document", &["json"])
        .pick_file()
        .map(|path| normalize_open_document_root(&path));
    if let Some(path) = selected {
        return Some(path);
    }
    log_missing_dialog_selection("open", file_root);
    prompt_path_in_terminal(
        "native open dialog unavailable; type a document.json path or a document folder path",
        file_root,
    )
    .map(|path| normalize_open_document_root(&path))
}

fn save_as_root_from_dialog_path(path: &Path) -> PathBuf {
    if path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("document.json"))
    {
        return path.parent().unwrap_or(path).to_path_buf();
    }

    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(slugify_file_stem)
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| "untitled-document".to_string());
    path.parent()
        .unwrap_or(path)
        .join(stem)
}

fn prompt_save_document_root(file_root: &Path, title: &str) -> Option<PathBuf> {
    let suggested = slugify_file_stem(title);
    let suggested = if suggested.is_empty() {
        "untitled-document".to_string()
    } else {
        suggested
    };
    let selected = FileDialog::new()
        .set_directory(file_root)
        .set_title("Save thaum-painter document as")
        .set_file_name(&format!("{suggested}.json"))
        .save_file()
        .map(|path| save_as_root_from_dialog_path(&path));
    if let Some(path) = selected {
        return Some(path);
    }
    log_missing_dialog_selection("save", file_root);
    prompt_path_in_terminal(
        "native save dialog unavailable; type a target document.json path or a folder/name path",
        file_root,
    )
    .map(|path| save_as_root_from_dialog_path(&path))
}

fn load_painter_user_session_state(path: &Path) -> Result<Option<PainterUserSessionState>> {
    if !path.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(path).with_context(|| {
        format!(
            "failed to read painter user session state at {}",
            path.display()
        )
    })?;
    let state = serde_json::from_str(&text).with_context(|| {
        format!(
            "failed to parse painter user session state JSON at {}",
            path.display()
        )
    })?;
    Ok(Some(state))
}

fn save_painter_user_session_state(path: &Path, state: &PainterUserSessionState) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create painter user session state directory {}",
                parent.display()
            )
        })?;
    }
    let text = serde_json::to_string_pretty(state)
        .context("failed to serialize painter user session state")?;
    fs::write(path, text).with_context(|| {
        format!(
            "failed to write painter user session state at {}",
            path.display()
        )
    })
}

fn file_menu_buttons() -> Vec<CommandBarButton> {
    vec![
        CommandBarButton::new("file:new", "NEW"),
        CommandBarButton::new("file:open", "OPEN"),
        CommandBarButton::new("file:save", "SAVE"),
        CommandBarButton::new("file:save-as", "SAVE AS"),
    ]
}

fn module_menu_buttons(modules: &ModuleRegistry) -> Vec<CommandBarButton> {
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

fn handle_command_bar_button(
    button_id: &str,
    modules: &mut ModuleRegistry,
    shared_document: &mut SharedDocumentRuntime,
    shared_document_paths: &mut SharedDocumentPaths,
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
    let file_root = painter_file_root();
    match button_id {
        "file:new" => {
            *shared_document = new_unsaved_document();
            *shared_document_paths = painter_shared_document_paths(&shared_document.document.document_id);
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
        "file:save-as" => {
            if let Some(next_root) = prompt_save_document_root(&file_root, &shared_document.document.title) {
                let next_paths = SharedDocumentPaths::new(next_root.clone());
                if let Err(error) = save_shared_document_snapshot(&next_paths, shared_document) {
                    recover_snapshot_conflict(
                        &error,
                        shared_document,
                        &next_paths,
                        active_layer_id,
                        current_breath,
                        canvas,
                        selection,
                        shared_action_counter,
                    );
                }
                *shared_document_paths = next_paths;
                *current_document_root = Some(next_root);
            }
        }
        _ => {}
    }
    Ok(())
}

fn build_user_session_state(
    user_id: &str,
    camera: thaum_renderer_domain::Camera,
    modules: &ModuleRegistry,
    ui_palette: &UiPalette,
    command_bar: &CommandBar,
    active_layer_id: Option<&str>,
    current_breath: u32,
    drawing_space_wheel_mode: DrawingSpaceWheelMode,
    selection_mode: SelectionMode,
    tool_state: &ToolState,
    controls_profile: &ControlsProfile,
) -> PainterUserSessionState {
    PainterUserSessionState {
        schema_version: 1,
        app_id: "thaum-painter".to_string(),
        user_id: user_id.to_string(),
        workspace_id: "default-workspace".to_string(),
        renderer: PersistedRendererUiSessionState::new(
            camera,
            modules.persisted_ui_state(),
            ui_palette,
        ),
        painter: PersistedPainterUiState::from_runtime(
            drawing_space_wheel_mode,
            selection_mode,
            command_bar,
            active_layer_id,
            current_breath,
            tool_state,
        ),
        controls_profile: controls_profile.clone(),
    }
}

fn sync_renderer_background_from_ui_palette(state: &mut BootState, ui_palette: &UiPalette) {
    let [red, green, blue] = ui_palette.get_rgb(UiColorRole::Background);
    state.config.window.clear_color = [
        red as f64 / 255.0,
        green as f64 / 255.0,
        blue as f64 / 255.0,
        1.0,
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolate_cell_path_fills_every_step_between_two_points() {
        let points = interpolate_cell_path(
            CellPoint { x: 1, y: 1, z: 0 },
            CellPoint { x: 4, y: 4, z: 0 },
        );
        assert_eq!(
            points,
            vec![
                CellPoint { x: 1, y: 1, z: 0 },
                CellPoint { x: 2, y: 2, z: 0 },
                CellPoint { x: 3, y: 3, z: 0 },
                CellPoint { x: 4, y: 4, z: 0 },
            ]
        );
    }

    #[test]
    fn painter_file_root_defaults_to_repo_context_folder() {
        let root = painter_file_root();
        assert!(root.ends_with("context/painter/painter-files"));
        assert!(!root.to_string_lossy().contains("/orchestration/context/"));
    }

    #[test]
    fn save_as_root_from_dialog_path_uses_document_parent_for_document_json() {
        let root = save_as_root_from_dialog_path(Path::new("/tmp/example/document.json"));
        assert_eq!(root, PathBuf::from("/tmp/example"));
    }

    #[test]
    fn save_as_root_from_dialog_path_uses_slugged_file_stem_for_other_names() {
        let root = save_as_root_from_dialog_path(Path::new("/tmp/example/My Sketch.json"));
        assert_eq!(root, PathBuf::from("/tmp/example/my-sketch"));
    }

    #[test]
    fn normalize_open_document_root_uses_parent_for_file_input() {
        let root = normalize_open_document_root(Path::new("/tmp/example/document.json"));
        assert_eq!(root, PathBuf::from("/tmp/example"));
    }

    #[test]
    fn normalize_open_document_root_keeps_directory_input() {
        let root = normalize_open_document_root(Path::new("/tmp/example-folder"));
        assert_eq!(root, PathBuf::from("/tmp/example-folder"));
    }

    #[test]
    fn typing_reserved_inputs_cover_the_camera_and_depth_bindings_only() {
        let reserved = typing_reserved_inputs(&thaum_painter_domain::tai::painter_bindings());
        let labels: Vec<&str> = reserved
            .iter()
            .map(|input| match input {
                RawInput::Key(label) => label.as_str(),
                _ => panic!("camera/depth bindings are key bindings"),
            })
            .collect();
        assert_eq!(
            labels,
            vec![
                "NUMPAD4", "NUMPAD6", "NUMPAD8", "NUMPAD2", "NUMPAD7", "NUMPAD9", "NUMPAD1",
                "NUMPAD3",
            ]
        );
    }

    #[test]
    fn raw_key_labels_cover_the_keys_the_registry_binds() {
        assert_eq!(raw_key_label(KeyCode::KeyP).as_deref(), Some("P"));
        assert_eq!(raw_key_label(KeyCode::KeyB).as_deref(), Some("B"));
        assert_eq!(raw_key_label(KeyCode::KeyZ).as_deref(), Some("Z"));
        assert_eq!(raw_key_label(KeyCode::Numpad4).as_deref(), Some("NUMPAD4"));
        assert_eq!(
            raw_key_label(KeyCode::NumpadAdd).as_deref(),
            Some("NUMPAD_ADD")
        );
    }

    #[test]
    fn live_painter_actions_stay_in_sync_with_the_registry() {
        let bindings = thaum_painter_domain::tai::painter_bindings();
        let registry_names: Vec<&str> = bindings
            .actions()
            .map(|action| action.0.as_str())
            .collect();
        for name in LIVE_PAINTER_ACTIONS {
            assert!(
                registry_names.contains(name),
                "live action {name} must stay registered in painter_bindings()"
            );
        }
        for name in &registry_names {
            assert!(
                LIVE_PAINTER_ACTIONS.contains(&name),
                "registry action {name} must stay handled by the live dispatch"
            );
        }
    }

    #[test]
    fn registry_painter_hotkeys_resolve_to_live_tool_dispatch() {
        let bindings = thaum_painter_domain::tai::painter_bindings();
        let pencil_actions = painter_key_actions_for_label(&bindings, "P");
        assert_eq!(pencil_actions.len(), 1, "P resolves to exactly one action");
        assert_eq!(
            painter_tool_for_action(&pencil_actions[0]),
            Some(PaintTool::Brush)
        );
        let bucket_actions = painter_key_actions_for_label(&bindings, "B");
        assert_eq!(bucket_actions.len(), 1, "B resolves to exactly one action");
        assert_eq!(
            painter_tool_for_action(&bucket_actions[0]),
            Some(PaintTool::Fill)
        );
        let lasso_actions = painter_key_actions_for_label(&bindings, "L");
        assert_eq!(lasso_actions.len(), 1, "L resolves to exactly one action");
        assert_eq!(
            painter_tool_for_action(&lasso_actions[0]),
            Some(PaintTool::Lasso)
        );
        // The zoom key resolves through the registry too (remappable), and
        // only the wheel binding stays off the key path.
        assert_eq!(
            painter_key_actions_for_label(&bindings, "-")[0].0.as_str(),
            "painter_zoom_out"
        );
        assert!(
            painter_key_actions_for_label(&bindings, "wheel_only").is_empty(),
            "unbound labels resolve to nothing"
        );
    }

    #[test]
    fn every_action_the_key_dispatch_fires_exists_in_the_registry() {
        let bindings = thaum_painter_domain::tai::painter_bindings();
        for name in [
            "painter_select_pencil",
            "painter_select_bucket",
            "painter_select_lasso",
        ] {
            assert!(
                !bindings
                    .bindings_for(&ActionName::new(name))
                    .is_empty(),
                "dispatch action {name} must stay registered in painter_bindings()"
            );
        }
    }
}

/// Physical-key label used by the renderer's `RawInput::Key` bindings, for the
/// key classes the painter dispatches on. Unmapped keys simply resolve to no
/// registry action and fall through to the live-only match arms.
fn raw_key_label(key: KeyCode) -> Option<String> {
    let label = match key {
        KeyCode::KeyA => "A",
        KeyCode::KeyB => "B",
        KeyCode::KeyC => "C",
        KeyCode::KeyD => "D",
        KeyCode::KeyE => "E",
        KeyCode::KeyF => "F",
        KeyCode::KeyG => "G",
        KeyCode::KeyH => "H",
        KeyCode::KeyI => "I",
        KeyCode::KeyJ => "J",
        KeyCode::KeyK => "K",
        KeyCode::KeyL => "L",
        KeyCode::KeyM => "M",
        KeyCode::KeyN => "N",
        KeyCode::KeyO => "O",
        KeyCode::KeyP => "P",
        KeyCode::KeyQ => "Q",
        KeyCode::KeyR => "R",
        KeyCode::KeyS => "S",
        KeyCode::KeyT => "T",
        KeyCode::KeyU => "U",
        KeyCode::KeyV => "V",
        KeyCode::KeyW => "W",
        KeyCode::KeyX => "X",
        KeyCode::KeyY => "Y",
        KeyCode::KeyZ => "Z",
        KeyCode::Digit0 => "0",
        KeyCode::Digit1 => "1",
        KeyCode::Digit2 => "2",
        KeyCode::Digit3 => "3",
        KeyCode::Digit4 => "4",
        KeyCode::Digit5 => "5",
        KeyCode::Digit6 => "6",
        KeyCode::Digit7 => "7",
        KeyCode::Digit8 => "8",
        KeyCode::Digit9 => "9",
        KeyCode::Minus => "-",
        KeyCode::Equal => "=",
        KeyCode::Numpad0 => "NUMPAD0",
        KeyCode::Numpad1 => "NUMPAD1",
        KeyCode::Numpad2 => "NUMPAD2",
        KeyCode::Numpad3 => "NUMPAD3",
        KeyCode::Numpad4 => "NUMPAD4",
        KeyCode::Numpad5 => "NUMPAD5",
        KeyCode::Numpad6 => "NUMPAD6",
        KeyCode::Numpad7 => "NUMPAD7",
        KeyCode::Numpad8 => "NUMPAD8",
        KeyCode::Numpad9 => "NUMPAD9",
        KeyCode::NumpadAdd => "NUMPAD_ADD",
        KeyCode::NumpadSubtract => "NUMPAD_SUB",
        KeyCode::Enter | KeyCode::NumpadEnter => "ENTER",
        KeyCode::Escape => "ESCAPE",
        KeyCode::ArrowLeft => "LEFT",
        KeyCode::ArrowRight => "RIGHT",
        KeyCode::ArrowUp => "UP",
        KeyCode::ArrowDown => "DOWN",
        KeyCode::Backspace => "BACKSPACE",
        KeyCode::Delete => "DELETE",
        KeyCode::Space => "SPACE",
        _ => return None,
    };
    Some(label.to_string())
}

/// Registry actions bound to one physical key label, resolved through the TAI
/// registry's `painter_bindings()`. Wheel-bound actions (`painter_zoom_in` on
/// `MouseWheelUp`) never resolve here, so the zoom keys (Minus/Equal) stay
/// live-only until they are registered as key bindings.
fn painter_key_actions_for_label(bindings: &ActionBindingMap, label: &str) -> Vec<ActionName> {
    bindings
        .actions_for(RawInput::Key(label.to_string()))
        .into_iter()
        .cloned()
        .collect()
}

/// Registry actions bound to one physical key press.
fn painter_key_actions(bindings: &ActionBindingMap, key: KeyCode) -> Vec<ActionName> {
    raw_key_label(key)
        .map(|label| painter_key_actions_for_label(bindings, &label))
        .unwrap_or_default()
}

/// Translates a physical key press into a text-entry key while typing. Single-
/// character labels (letters, digits, `-`, `=`) become chars; multi-character
/// labels (numpad keys) have no glyph, so they fall through as suppressed.
fn text_entry_key_for_key(key: KeyCode) -> Option<TextEntryKey> {
    match key {
        KeyCode::Space => Some(TextEntryKey::Space),
        KeyCode::Enter | KeyCode::NumpadEnter => Some(TextEntryKey::Enter),
        KeyCode::Escape => Some(TextEntryKey::Escape),
        KeyCode::Backspace => Some(TextEntryKey::Backspace),
        KeyCode::Delete => Some(TextEntryKey::Delete),
        KeyCode::ArrowLeft => Some(TextEntryKey::ArrowLeft),
        KeyCode::ArrowRight => Some(TextEntryKey::ArrowRight),
        KeyCode::ArrowUp => Some(TextEntryKey::ArrowUp),
        KeyCode::ArrowDown => Some(TextEntryKey::ArrowDown),
        _ => raw_key_label(key).and_then(|label| {
            let mut chars = label.chars();
            let ch = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            Some(TextEntryKey::Char(ch.to_ascii_lowercase()))
        }),
    }
}

/// Inputs that stay live while a typing session owns the input surface: the
/// current bindings of the camera swing/roll/focus-depth actions, so a
/// remap moves the reserved set with it (the old text-mode allowlist was
/// hardcoded to numpad 4/6/8/2/7/9 and 1/3).
fn typing_reserved_inputs(bindings: &ActionBindingMap) -> Vec<RawInput> {
    const TYPING_RESERVED_ACTIONS: &[&str] = &[
        "painter_swing_left",
        "painter_swing_right",
        "painter_swing_up",
        "painter_swing_down",
        "painter_roll_counter_clockwise",
        "painter_roll_clockwise",
        "painter_focus_depth_toward",
        "painter_focus_depth_away",
    ];
    TYPING_RESERVED_ACTIONS
        .iter()
        .flat_map(|action| {
            bindings
                .bindings_for(&ActionName::new(*action))
                .to_vec()
        })
        .collect()
}

/// Live painter behavior for one registry action. One entry per action the
/// dispatcher handles; `LIVE_PAINTER_ACTIONS` and the dispatch match below
/// must stay in sync, which the drift tests assert in both directions.
fn painter_tool_for_action(action: &ActionName) -> Option<PaintTool> {
    // Derived from the tool registry: a descriptor's select_action maps to
    // its registered tool, so new tools become hotkey-equippable by
    // declaring select_action + hotkey in their descriptor.
    thaum_painter_domain::painter_tools::all()
        .iter()
        .find(|descriptor| descriptor.select_action == Some(action.0.as_str()))
        .and_then(|descriptor| PaintTool::from_id(descriptor.id))
}

/// Every named action the entrypoint's live dispatch handles. The effective
/// binding map (declared defaults + per-user profile) must declare exactly
/// these actions; anything declared-but-unhandled or handled-but-undeclared
/// fails the hotkey drift tests.
pub const LIVE_PAINTER_ACTIONS: &[&str] = &[
    "painter_select_pencil",
    "painter_select_bucket",
    "painter_select_lasso",
    "painter_pan_left",
    "painter_pan_right",
    "painter_pan_up",
    "painter_pan_down",
    "painter_swing_left",
    "painter_swing_right",
    "painter_swing_up",
    "painter_swing_down",
    "painter_roll_counter_clockwise",
    "painter_roll_clockwise",
    "painter_focus_depth_toward",
    "painter_focus_depth_away",
    "painter_zoom_out",
    "painter_zoom_in",
    "painter_selection_mode_replace",
    "painter_selection_mode_additive",
    "painter_selection_mode_subtract",
    "painter_selection_mode_intersect",
    "painter_selection_clear",
    "painter_selection_invert",
    "painter_selection_all",
    "painter_undo",
    "painter_redo",
    "painter_play_pause",
];

/// Presentation rows for the controls panel: the declared action list with
/// human labels. Drift tests keep these names locked to the registry.
fn painter_control_rows() -> Vec<ControlActionRow> {
    let rows = [
        ("tools", "Select Pencil", "painter_select_pencil"),
        ("tools", "Select Bucket", "painter_select_bucket"),
        ("tools", "Select Lasso", "painter_select_lasso"),
        ("pan", "Pan Left", "painter_pan_left"),
        ("pan", "Pan Right", "painter_pan_right"),
        ("pan", "Pan Up", "painter_pan_up"),
        ("pan", "Pan Down", "painter_pan_down"),
        ("camera", "Swing Left", "painter_swing_left"),
        ("camera", "Swing Right", "painter_swing_right"),
        ("camera", "Swing Up", "painter_swing_up"),
        ("camera", "Swing Down", "painter_swing_down"),
        ("camera", "Roll Counter-Clockwise", "painter_roll_counter_clockwise"),
        ("camera", "Roll Clockwise", "painter_roll_clockwise"),
        ("camera", "Focus Depth Toward", "painter_focus_depth_toward"),
        ("camera", "Focus Depth Away", "painter_focus_depth_away"),
        ("camera", "Zoom Out", "painter_zoom_out"),
        ("camera", "Zoom In", "painter_zoom_in"),
        ("selection", "Mode: Replace", "painter_selection_mode_replace"),
        ("selection", "Mode: Additive", "painter_selection_mode_additive"),
        ("selection", "Mode: Subtract", "painter_selection_mode_subtract"),
        ("selection", "Mode: Intersect", "painter_selection_mode_intersect"),
        ("selection", "Clear Plane", "painter_selection_clear"),
        ("selection", "Invert Plane", "painter_selection_invert"),
        ("selection", "Select All Plane", "painter_selection_all"),
        ("history", "Undo", "painter_undo"),
        ("history", "Redo", "painter_redo"),
    ];
    rows.into_iter()
        .map(|(category, label, action)| ControlActionRow {
            action: ActionName::new(action),
            label: label.to_string(),
            category: category.to_string(),
        })
        .collect()
}

fn main() -> Result<()> {
    thaum_painter_domain::debug_log::configure_from_env();
    let mut config = BootConfig::default();
    config.asset_root = development_asset_root();
    config.window.title = "thaum-painter".to_string();

    let session_user_id = session_user_id();
    let session_state_path = painter_session_state_path(&session_user_id);
    let persisted_session = load_painter_user_session_state(&session_state_path)?;
    // Boot to a blank unsaved document: the user opens or saves explicitly. Restoring
    // the last document from session state once pinned boot to a legacy file that
    // would break silently on schema changes.
    let mut shared_document = new_unsaved_document();
    let mut shared_document_paths = painter_shared_document_paths(&shared_document.document.document_id);
    let mut current_document_root: Option<PathBuf> = None;

    let mut state = boot_renderer(config)?;
    if let Some(session) = &persisted_session {
        session.renderer.camera.apply_to_runtime(&mut state.camera);
    }

    let mut modules = ModuleRegistry::new();
    let ui_palette = UiPalette::default();
    // Per-user controls profile (overrides only), restored from the saved
    // session. Effective bindings are the declared defaults merged with it;
    // live dispatch and the controls panel both resolve through this map.
    let painter_bindings = thaum_painter_domain::tai::painter_bindings();
    let controls_profile = Rc::new(RefCell::new(
        persisted_session
            .as_ref()
            .map(|session| session.controls_profile.clone())
            .unwrap_or_default(),
    ));
    let effective_painter_bindings = Rc::new(RefCell::new(effective_bindings(
        &painter_bindings,
        &controls_profile.borrow(),
    )));
    let tool_state = Rc::new(RefCell::new(ToolState::default()));
    let timeline_state = Rc::new(RefCell::new(TimelineState::default()));
    let paint_canvas_viewport = Rc::new(RefCell::new(INITIAL_PAINT_CANVAS_VIEWPORT));
    let drawing_space_wheel_mode = Rc::new(RefCell::new(DrawingSpaceWheelMode::Pan));
    let paint_canvas_bounds = Rc::new(RefCell::new(INITIAL_PAINT_CANVAS_BOUNDS));
    let selection = Rc::new(RefCell::new(PainterSelection::new(
        *paint_canvas_bounds.borrow(),
    )));
    // Restore the document-owned 3D selection into the UI cache at boot. The set is
    // not plane-pruned, so cells at other depths/planes survive camera moves.
    selection.borrow_mut().restore_points(
        shared_document.selection_points(DEFAULT_SELECTION_CHANNEL_ID),
    );

    modules.register(Box::new(
        ToolboxModule::new(
            "painter_toolbox",
            ModuleRect {
                x0: -6,
                y0: -1,
                x1: 12,
                y1: 5,
            },
            tool_state.clone(),
            thaum_painter_domain::painter_tools::all()
                .iter()
                .filter_map(|descriptor| {
                    Some(ToolDef {
                        tool: PaintTool::from_id(descriptor.id)?,
                        icon: descriptor.icon,
                        label: descriptor.label,
                    })
                })
                .collect(),
        )
        .with_palette(ui_palette.clone()),
    ));
    modules.register(Box::new(PaintColorPickerModule::new(
        "painter_color_picker",
        ModuleRect {
            x0: 13,
            y0: -8,
            x1: 29,
            y1: 0,
        },
        tool_state.clone(),
        ui_palette.clone(),
    )));
    modules.register(Box::new(PaintColorBlockModule::new(
        "painter_color_block",
        ModuleRect {
            x0: 13,
            y0: -21,
            x1: 31,
            y1: -10,
        },
        tool_state.clone(),
        ui_palette.clone(),
    )));
    modules.register(Box::new(
        MaterialPickerModule::new(
            "painter_material_picker",
            ModuleRect {
                x0: 32,
                y0: -21,
                x1: 48,
                y1: -10,
            },
            tool_state.clone(),
        )
        .with_palette(ui_palette.clone()),
    ));
    modules.register(Box::new(
        GraphicPickerModule::new(
            "painter_graphic_picker",
            ModuleRect {
                x0: 31,
                y0: -8,
                x1: 70,
                y1: 28,
            },
            tool_state.clone(),
        )
        .with_palette(ui_palette.clone()),
    ));
    // Shared in-place number-field edit state for the hand-settings panel:
    // the module opens it on a field click, the typing seam routes the keys,
    // and Enter commits the value through the tool state.
    let number_edit: Rc<RefCell<Option<NumberFieldEdit>>> = Rc::new(RefCell::new(None));
    modules.register(Box::new(
        HandSettingsModule::new(
            "painter_hand_settings",
            ModuleRect {
                x0: 13,
                y0: 2,
                x1: 43,
                y1: 15,
            },
            tool_state.clone(),
            selection.clone(),
            number_edit.clone(),
        )
        .with_palette(ui_palette.clone()),
    ));
    modules.register(Box::new(PaintCanvasBoundsModule::new(
        "paint_canvas_bounds",
        paint_canvas_viewport.clone(),
        drawing_space_wheel_mode.clone(),
        ui_palette.clone(),
    )));
    let layers_panel_state = Rc::new(RefCell::new(LayersPanelState::default()));
    modules.register(Box::new(
        LayersPanelModule::new(
            "painter_layers_panel",
            ModuleRect {
                x0: 25,
                y0: -24,
                x1: 70,
                y1: -3,
            },
            layers_panel_state.clone(),
        )
        .with_palette(ui_palette.clone()),
    ));
    modules.register(Box::new(UiCustomizationModule::new(
        "painter_ui_customization",
        ModuleRect {
            x0: 44,
            y0: 2,
            x1: 66,
            y1: 12,
        },
        ui_palette.clone(),
        {
            let tool_state = tool_state.clone();
            move || {
                let rgb = tool_state.borrow().left_hand.color.preview_rgb();
                [rgb.0, rgb.1, rgb.2]
            }
        },
        {
            let tool_state = tool_state.clone();
            move || {
                let rgb = tool_state.borrow().right_hand.color.preview_rgb();
                [rgb.0, rgb.1, rgb.2]
            }
        },
    )));
    modules.register(Box::new(ControlsPanelModule::new(
        "painter_controls_panel",
        ModuleRect {
            x0: 4,
            y0: 13,
            x1: 44,
            y1: 47,
        },
        ui_palette.clone(),
        painter_control_rows(),
        {
            let effective = effective_painter_bindings.clone();
            move |action| {
                effective
                    .borrow()
                    .bindings_for(action)
                    .first()
                    .map(format_raw_input)
                    .unwrap_or_else(|| "unbound".to_string())
            }
        },
        {
            let effective = effective_painter_bindings.clone();
            move |action| {
                conflicting_actions(&effective.borrow(), action)
                    .iter()
                    .filter_map(|other| {
                        effective
                            .borrow()
                            .bindings_for(other)
                            .first()
                            .map(format_raw_input)
                    })
                    .collect()
            }
        },
        {
            let profile = controls_profile.clone();
            let effective = effective_painter_bindings.clone();
            let defaults = painter_bindings.clone();
            move |action, binding| {
                profile.borrow_mut().set_override(action, binding);
                *effective.borrow_mut() =
                    effective_bindings(&defaults, &profile.borrow());
            }
        },
    )));
    if let Some(session) = &persisted_session {
        session.renderer.palette.apply_to_runtime(&ui_palette);
        modules.apply_persisted_ui_state(&session.renderer.modules);
    }
    sync_renderer_background_from_ui_palette(&mut state, &ui_palette);

    let mut left_pointer_was_down = false;
    let mut right_pointer_was_down = false;
    let mut left_drag_position: Option<CellPoint> = None;
    let mut right_drag_position: Option<CellPoint> = None;
    // Canvas snapshot + target block captured at stroke press; the release commits
    // the whole drag as one record (one undo per stroke).
    let mut left_stroke_start: Option<(Canvas, String)> = None;
    let mut right_stroke_start: Option<(Canvas, String)> = None;
    // Live text-entry session: begun by a Text-tool canvas click, ended by
    // Escape/exit. Pending keystrokes commit as one 'Type Text' record per
    // segment (Enter starts a new segment), the old commit granularity.
    let mut text_entry: Option<TextEntryState> = None;
    let mut text_stroke_start: Option<(Canvas, String)> = None;
    // Renderer-owned input-focus gate for typing sessions; declared reserved
    // inputs (camera/depth bindings) stay live, everything else is focused
    // into the session or suppressed.
    let mut typing_mode = TypingMode::default();
    let mut selection_stroke: Option<SelectionStroke> = None;
    // In-progress lasso bound (either hand): press starts the path, drag
    // extends it, release rasterizes and fills/selects the enclosed region.
    let mut lasso_stroke: Option<LassoStroke> = None;
    let mut active_layer_id = resolved_active_layer_id(
        &shared_document,
        persisted_session
            .as_ref()
            .and_then(|session| session.painter.active_layer_id.as_deref()),
    );
    // Seek the playhead to a breath the active layer's raster track actually covers:
    // prefer the restored playhead, but if it (or the boot default of 0) sits in a
    // raster gap, the layer would render nothing and every stroke would be silently
    // rejected — exactly the "booted and could not draw" failure.
    let restored_breath = persisted_session
        .as_ref()
        .map(|session| session.painter.current_breath)
        .filter(|breath| {
            shared_document
                .active_raster_block_id(&active_layer_id, *breath)
                .is_some()
        });
    let initial_breath = restored_breath.unwrap_or_else(|| {
        shared_document
            .first_breath_with_raster_block(&active_layer_id)
            .unwrap_or(0)
    });
    timeline_state.borrow_mut().set_current_breath(initial_breath);
    let mut selected_property_id: Option<String> = None;
    let mut canvas = shared_document
        .canvas_for_layer(&active_layer_id, timeline_state.borrow().current_breath)
        .cloned()
        .unwrap_or_default();
    let mut command_bar = CommandBar::new("painter_command_bar", ui_palette.clone())
        .with_buttons(vec![
            CommandBarButton::new("menu:file", "FILE"),
            CommandBarButton::new("menu:modules", "MODULES"),
        ])
        .with_nested_buttons("menu:file", file_menu_buttons())
        .with_nested_buttons("menu:modules", module_menu_buttons(&modules));
    if let Some(session) = &persisted_session {
        let mut wheel_mode = drawing_space_wheel_mode.borrow_mut();
        let mut selection_ref = selection.borrow_mut();
        let mut tool_state_ref = tool_state.borrow_mut();
        let mut selection_mode = selection_ref.mode();
        session.painter.apply_to_runtime(
            &mut wheel_mode,
            &mut selection_mode,
            &mut command_bar,
            &mut tool_state_ref,
        );
        selection_ref.set_mode(selection_mode);
    }
    let mut shared_action_counter = shared_document.actions.len() as u64;
    let mut last_saved_session_text: Option<String> = None;
    let mut last_save_at = Instant::now() - Duration::from_secs(1);
    // Playback advances one breath per tick while the timeline is playing.
    const PLAYBACK_BREATH_INTERVAL: Duration = Duration::from_millis(125);
    let mut last_playback_step = Instant::now();
    // Text cursor blink state (rendered overlay only; see cursor_overlay_group).
    const CURSOR_BLINK_INTERVAL: Duration = Duration::from_millis(400);
    let mut cursor_blink_on = true;
    let mut cursor_blink_at = Instant::now();

    run_renderer_window_with_state_frame_provider(state, move |state, frame| {
        sync_renderer_background_from_ui_palette(state, &ui_palette);

        // Which pan WASD should drive this frame: hovering the drawing
        // surface routes to 3D pan (the file's contents, under a
        // screen-fixed frame); anywhere else routes to 2D pan (the HUD
        // layer itself, so off-screen panels can be reached).
        let hovering_canvas_bounds = frame
            .input
            .cursor_position
            .and_then(|cursor| {
                let cell_clip_size = cell_clip_size_for_state(state, frame.surface_size);
                let screen =
                    remap_surface_units_to_flat_2d_local(state.camera, cursor, cell_clip_size);
                modules.hit_test(screen.x, screen.y)
            })
            .is_some_and(|module| module.id() == "paint_canvas_bounds");

        for key in &frame.input.pressed_keys {
            // While a typing session owns the input surface, only the
            // reserved camera/depth bindings stay live for held keys; every
            // other held binding (pan WASD included) is suppressed.
            if typing_mode.is_active() {
                let reserved = raw_key_label(*key)
                    .map(|label| typing_mode.route(&RawInput::Key(label)))
                    .is_some_and(|route| route == TypingRoute::Reserved);
                if !reserved {
                    continue;
                }
            }
            // Held pan resolves through the effective binding map too, so a
            // remap moves the continuous pan behavior along with the press.
            let held_actions = painter_key_actions(&effective_painter_bindings.borrow(), *key);
            let binds = |name: &str| {
                held_actions
                    .iter()
                    .any(|action| action.0.as_str() == name)
            };
            // Inverted from the camera's own right/up so the content
            // visually moves the way the key points, not the way the
            // camera's aim point moves.
            if binds("painter_pan_left") {
                if hovering_canvas_bounds {
                    state.camera.pan_focus_right(1);
                } else {
                    pan_hud_and_focus_right(&mut state.camera, 1);
                }
            }
            if binds("painter_pan_right") {
                if hovering_canvas_bounds {
                    state.camera.pan_focus_right(-1);
                } else {
                    pan_hud_and_focus_right(&mut state.camera, -1);
                }
            }
            if binds("painter_pan_up") {
                if hovering_canvas_bounds {
                    state.camera.pan_focus_up(-1);
                } else {
                    pan_hud_and_focus_up(&mut state.camera, -1);
                }
            }
            if binds("painter_pan_down") {
                if hovering_canvas_bounds {
                    state.camera.pan_focus_up(1);
                } else {
                    pan_hud_and_focus_up(&mut state.camera, 1);
                }
            }
        }
        for key in &frame.input.just_pressed_keys {
            // While typing mode owns the input surface: session keys route
            // into the typing session, reserved camera/depth inputs fall
            // through to live dispatch, and everything else — including
            // module key capture — is suppressed.
            if typing_mode.is_active() {
                let route = raw_key_label(*key)
                    .map(|label| typing_mode.route(&RawInput::Key(label)))
                    .unwrap_or(TypingRoute::Suppressed);
                match route {
                    TypingRoute::Suppressed => continue,
                    TypingRoute::Owned => {
                        let Some(entry_key) = text_entry_key_for_key(*key) else {
                            continue;
                        };
                        // An open number-field edit consumes the keys first:
                        // digits and '-' build the buffer, Backspace erases,
                        // Enter commits into the tool state, Escape cancels.
                        if number_edit.borrow().is_some() {
                            match entry_key {
                                TextEntryKey::Enter => {
                                    if let Some(edit) = number_edit.borrow_mut().take() {
                                        if let Some(value) = edit.commit(-9, 9) {
                                            let mut tool_state = tool_state.borrow_mut();
                                            if edit.row_id == "text_enter_step" {
                                                tool_state.set_text_enter_step_axis(
                                                    edit.field.min(2),
                                                    value,
                                                );
                                            } else {
                                                tool_state.set_text_char_step_axis(
                                                    edit.field.min(2),
                                                    value,
                                                );
                                            }
                                        }
                                    }
                                }
                                TextEntryKey::Escape => {
                                    number_edit.borrow_mut().take();
                                }
                                TextEntryKey::Backspace | TextEntryKey::Delete => {
                                    if let Some(edit) = number_edit.borrow_mut().as_mut() {
                                        edit.backspace();
                                    }
                                }
                                TextEntryKey::Char(ch) => {
                                    if let Some(edit) = number_edit.borrow_mut().as_mut() {
                                        edit.push(ch);
                                    }
                                }
                                _ => {}
                            }
                            continue;
                        }
                        match text_entry
                            .as_mut()
                            .expect("typing mode active without a typing session")
                            .handle_key(entry_key)
                        {
                    TextEntryOutcome::Applied { point, cell } => {
                        if let Some((_, block_id)) = text_stroke_start.as_ref() {
                            stage_text_entry_change(
                                &mut shared_document,
                                &mut canvas,
                                (point, cell),
                                &active_layer_id,
                                block_id,
                            );
                        }
                    }
                    TextEntryOutcome::Committed => {
                        // Enter: the pending segment becomes one 'Type Text'
                        // record; typing continues on the next line as a new
                        // undo segment.
                        if let Err(err) = commit_staged_paint_stroke(
                            &mut shared_document,
                            &shared_document_paths,
                            &mut shared_action_counter,
                            &session_user_id,
                            &active_layer_id,
                            &mut canvas,
                            text_stroke_start.take(),
                        ) {
                            eprintln!("text commit failed (kept in memory): {err:#}");
                        }
                        let current_breath = timeline_state.borrow().current_breath;
                        text_stroke_start = shared_document
                            .active_raster_block_id(&active_layer_id, current_breath)
                            .map(|block_id| (canvas.clone(), block_id.clone()));
                    }
                    TextEntryOutcome::Finished => {
                        if let Err(err) = commit_staged_paint_stroke(
                            &mut shared_document,
                            &shared_document_paths,
                            &mut shared_action_counter,
                            &session_user_id,
                            &active_layer_id,
                            &mut canvas,
                            text_stroke_start.take(),
                        ) {
                            eprintln!("text commit failed (kept in memory): {err:#}");
                        }
                        text_entry = None;
                        typing_mode.end();
                    }
                    TextEntryOutcome::Idle | TextEntryOutcome::Ignored => {}
                        }
                        // The owned key belongs to the session alone: it must
                        // not also reach module key capture or live dispatch,
                        // or typed chars that share a binding (W/A/S/D pan,
                        // P/B tool select) fire while typing.
                        continue;
                    }
                    // Reserved camera/depth inputs fall through to module key
                    // capture and live binding dispatch below.
                    TypingRoute::Reserved => {}
                }
            }
            // While a controls-panel row waits for a captured key, the press
            // becomes that row's new binding and never dispatches a command.
            if let Some(label) = raw_key_label(*key) {
                if modules.dispatch_key_capture(&label).is_some() {
                    continue;
                }
            }
            // Registry-owned keys dispatch by binding name through the
            // effective map (declared defaults + user profile), one live
            // behavior per action, so a remap moves the behavior with it.
            // Unbound keys do nothing: there are no hidden live-only arms.
            let binding_actions =
                painter_key_actions(&effective_painter_bindings.borrow(), *key);
            for action in &binding_actions {
                if let Some(tool) = painter_tool_for_action(action) {
                    let hand = tool_state.borrow().active_hand;
                    tool_state.borrow_mut().set_tool_for_hand(hand, tool);
                    continue;
                }
                match action.0.as_str() {
                    "painter_play_pause" => {
                        // Space toggles playback over the document's loop
                        // window; starting outside the window snaps to its
                        // start (same as the panel's PLAY button).
                        let window = shared_document.document_window();
                        let was_playing = timeline_state.borrow().playing;
                        timeline_state.borrow_mut().toggle_play();
                        if !was_playing
                            && (timeline_state.borrow().current_breath < window.start_breath
                                || timeline_state.borrow().current_breath > window.end_breath)
                        {
                            timeline_state
                                .borrow_mut()
                                .set_current_breath(window.start_breath);
                            sync_canvas_from_active_layer(
                                &shared_document,
                                &active_layer_id,
                                window.start_breath,
                                &mut canvas,
                            );
                        }
                    }
                    "painter_pan_left" if hovering_canvas_bounds => {
                        state.camera.pan_focus_right(1)
                    }
                    "painter_pan_left" => pan_hud_and_focus_right(&mut state.camera, 1),
                    "painter_pan_right" if hovering_canvas_bounds => {
                        state.camera.pan_focus_right(-1)
                    }
                    "painter_pan_right" => pan_hud_and_focus_right(&mut state.camera, -1),
                    "painter_pan_up" if hovering_canvas_bounds => {
                        state.camera.pan_focus_up(-1)
                    }
                    "painter_pan_up" => pan_hud_and_focus_up(&mut state.camera, -1),
                    "painter_pan_down" if hovering_canvas_bounds => {
                        state.camera.pan_focus_up(1)
                    }
                    "painter_pan_down" => pan_hud_and_focus_up(&mut state.camera, 1),
                    "painter_swing_left" => reorient_camera_around_viewport_center(
                        &mut state.camera,
                        *paint_canvas_viewport.borrow(),
                        |camera| camera.swing_left(),
                    ),
                    "painter_swing_right" => reorient_camera_around_viewport_center(
                        &mut state.camera,
                        *paint_canvas_viewport.borrow(),
                        |camera| camera.swing_right(),
                    ),
                    "painter_swing_up" => reorient_camera_around_viewport_center(
                        &mut state.camera,
                        *paint_canvas_viewport.borrow(),
                        |camera| camera.swing_up(),
                    ),
                    "painter_swing_down" => reorient_camera_around_viewport_center(
                        &mut state.camera,
                        *paint_canvas_viewport.borrow(),
                        |camera| camera.swing_down(),
                    ),
                    "painter_roll_counter_clockwise" => reorient_camera_around_viewport_center(
                        &mut state.camera,
                        *paint_canvas_viewport.borrow(),
                        |camera| camera.roll = camera.roll.rotate_counter_clockwise(),
                    ),
                    "painter_roll_clockwise" => reorient_camera_around_viewport_center(
                        &mut state.camera,
                        *paint_canvas_viewport.borrow(),
                        |camera| camera.roll = camera.roll.rotate_clockwise(),
                    ),
                    "painter_focus_depth_toward" => state.camera.pan_focus_depth(-1),
                    "painter_focus_depth_away" => state.camera.pan_focus_depth(1),
                    "painter_zoom_out" => state.camera.zoom_out(),
                    "painter_zoom_in" => state.camera.zoom_in(),
                    "painter_selection_mode_replace" => {
                        selection.borrow_mut().set_mode(SelectionMode::Replace)
                    }
                    "painter_selection_mode_additive" => {
                        selection.borrow_mut().set_mode(SelectionMode::Additive)
                    }
                    "painter_selection_mode_subtract" => {
                        selection.borrow_mut().set_mode(SelectionMode::Subtract)
                    }
                    "painter_selection_mode_intersect" => {
                        selection.borrow_mut().set_mode(SelectionMode::Intersect)
                    }
                    "painter_selection_clear" => {
                        selection.borrow_mut().clear_plane();
                        commit_selection_channel(
                            &mut shared_document,
                            &shared_document_paths,
                            std::iter::empty(),
                            SelectionMode::Replace,
                            &mut active_layer_id,
                            timeline_state.borrow().current_breath,
                            &mut canvas,
                            &selection,
                            &mut shared_action_counter,
                        );
                    }
                    "painter_selection_invert" => {
                        selection.borrow_mut().invert_plane();
                        let points: Vec<CellPoint> =
                            selection.borrow().plane().iter().collect();
                        commit_selection_channel(
                            &mut shared_document,
                            &shared_document_paths,
                            points,
                            SelectionMode::Replace,
                            &mut active_layer_id,
                            timeline_state.borrow().current_breath,
                            &mut canvas,
                            &selection,
                            &mut shared_action_counter,
                        );
                    }
                    "painter_selection_all" => {
                        selection.borrow_mut().select_all_plane();
                        let points: Vec<CellPoint> =
                            selection.borrow().plane().iter().collect();
                        commit_selection_channel(
                            &mut shared_document,
                            &shared_document_paths,
                            points,
                            SelectionMode::Replace,
                            &mut active_layer_id,
                            timeline_state.borrow().current_breath,
                            &mut canvas,
                            &selection,
                            &mut shared_action_counter,
                        );
                    }
                    "painter_undo" => apply_shared_history_action(
                        &mut shared_document,
                        &shared_document_paths,
                        &mut shared_action_counter,
                        &session_user_id,
                        &active_layer_id,
                        &mut canvas,
                        true,
                        timeline_state.borrow().current_breath,
                    )?,
                    "painter_redo" => apply_shared_history_action(
                        &mut shared_document,
                        &shared_document_paths,
                        &mut shared_action_counter,
                        &session_user_id,
                        &active_layer_id,
                        &mut canvas,
                        false,
                        timeline_state.borrow().current_breath,
                    )?,
                    _ => {}
                }
            }
        }

        sync_canvas_bounds_to_camera(
            &paint_canvas_viewport,
            &paint_canvas_bounds,
            &selection,
            &state.camera,
        );

        let camera = state.camera;
        let view_orientation = camera_view_orientation_for_camera(camera.swing, camera.roll);
        let cell_clip_size = cell_clip_size_for_state(state, frame.surface_size);
        let to_world = move |surface_units: [f32; 2]| {
            remap_surface_units_to_active_plane_world(camera, surface_units, cell_clip_size)
        };
        // Screen-space (2D HUD layer) counterpart of `to_world`: modules and
        // their gizmos live on the roll/swing/pan-immune Flat2d layer, so
        // hit-testing them must go through this, not `to_world`.
        let to_screen = move |surface_units: [f32; 2]| {
            remap_surface_units_to_flat_2d_local(camera, surface_units, cell_clip_size)
        };
        // Typing is camera-relative: the session re-bases to the live view
        // every frame, so arrows and typed cells keep following the camera
        // even after a mid-session swing.
        if let Some(entry) = text_entry.as_mut() {
            entry.set_orientation(view_orientation);
        }
        command_bar.set_nested_buttons("menu:modules", module_menu_buttons(&modules));
        // Playback tick: while playing, step the playhead across the document
        // loop window (wrapping when loop is on, stopping at the end when not)
        // and resync the edit surface like a scrub does.
        if last_playback_step.elapsed() >= PLAYBACK_BREATH_INTERVAL {
            last_playback_step = Instant::now();
            let window = shared_document.document_window();
            if let Some(breath) = timeline_state
                .borrow_mut()
                .advance_playback(window.start_breath, window.end_breath)
            {
                sync_canvas_from_active_layer(
                    &shared_document,
                    &active_layer_id,
                    breath,
                    &mut canvas,
                );
            }
        }
        layers_panel_state.borrow_mut().sync(
            shared_document
                .layers()
                .iter()
                .map(|layer| LayerRow {
                    id: layer.layer_id.clone(),
                    name: layer.name.clone(),
                    visible: layer.visible,
                    locked: layer.locked,
                    start_breath: layer.start_breath,
                    length_breaths: layer.length_breaths,
                })
                .collect(),
            build_selected_layer_property_rows(&shared_document, &active_layer_id),
            Some(active_layer_id.clone()),
            selected_property_id.clone(),
            timeline_state.borrow().current_breath,
            timeline_state.borrow().auto_key_enabled,
            shared_document.document_window().start_breath,
            shared_document.document_window().end_breath,
            timeline_state.borrow().playing,
            timeline_state.borrow().loop_enabled,
        );
        command_bar.update_layout(cell_clip_size, state.camera.hud_pan_offset);
        let command_bar_hover = frame.input.cursor_position.map(to_screen);
        command_bar.set_pointer_position(command_bar_hover.map(|screen| (screen.x, screen.y)));
        let paint_viewport = *paint_canvas_viewport.borrow();
        let paint_surface = PaintCanvasBoundsModule::content_rect(paint_viewport);

        // While typing mode owns the input surface, all pointer input is
        // suppressed: drags and panel interactions cannot reach modules or
        // the canvas behind the session's back. A click is the user saying
        // they want out of typing, so it ends the session (committing the
        // pending stroke, like Escape) and is otherwise swallowed.
        if typing_mode.is_active() {
            if frame.input.just_clicked.is_some() {
                if number_edit.borrow().is_some() {
                    // A number edit stages nothing; a click just closes it.
                    number_edit.borrow_mut().take();
                } else if let Err(err) = commit_staged_paint_stroke(
                    &mut shared_document,
                    &shared_document_paths,
                    &mut shared_action_counter,
                    &session_user_id,
                    &active_layer_id,
                    &mut canvas,
                    text_stroke_start.take(),
                ) {
                    // A failed commit must not tear down the session: the
                    // in-memory document already holds the change.
                    eprintln!("text commit failed (kept in memory): {err:#}");
                }
                text_entry = None;
                typing_mode.end();
            }
        } else if let Some(click) = frame.input.just_clicked {
            let world = to_world(click);
            let screen = to_screen(click);
            let handled_command_bar = command_bar.contains(screen.x, screen.y);
            if let Some(CommandBarClickOutcome::ButtonPressed { button_id }) = command_bar
                .on_pointer_event(ModulePointerEvent::Click {
                    x: screen.x,
                    y: screen.y,
                    button: ModulePointerButton::Left,
                })
            {
                handle_command_bar_button(
                    &button_id,
                    &mut modules,
                    &mut shared_document,
                    &mut shared_document_paths,
                    &mut current_document_root,
                    &mut active_layer_id,
                    &mut selected_property_id,
                    &mut shared_action_counter,
                    timeline_state.borrow().current_breath,
                    &mut canvas,
                    &selection,
                )?;
            }
            let module_hit = if handled_command_bar {
                None
            } else {
                modules
                    .dispatch_pointer_event_at(
                        screen.x,
                        screen.y,
                        ModulePointerEvent::Click {
                            x: screen.x,
                            y: screen.y,
                            button: ModulePointerButton::Left,
                        },
                    )
                    .map(str::to_string)
            };
            // The drawing-space module sits under the whole paint surface, so its hit
            // must not swallow canvas clicks: only clicks on its gizmo bar or wheel
            // chip count as handled; everything else falls through to painting.
            let handled_module = match module_hit.as_deref() {
                Some("paint_canvas_bounds") => {
                    PaintCanvasBoundsModule::is_gizmo_hit(paint_viewport, screen.x, screen.y)
                }
                Some(_) => true,
                None => false,
            };
            apply_layers_panel_action(
                layers_panel_state.borrow_mut().take_pending_action(),
                &mut shared_document,
                &shared_document_paths,
                &mut shared_action_counter,
                &session_user_id,
                &mut active_layer_id,
                &mut selected_property_id,
                &mut canvas,
                &timeline_state,
                &selection,
            );
            if handled_command_bar || handled_module || modules.is_pointer_captured() {
                selection_stroke = None;
                lasso_stroke = None;
                left_drag_position = None;
            } else {
                let bounds = *paint_canvas_bounds.borrow();
                let position = CellPoint {
                    x: world.x,
                    y: world.y,
                    z: world.z,
                };
                if paint_surface.contains(screen.x, screen.y)
                    && bounds.contains(position)
                    && !PaintCanvasBoundsModule::is_gizmo_hit(paint_viewport, screen.x, screen.y)
                    && text_entry.as_ref().map_or(true, |entry| !entry.is_active())
                {
                    left_drag_position = Some(position);
                    if tool_state.borrow().hand_state(PaintHand::Left).target
                        == PaintTarget::Selection
                    {
                        // Lasso on the selection surface records its bound;
                        // the enclosed region is selected on release.
                        if tool_state.borrow().tool_for_hand(PaintHand::Left)
                            == PaintTool::Lasso
                        {
                            lasso_stroke = Some(LassoStroke::new(PaintHand::Left, position));
                        } else {
                        let mode = thaum_painter_domain::painter_tools::shared::selection_behavior(
                            tool_state.borrow().tool_for_hand(PaintHand::Left).id(),
                        )
                        .resolve(selection.borrow().mode(), SelectionMode::Subtract);
                        let mut stroke = SelectionStroke::new(PaintHand::Left, mode);
                        stroke.extend(tool_state.borrow().selection_points_for_hand(
                            &canvas,
                            position,
                            PaintHand::Left,
                            bounds,
                            view_orientation,
                        ));
                        selection_stroke = Some(stroke);
                        }
                    } else {
                        let target = tool_state.borrow().hand_state(PaintHand::Left).target;
                        let mut selection_state = selection.borrow_mut();
                        if target == PaintTarget::Image {
                            // Text tool: the click begins a typing session anchored at
                            // the click cell with the hand's brush captured; typing then
                            // owns the keyboard until Escape/exit.
                            if tool_state.borrow().tool_for_hand(PaintHand::Left) == PaintTool::Text {
                                let current_breath = timeline_state.borrow().current_breath;
                                if let Some(block_id) = shared_document
                                    .active_raster_block_id(&active_layer_id, current_breath)
                                {
                                    let brush_cell = tool_state
                                        .borrow()
                                        .text_brush_cell_for_hand(PaintHand::Left);
                                    let (options, space_replace) = {
                                        let ts = tool_state.borrow();
                                        (ts.text_options, ts.text_space_replace)
                                    };
                                    text_entry = Some(TextEntryState::begin(
                                        position,
                                        view_orientation,
                                        options,
                                        space_replace,
                                        brush_cell,
                                    ));
                                    text_stroke_start = Some((canvas.clone(), block_id.clone()));
                                    typing_mode.begin(typing_reserved_inputs(
                                        &effective_painter_bindings.borrow(),
                                    ));
                                }
                            } else if tool_state.borrow().tool_for_hand(PaintHand::Left)
                                == PaintTool::Lasso
                            {
                                // Lasso records its bound only; the fill lands
                                // on release as one committed stroke.
                                let current_breath = timeline_state.borrow().current_breath;
                                if let Some(block_id) = shared_document
                                    .active_raster_block_id(&active_layer_id, current_breath)
                                {
                                    left_stroke_start = Some((canvas.clone(), block_id.clone()));
                                    lasso_stroke =
                                        Some(LassoStroke::new(PaintHand::Left, position));
                                }
                            } else {
                            // Paint strokes land on the raster block covering the playhead
                            // breath; a breath in a gap has no canvas, so the stroke is rejected.
                            let current_breath = timeline_state.borrow().current_breath;
                            if let Some(block_id) = shared_document
                                .active_raster_block_id(&active_layer_id, current_breath)
                            {
                                left_stroke_start = Some((canvas.clone(), block_id.clone()));
                                stage_image_edit_chunk(
                                    &mut shared_document,
                                    &mut canvas,
                                    &mut tool_state.borrow_mut(),
                                    &mut selection_state,
                                    [position],
                                    PaintHand::Left,
                                    bounds,
                                    view_orientation,
                                    &active_layer_id,
                                    &block_id,
                                );
                            }
                            }
                        } else {
                            tool_state.borrow_mut().apply_at_for_hand(
                                &mut canvas,
                                &mut selection_state,
                                position,
                                PaintHand::Left,
                                bounds,
                                view_orientation,
                            );
                        }
                    }
                } else {
                }
            }
        } else if let Some(click) = frame.input.just_right_clicked {
            let world = to_world(click);
            let screen = to_screen(click);
            let handled_command_bar = command_bar.contains(screen.x, screen.y);
            // Same drawing-space passthrough as the left-click path: only gizmo
            // bar and wheel chip hits count as handled; the rest fall through.
            let handled_module = match (!handled_command_bar).then(|| {
                modules
                    .dispatch_pointer_event_at(
                        screen.x,
                        screen.y,
                        ModulePointerEvent::Click {
                            x: screen.x,
                            y: screen.y,
                            button: ModulePointerButton::Right,
                        },
                    )
                    .map(str::to_string)
            }) {
                Some(Some(id)) if id == "paint_canvas_bounds" => {
                    PaintCanvasBoundsModule::is_gizmo_hit(paint_viewport, screen.x, screen.y)
                }
                Some(Some(_)) => true,
                _ => false,
            };
            apply_layers_panel_action(
                layers_panel_state.borrow_mut().take_pending_action(),
                &mut shared_document,
                &shared_document_paths,
                &mut shared_action_counter,
                &session_user_id,
                &mut active_layer_id,
                &mut selected_property_id,
                &mut canvas,
                &timeline_state,
                &selection,
            );
            if handled_command_bar || handled_module || modules.is_pointer_captured() {
                selection_stroke = None;
                lasso_stroke = None;
                right_drag_position = None;
            } else {
                let bounds = *paint_canvas_bounds.borrow();
                let position = CellPoint {
                    x: world.x,
                    y: world.y,
                    z: world.z,
                };
                if paint_surface.contains(screen.x, screen.y)
                    && bounds.contains(position)
                    && !PaintCanvasBoundsModule::is_gizmo_hit(paint_viewport, screen.x, screen.y)
                    && text_entry.as_ref().map_or(true, |entry| !entry.is_active())
                {
                    right_drag_position = Some(position);
                    if tool_state.borrow().hand_state(PaintHand::Right).target
                        == PaintTarget::Selection
                    {
                        // Lasso on the selection surface records its bound;
                        // the enclosed region is selected on release.
                        if tool_state.borrow().tool_for_hand(PaintHand::Right)
                            == PaintTool::Lasso
                        {
                            lasso_stroke = Some(LassoStroke::new(PaintHand::Right, position));
                        } else {
                        let mode = thaum_painter_domain::painter_tools::shared::selection_behavior(
                            tool_state.borrow().tool_for_hand(PaintHand::Right).id(),
                        )
                        .resolve(selection.borrow().mode(), SelectionMode::Subtract);
                        let mut stroke = SelectionStroke::new(PaintHand::Right, mode);
                        stroke.extend(tool_state.borrow().selection_points_for_hand(
                            &canvas,
                            position,
                            PaintHand::Right,
                            bounds,
                            view_orientation,
                        ));
                        selection_stroke = Some(stroke);
                        }
                    } else {
                        let target = tool_state.borrow().hand_state(PaintHand::Right).target;
                        let mut selection_state = selection.borrow_mut();
                        if target == PaintTarget::Image {
                            // Text tool: same typing-session begin as the left hand.
                            if tool_state.borrow().tool_for_hand(PaintHand::Right) == PaintTool::Text {
                                let current_breath = timeline_state.borrow().current_breath;
                                if let Some(block_id) = shared_document
                                    .active_raster_block_id(&active_layer_id, current_breath)
                                {
                                    let brush_cell = tool_state
                                        .borrow()
                                        .text_brush_cell_for_hand(PaintHand::Right);
                                    let (options, space_replace) = {
                                        let ts = tool_state.borrow();
                                        (ts.text_options, ts.text_space_replace)
                                    };
                                    text_entry = Some(TextEntryState::begin(
                                        position,
                                        view_orientation,
                                        options,
                                        space_replace,
                                        brush_cell,
                                    ));
                                    text_stroke_start = Some((canvas.clone(), block_id.clone()));
                                    typing_mode.begin(typing_reserved_inputs(
                                        &effective_painter_bindings.borrow(),
                                    ));
                                }
                            } else if tool_state.borrow().tool_for_hand(PaintHand::Right)
                                == PaintTool::Lasso
                            {
                                // Lasso records its bound only; the fill lands
                                // on release as one committed stroke.
                                let current_breath = timeline_state.borrow().current_breath;
                                if let Some(block_id) = shared_document
                                    .active_raster_block_id(&active_layer_id, current_breath)
                                {
                                    right_stroke_start = Some((canvas.clone(), block_id.clone()));
                                    lasso_stroke =
                                        Some(LassoStroke::new(PaintHand::Right, position));
                                }
                            } else {
                            let current_breath = timeline_state.borrow().current_breath;
                            if let Some(block_id) = shared_document
                                .active_raster_block_id(&active_layer_id, current_breath)
                            {
                                right_stroke_start = Some((canvas.clone(), block_id.clone()));
                                stage_image_edit_chunk(
                                    &mut shared_document,
                                    &mut canvas,
                                    &mut tool_state.borrow_mut(),
                                    &mut selection_state,
                                    [position],
                                    PaintHand::Right,
                                    bounds,
                                    view_orientation,
                                    &active_layer_id,
                                    &block_id,
                                );
                            }
                            }
                        } else {
                            tool_state.borrow_mut().apply_at_for_hand(
                                &mut canvas,
                                &mut selection_state,
                                position,
                                PaintHand::Right,
                                bounds,
                                view_orientation,
                            );
                        }
                    }
                }
            }
            // A number-field click just began an in-place edit; typing mode
            // owns the keyboard until a click or Escape ends the session.
            if !typing_mode.is_active() && number_edit.borrow().is_some() {
                typing_mode.begin(typing_reserved_inputs(
                    &effective_painter_bindings.borrow(),
                ));
            }
        } else if frame.input.pointer_down {
            if let Some(cursor) = frame.input.cursor_position {
                let world = to_world(cursor);
                let screen = to_screen(cursor);
                modules.dispatch_captured_pointer_move(screen.x, screen.y);
                apply_layers_panel_action(
                    layers_panel_state.borrow_mut().take_pending_action(),
                    &mut shared_document,
                    &shared_document_paths,
                    &mut shared_action_counter,
                    &session_user_id,
                    &mut active_layer_id,
                    &mut selected_property_id,
                    &mut canvas,
                    &timeline_state,
                    &selection,
                );
                if command_bar.contains(screen.x, screen.y) || modules.is_pointer_captured() {
                    selection_stroke = None;
                    lasso_stroke = None;
                    left_drag_position = None;
                } else {
                    let bounds = *paint_canvas_bounds.borrow();
                    let position = CellPoint {
                        x: world.x,
                        y: world.y,
                        z: world.z,
                    };
                    if paint_surface.contains(screen.x, screen.y)
                        && bounds.contains(position)
                        && text_entry.as_ref().map_or(true, |entry| !entry.is_active())
                    {
                        let stroke_positions = left_drag_position
                            .map(|last| interpolate_cell_path(last, position))
                            .unwrap_or_else(|| vec![position]);
                        if let Some(stroke) = lasso_stroke.as_mut() {
                            if stroke.hand == PaintHand::Left {
                                stroke.extend(&stroke_positions);
                            }
                        } else if let Some(stroke) = selection_stroke.as_mut() {
                            if stroke.hand == PaintHand::Left {
                                let tool_state_ref = tool_state.borrow();
                                for anchor in &stroke_positions {
                                    stroke.extend(tool_state_ref.selection_points_for_hand(
                                        &canvas,
                                        *anchor,
                                        PaintHand::Left,
                                        bounds,
                                        view_orientation,
                                    ));
                                }
                            }
                        } else {
                            let target = tool_state.borrow().hand_state(PaintHand::Left).target;
                            let mut selection_state = selection.borrow_mut();
                            if target == PaintTarget::Image {
                                if let Some((_, block_id)) = left_stroke_start.as_ref() {
                                    stage_image_edit_chunk(
                                        &mut shared_document,
                                        &mut canvas,
                                        &mut tool_state.borrow_mut(),
                                        &mut selection_state,
                                        stroke_positions,
                                        PaintHand::Left,
                                        bounds,
                                        view_orientation,
                                        &active_layer_id,
                                        block_id,
                                    );
                                }
                            } else {
                                for anchor in stroke_positions {
                                    tool_state.borrow_mut().apply_at_for_hand(
                                        &mut canvas,
                                        &mut selection_state,
                                        anchor,
                                        PaintHand::Left,
                                        bounds,
                                        view_orientation,
                                    );
                                }
                            }
                        }
                        left_drag_position = Some(position);
                    }
                }
            }
        } else if frame.input.right_pointer_down {
            if let Some(cursor) = frame.input.cursor_position {
                let world = to_world(cursor);
                let screen = to_screen(cursor);
                modules.dispatch_captured_pointer_move(screen.x, screen.y);
                if command_bar.contains(screen.x, screen.y) || modules.is_pointer_captured() {
                    selection_stroke = None;
                    lasso_stroke = None;
                    right_drag_position = None;
                } else {
                    let bounds = *paint_canvas_bounds.borrow();
                    let position = CellPoint {
                        x: world.x,
                        y: world.y,
                        z: world.z,
                    };
                    if paint_surface.contains(screen.x, screen.y)
                        && bounds.contains(position)
                        && text_entry.as_ref().map_or(true, |entry| !entry.is_active())
                    {
                        let stroke_positions = right_drag_position
                            .map(|last| interpolate_cell_path(last, position))
                            .unwrap_or_else(|| vec![position]);
                        if let Some(stroke) = lasso_stroke.as_mut() {
                            if stroke.hand == PaintHand::Right {
                                stroke.extend(&stroke_positions);
                            }
                        } else if let Some(stroke) = selection_stroke.as_mut() {
                            if stroke.hand == PaintHand::Right {
                                let tool_state_ref = tool_state.borrow();
                                for anchor in &stroke_positions {
                                    stroke.extend(tool_state_ref.selection_points_for_hand(
                                        &canvas,
                                        *anchor,
                                        PaintHand::Right,
                                        bounds,
                                        view_orientation,
                                    ));
                                }
                            }
                        } else {
                            let target = tool_state.borrow().hand_state(PaintHand::Right).target;
                            let mut selection_state = selection.borrow_mut();
                            if target == PaintTarget::Image {
                                if let Some((_, block_id)) = right_stroke_start.as_ref() {
                                    stage_image_edit_chunk(
                                        &mut shared_document,
                                        &mut canvas,
                                        &mut tool_state.borrow_mut(),
                                        &mut selection_state,
                                        stroke_positions,
                                        PaintHand::Right,
                                        bounds,
                                        view_orientation,
                                        &active_layer_id,
                                        block_id,
                                    );
                                }
                            } else {
                                for anchor in stroke_positions {
                                    tool_state.borrow_mut().apply_at_for_hand(
                                        &mut canvas,
                                        &mut selection_state,
                                        anchor,
                                        PaintHand::Right,
                                        bounds,
                                        view_orientation,
                                    );
                                }
                            }
                        }
                        right_drag_position = Some(position);
                    }
                }
            }
        }

        if selection_stroke.is_some()
            && !frame.input.pointer_down
            && !frame.input.right_pointer_down
        {
            if let Some(stroke) = selection_stroke.take() {
                selection
                    .borrow_mut()
                    .apply_plane_points_with_mode(stroke.points, stroke.mode);
                // Mirror the full 3D set into the document channel as an exact
                // replacement — the channel is the shared truth, the plane cache
                // is the interaction surface.
                let points: Vec<CellPoint> = selection.borrow().plane().iter().collect();
                commit_selection_channel(
                    &mut shared_document,
                    &shared_document_paths,
                    points,
                    SelectionMode::Replace,
                    &mut active_layer_id,
                    timeline_state.borrow().current_breath,
                    &mut canvas,
                    &selection,
                    &mut shared_action_counter,
                );
            }
        }

        if !typing_mode.is_active()
            && (frame.input.wheel_delta_x != 0.0 || frame.input.wheel_delta_y != 0.0)
        {
            if let Some(cursor) = frame.input.cursor_position {
                let screen = to_screen(cursor);
                let wheel_handled_by_command_bar = command_bar.on_wheel(
                    screen.x,
                    screen.y,
                    frame.input.wheel_delta_x,
                    frame.input.wheel_delta_y,
                );
                let wheel_handled_by_module = !wheel_handled_by_command_bar
                    && modules
                        .dispatch_wheel_at(
                            screen.x,
                            screen.y,
                            frame.input.wheel_delta_x,
                            frame.input.wheel_delta_y,
                        )
                        .is_some();
                if !wheel_handled_by_command_bar && !wheel_handled_by_module {
                    if paint_viewport.contains(screen.x, screen.y) {
                        match *drawing_space_wheel_mode.borrow() {
                            DrawingSpaceWheelMode::Time => {
                                // Wheel steps the playhead through time,
                                // wrapping at the loop-window edges: up =
                                // forward (end wraps to start), down =
                                // backward (start wraps to end).
                                let window = shared_document.document_window();
                                let current = timeline_state.borrow().current_breath;
                                let next = if frame.input.wheel_delta_y > 0.0 {
                                    if current >= window.end_breath {
                                        window.start_breath
                                    } else {
                                        current + 1
                                    }
                                } else if current <= window.start_breath {
                                    window.end_breath
                                } else {
                                    current - 1
                                };
                                timeline_state.borrow_mut().set_current_breath(next);
                                sync_canvas_from_active_layer(
                                    &shared_document,
                                    &active_layer_id,
                                    next,
                                    &mut canvas,
                                );
                            }
                            mode => apply_drawing_space_scroll(
                                &mut state.camera,
                                mode,
                                frame.input.wheel_delta_x,
                                frame.input.wheel_delta_y,
                            ),
                        }
                    } else {
                        apply_hud_scroll(
                            &mut state.camera,
                            frame.input.wheel_delta_x,
                            frame.input.wheel_delta_y,
                        );
                    }
                }
            }
        }

        if !frame.input.pointer_down {
            left_drag_position = None;
        }
        if !frame.input.right_pointer_down {
            right_drag_position = None;
        }

        if (left_pointer_was_down && !frame.input.pointer_down)
            || (right_pointer_was_down && !frame.input.right_pointer_down)
        {
            let screen = frame
                .input
                .cursor_position
                .map(to_screen)
                .unwrap_or_else(|| to_screen([0.0, 0.0]));
            // Broadcast the release to every module, not just the captured
            // one: a drag that never requested capture (or lost it) would
            // otherwise keep following the pointer through hover moves
            // forever, since its Up never arrived.
            modules.dispatch_pointer_up_all(screen.x, screen.y);
        }
        // The pointer-up dispatch is where a layers-panel drag commits its single
        // timing action (or block swap), so the pending action must be applied right
        // here — waiting for the next click/drag frame would leave the commit stranded
        // in the queue.
        if (left_pointer_was_down && !frame.input.pointer_down)
            || (right_pointer_was_down && !frame.input.right_pointer_down)
        {
            // One committed action per stroke: the drag's staged patches become a
            // single CellPatchSet record, written once on release (one undo per
            // stroke). Hands that didn't paint are no-ops.
            // A lasso bound closes here: the enclosed cells fill through the
            // hand's tool state (image target) or select through the hand's
            // resolved mode (selection target), then share the stroke commit.
            if let Some(stroke) = lasso_stroke.take() {
                let target = tool_state.borrow().hand_state(stroke.hand).target;
                if target == PaintTarget::Image {
                    tool_state.borrow_mut().apply_lasso_for_hand(
                        &mut canvas,
                        &selection.borrow(),
                        &stroke.path,
                        stroke.hand,
                        view_orientation,
                    );
                } else {
                    {
                        let mut selection_state = selection.borrow_mut();
                        let mode = thaum_painter_domain::painter_tools::shared::selection_behavior(
                            tool_state.borrow().tool_for_hand(stroke.hand).id(),
                        )
                        .resolve(selection_state.mode(), SelectionMode::Subtract);
                        selection_state.apply_plane_points_with_mode(
                            tool_state.borrow().lasso_selection_points(
                                &stroke.path,
                                stroke.hand,
                                view_orientation,
                            ),
                            mode,
                        );
                    }
                    let points: Vec<CellPoint> = selection.borrow().plane().iter().collect();
                    commit_selection_channel(
                        &mut shared_document,
                        &shared_document_paths,
                        points,
                        SelectionMode::Replace,
                        &mut active_layer_id,
                        timeline_state.borrow().current_breath,
                        &mut canvas,
                        &selection,
                        &mut shared_action_counter,
                    );
                }
            }
            if let Err(err) = commit_staged_paint_stroke(
                &mut shared_document,
                &shared_document_paths,
                &mut shared_action_counter,
                &session_user_id,
                &active_layer_id,
                &mut canvas,
                left_stroke_start.take(),
            ) {
                eprintln!("stroke commit failed (kept in memory): {err:#}");
            }
            if let Err(err) = commit_staged_paint_stroke(
                &mut shared_document,
                &shared_document_paths,
                &mut shared_action_counter,
                &session_user_id,
                &active_layer_id,
                &mut canvas,
                right_stroke_start.take(),
            ) {
                eprintln!("stroke commit failed (kept in memory): {err:#}");
            }
            apply_layers_panel_action(
                layers_panel_state.borrow_mut().take_pending_action(),
                &mut shared_document,
                &shared_document_paths,
                &mut shared_action_counter,
                &session_user_id,
                &mut active_layer_id,
                &mut selected_property_id,
                &mut canvas,
                &timeline_state,
                &selection,
            );
        }
        left_pointer_was_down = frame.input.pointer_down;
        right_pointer_was_down = frame.input.right_pointer_down;

        // Continuous hover, independent of clicking/dragging: lets a
        // seamless module reveal its gizmo bar only while moused over.
        let hover_point = frame
            .input
            .cursor_position
            .map(to_screen)
            .map(|screen| (screen.x, screen.y));
        modules.update_hover_at(hover_point);

        modules.remove_closed_modules();

        let session_state = build_user_session_state(
            &session_user_id,
            state.camera,
            &modules,
            &ui_palette,
            &command_bar,
            Some(&active_layer_id),
            timeline_state.borrow().current_breath,
            *drawing_space_wheel_mode.borrow(),
            selection.borrow().mode(),
            &tool_state.borrow(),
            &controls_profile.borrow(),
        );
        if let Ok(session_text) = serde_json::to_string_pretty(&session_state) {
            if last_saved_session_text.as_ref() != Some(&session_text)
                && last_save_at.elapsed() >= Duration::from_millis(150)
            {
                if save_painter_user_session_state(&session_state_path, &session_state).is_ok() {
                    last_saved_session_text = Some(session_text);
                    last_save_at = Instant::now();
                }
            }
        }

        let mut groups = build_document_layer_cell_groups(&shared_document, timeline_state.borrow().current_breath);
        groups.extend(modules.iter().map(|module| module.draw()));
        groups.extend(build_plane_selection_cell_groups(
            &selection.borrow(),
            selection_stroke.as_ref(),
            &canvas,
            ui_palette.get(UiColorRole::Vivid),
        ));
        // In-progress lasso bound: the bound path itself, plus a live interior
        // preview built from the same lasso seam release consumes — each cell
        // flashes between its current character/weight recolored vivid and the
        // exact appearance release will paint. Never committed before release.
        if let Some(stroke) = lasso_stroke.as_ref() {
            groups.push(build_lasso_path_cell_group(stroke));
            let target = tool_state.borrow().hand_state(stroke.hand).target;
            let previews = if target == PaintTarget::Image {
                tool_state.borrow().lasso_preview_cells(
                    &canvas,
                    &selection.borrow(),
                    &stroke.path,
                    stroke.hand,
                    view_orientation,
                )
            } else {
                // Selection-target lasso: the drawing will not change, so both
                // flash halves show the cell as currently drawn (vivid recolor
                // vs true) — the selection display behavior.
                tool_state.borrow().lasso_select_preview_cells(
                    &canvas,
                    &stroke.path,
                    stroke.hand,
                    view_orientation,
                )
            };
            groups.extend(build_lasso_preview_cell_groups(
                &previews,
                ui_palette.get(UiColorRole::Vivid),
            ));
        }
        // Typing cursor: a flashing bright block on the cell that will receive
        // the next character. Composed on top like the selection overlay, never
        // staged into the canvas or document, so it is never part of the drawing.
        let now = Instant::now();
        if now.duration_since(cursor_blink_at) >= CURSOR_BLINK_INTERVAL {
            cursor_blink_on = !cursor_blink_on;
            cursor_blink_at = now;
        }
        if typing_mode.is_active() {
            if let Some(entry) = text_entry.as_ref() {
                let glyph = if cursor_blink_on { '█' } else { '□' };
                groups.push(cursor_overlay_group(entry.cursor_point(), glyph));
            }
        }
        groups.push(command_bar.draw());
        state.composition = Composition::ordered(groups).with_natural_pass_order();
        Ok(())
    })
}
