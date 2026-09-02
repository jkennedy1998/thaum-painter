use std::{
    cell::RefCell,
    collections::BTreeSet,
    env, fs,
    io::{self, Write},
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use rfd::FileDialog;
use thaum_painter_domain::{
    append_action_record, camera_viewport::{
        apply_drawing_space_scroll, apply_hud_scroll, pan_hud_and_focus_right,
        pan_hud_and_focus_up, reorient_camera_around_viewport_center, sync_canvas_bounds_to_camera,
        INITIAL_PAINT_CANVAS_BOUNDS, INITIAL_PAINT_CANVAS_VIEWPORT,
    }, load_or_create_shared_document, save_shared_document_snapshot, Canvas,
    CanvasBounds, DrawingSpaceWheelMode, GraphicPickerModule, HandSettingsModule,
    LayerPropertyKind, LayerRow, LayersPanelAction, LayersPanelModule, LayersPanelState,
    MaterialPickerModule, MergeDirection, PaintCanvasBoundsModule, PaintColorBlockModule,
    PaintColorPickerModule, PaintHand,
    PaintTarget, PaintTool, PainterSelection, PainterUserSessionState, PersistedPainterUiState,
    PropertyBlockMergeDirection, PropertyTrackBlock, PropertyTrackRow, SelectionMode,
    SharedCellPatch, SharedDocumentActionRecord, SharedDocumentFile, SharedDocumentPaths,
    SharedDocumentRuntime, SharedSelectionWriteMode, TimelineState,
    ToolDef, ToolState, ToolboxModule, DEFAULT_SELECTION_CHANNEL_ID,
};
use thaum_renderer_boot::{
    boot_renderer, cell_clip_size_for_state, run_renderer_window_with_state_frame_provider,
    BootConfig, BootState,
};
use thaum_renderer_domain::{
    remap_surface_units_to_active_plane_world,
    remap_surface_units_to_flat_2d_local, Cell, CellColor,
    CellGraphic, CellGroup, CellPoint, CellWeight, CommandBar, CommandBarButton,
    CommandBarClickOutcome, Composition, ModulePointerButton, ModulePointerEvent, ModuleRect,
    ModuleRegistry, PersistedRendererUiSessionState, UiColorRole, UiCustomizationModule,
    UiPalette, WorldPoint,
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

fn shared_document_id() -> String {
    env::var("THAUM_SHARED_DOCUMENT_ID").unwrap_or_else(|_| "local-document".to_string())
}

fn painter_shared_document_paths(document_id: &str) -> SharedDocumentPaths {
    SharedDocumentPaths::new(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../artifacts/shared-documents")
            .join(document_id),
    )
}

fn default_shared_document(document_id: &str) -> SharedDocumentFile {
    SharedDocumentFile::single_layer(
        document_id,
        "Untitled Document",
        INITIAL_LAYER_ID,
        INITIAL_LAYER_NAME,
    )
}

fn painter_repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn legacy_painter_file_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../context/painter/painter-files")
}

fn migrate_legacy_painter_file_root(legacy_root: &Path, target_root: &Path) {
    if target_root.exists() || !legacy_root.exists() {
        return;
    }
    if let Some(parent) = target_root.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::rename(legacy_root, target_root);
}

fn painter_file_root() -> PathBuf {
    if let Ok(path) = env::var("THAUM_PAINTER_FILE_ROOT") {
        return PathBuf::from(path);
    }

    let target_root = painter_repo_root().join("context/painter/painter-files");
    migrate_legacy_painter_file_root(&legacy_painter_file_root(), &target_root);
    target_root
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn load_document_from_root(root: &Path) -> Result<(SharedDocumentPaths, SharedDocumentRuntime)> {
    let document_id = root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("untitled-document")
        .to_string();
    let paths = SharedDocumentPaths::new(root.to_path_buf());
    let runtime = load_or_create_shared_document(&paths, default_shared_document(&document_id))?;
    Ok((paths, runtime))
}

fn new_unsaved_document() -> SharedDocumentRuntime {
    let document_id = format!("document-{}", action_timestamp_string());
    SharedDocumentRuntime::new(default_shared_document(&document_id))
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
    eprintln!(
        "thaum-painter: {action} dialog returned no selection. backend={}. root={}",
        native_file_dialog_backend_name(),
        display_path(file_root)
    );
    #[cfg(target_os = "linux")]
    eprintln!(
        "thaum-painter: on Linux this backend depends on a live desktop-portal session; if no dialog appears, check xdg-desktop-portal / DBus availability in the current desktop session."
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

const INITIAL_LAYER_ID: &str = "layer-1";
const INITIAL_LAYER_NAME: &str = "Layer 1";

/// Renders `canvas`'s live-painted cells as one `CellGroup` in the renderer's
/// real rotating 3D intake path, not as module chrome, so the painter canvas
/// lives in scene space while the UI panels stay in the flat 2D layer.
fn build_paint_canvas_cell_group(canvas: &Canvas) -> CellGroup {
    let mut group = CellGroup::new(WorldPoint { x: 0, y: 0, z: 0 });
    for (position, painted) in canvas {
        group.insert(Cell {
            position: *position,
            graphic: painted.graphic.clone(),
            color: painted.color.to_cell_color(),
            weight: CellWeight::from_index_clamped(painted.weight_index as i32),
            ..Cell::default()
        });
    }
    group
}

fn resolved_active_layer_id(
    runtime: &SharedDocumentRuntime,
    preferred_layer_id: Option<&str>,
) -> String {
    if let Some(layer_id) = preferred_layer_id.filter(|layer_id| runtime.document.has_layer(layer_id)) {
        return layer_id.to_string();
    }
    runtime
        .document
        .first_layer_id()
        .unwrap_or(INITIAL_LAYER_ID)
        .to_string()
}

fn next_layer_number(runtime: &SharedDocumentRuntime) -> usize {
    let mut number = 1;
    loop {
        if !runtime.document.has_layer(&format!("layer-{number}")) {
            return number;
        }
        number += 1;
    }
}

fn create_layer(runtime: &mut SharedDocumentRuntime) -> String {
    let number = next_layer_number(runtime);
    let layer_id = format!("layer-{number}");
    let layer_name = format!("Layer {number}");
    runtime.add_layer(layer_id.clone(), layer_name);
    layer_id
}

fn build_document_layer_cell_groups(runtime: &SharedDocumentRuntime, current_breath: u32) -> Vec<CellGroup> {
    runtime
        .layers()
        .iter()
        .filter_map(|layer| runtime.canvas_for_layer(&layer.layer_id, current_breath))
        .map(build_paint_canvas_cell_group)
        .collect()
}

#[derive(Debug, Clone)]
struct SelectionStroke {
    hand: PaintHand,
    mode: SelectionMode,
    points: BTreeSet<CellPoint>,
}

impl SelectionStroke {
    fn new(hand: PaintHand, mode: SelectionMode) -> Self {
        Self {
            hand,
            mode,
            points: BTreeSet::new(),
        }
    }

    fn extend<I>(&mut self, points: I)
    where
        I: IntoIterator<Item = CellPoint>,
    {
        self.points.extend(points);
    }
}

fn interpolate_cell_path(start: CellPoint, end: CellPoint) -> Vec<CellPoint> {
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    let dz = end.z - start.z;
    let steps = dx.abs().max(dy.abs()).max(dz.abs());
    if steps == 0 {
        return vec![start];
    }

    let mut points = Vec::with_capacity(steps as usize + 1);
    for step in 0..=steps {
        let t = step as f32 / steps as f32;
        points.push(CellPoint {
            x: start.x + (dx as f32 * t).round() as i32,
            y: start.y + (dy as f32 * t).round() as i32,
            z: start.z + (dz as f32 * t).round() as i32,
        });
    }
    points.dedup();
    points
}

fn build_plane_selection_cell_group(
    selection: &PainterSelection,
    stroke: Option<&SelectionStroke>,
    flash_on: bool,
) -> CellGroup {
    let preview = match stroke {
        Some(stroke) => {
            selection.preview_plane_with_mode(stroke.points.iter().copied(), stroke.mode)
        }
        None => selection.plane().clone(),
    };

    let glyph = if flash_on { '□' } else { '■' };
    let color = if flash_on {
        [1.0, 0.9, 0.25, 1.0]
    } else {
        [0.8, 0.6, 0.1, 1.0]
    };

    let mut group = CellGroup::new(WorldPoint { x: 0, y: 0, z: 0 });
    // Render every selected cell, not just a plane slice or border: selection is a
    // 3D bitmap shared across depths, so cells light up at any depth the camera can
    // see (the composition already shows whatever falls inside the camera window).
    for position in preview.iter() {
        group.insert(Cell {
            position,
            graphic: CellGraphic::Glyph(glyph),
            color: CellColor::Flat(color),
            weight: CellWeight::from_index_clamped(3),
            ..Cell::default()
        });
    }
    group
}

fn file_menu_buttons() -> Vec<CommandBarButton> {
    vec![
        CommandBarButton::new("file:new", "NEW"),
        CommandBarButton::new("file:open", "OPEN"),
        CommandBarButton::new("file:save", "SAVE"),
        CommandBarButton::new("file:save-as", "SAVE AS"),
    ]
}

fn build_selected_layer_property_rows(
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
fn apply_layers_panel_action(
    action: Option<LayersPanelAction>,
    shared_document: &mut SharedDocumentRuntime,
    shared_document_paths: &SharedDocumentPaths,
    shared_action_counter: &mut u64,
    session_user_id: &str,
    active_layer_id: &mut String,
    selected_property_id: &mut Option<String>,
    canvas: &mut Canvas,
    timeline_state: &Rc<RefCell<TimelineState>>,
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
                sync_canvas_from_active_layer(shared_document, active_layer_id, current_breath, canvas);
            }
        }
        LayersPanelAction::ToggleAutoKey => {
            timeline_state.borrow_mut().toggle_auto_key();
        }
        LayersPanelAction::SetCurrentBreath(breath) => {
            timeline_state.borrow_mut().set_current_breath(breath);
            // Scrubbing the playhead switches which raster block the edit surface shows.
            sync_canvas_from_active_layer(shared_document, active_layer_id, breath, canvas);
        }
        LayersPanelAction::SetLayerTiming(layer_id, start_breath, length_breaths) => {
            shared_document.set_layer_timing(&layer_id, start_breath, length_breaths);
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
            let Some(new_block_id) = shared_document
                .split_property_block(&layer_id, &property_id, &block_id, split_breath)
            else {
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
            shared_document.merge_blank_property_block(&layer_id, &property_id, &block_id, direction);
        }
        LayersPanelAction::SwapPropertyBlocks(layer_id, property_id, source_block_id, target_block_id) => {
            shared_document.swap_property_blocks(&layer_id, &property_id, &source_block_id, &target_block_id);
        }
    }
    if document_mutated {
        if let Err(error) = save_shared_document_snapshot(shared_document_paths, shared_document) {
            eprintln!("failed to save document snapshot: {error}");
        }
    }
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

fn build_user_session_state(
    user_id: &str,
    camera: thaum_renderer_domain::Camera,
    modules: &ModuleRegistry,
    ui_palette: &UiPalette,
    command_bar: &CommandBar,
    current_document_root: Option<&Path>,
    active_layer_id: Option<&str>,
    drawing_space_wheel_mode: DrawingSpaceWheelMode,
    selection_mode: SelectionMode,
    tool_state: &ToolState,
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
            current_document_root.map(path_to_string).as_deref(),
            active_layer_id,
            tool_state,
        ),
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

fn action_timestamp_string() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .to_string()
}

fn next_action_id(counter: &mut u64) -> String {
    *counter += 1;
    format!("action-{}-{counter}", action_timestamp_string())
}

fn collect_canvas_patches(before: &Canvas, after: &Canvas) -> Vec<SharedCellPatch> {
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

fn append_and_apply_shared_action(
    runtime: &mut SharedDocumentRuntime,
    paths: &SharedDocumentPaths,
    action: SharedDocumentActionRecord,
) -> Result<()> {
    append_action_record(&paths.actions_file_path, &action)?;
    runtime.apply_action_record(action);
    Ok(())
}

fn sync_canvas_from_active_layer(
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
fn stage_image_edit_chunk(
    runtime: &mut SharedDocumentRuntime,
    canvas: &mut Canvas,
    tool_state: &mut ToolState,
    selection: &mut PainterSelection,
    positions: impl IntoIterator<Item = CellPoint>,
    hand: PaintHand,
    bounds: CanvasBounds,
    layer_id: &str,
    block_id: &str,
) {
    let before = canvas.clone();
    let mut candidate = before.clone();
    for position in positions {
        tool_state.apply_at_for_hand(&mut candidate, selection, position, hand, bounds);
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

/// Commits one finished stroke as a single `CellPatchSet` record — one undo per
/// stroke. The runtime canvas already holds the staged content, so applying the
/// record is idempotent; it only registers the undo bookkeeping.
fn commit_staged_paint_stroke(
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
        next_action_id(action_counter),
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

/// Mirrors a committed selection change into the document's selection channel and
/// persists it. Selection is document-owned (per file, one shared 3D bitmap on the
/// canvas coordinate system), so the plane cache inside `PainterSelection` is only
/// the interaction surface; the channel is the truth. No-ops skip the snapshot save.
fn commit_selection_channel<I>(
    runtime: &mut SharedDocumentRuntime,
    paths: &SharedDocumentPaths,
    points: I,
    mode: SelectionMode,
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
            eprintln!("failed to save document snapshot: {error}");
        }
    }
}

fn apply_shared_history_action(
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
            next_action_id(action_counter),
            runtime.document.document_id.clone(),
            active_layer_id,
            user_id,
            action_timestamp_string(),
            revert.patches,
            Some(revert.block_id),
            revert.action_id,
        );
        append_action_record(&paths.actions_file_path, &record)?;
        runtime.push_history_record(record);
    }
    sync_canvas_from_active_layer(runtime, active_layer_id, current_breath, canvas);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use thaum_painter_domain::SharedDocumentSelection;

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
    fn resolved_active_layer_id_falls_back_to_first_document_layer() {
        let runtime = SharedDocumentRuntime::new(SharedDocumentFile {
            file_kind: thaum_painter_domain::SHARED_DOCUMENT_KIND.to_string(),
            schema_version: thaum_painter_domain::SHARED_DOCUMENT_SCHEMA_VERSION,
            document_id: "doc-1".to_string(),
            title: "Doc".to_string(),
            layers: vec![
                thaum_painter_domain::SharedDocumentLayer {
                    layer_id: "layer-a".to_string(),
                    name: "Layer A".to_string(),
                    visible: true,
                    locked: false,
                    start_breath: 0,
                    length_breaths: 24,
                    property_tracks: vec![],
                },
                thaum_painter_domain::SharedDocumentLayer {
                    layer_id: "layer-b".to_string(),
                    name: "Layer B".to_string(),
                    visible: true,
                    locked: false,
                    start_breath: 0,
                    length_breaths: 24,
                    property_tracks: vec![],
                },
            ],
            revision: 0,
            selection: SharedDocumentSelection::default(),
        });

        assert_eq!(resolved_active_layer_id(&runtime, Some("missing")), "layer-a");
    }

    #[test]
    fn create_layer_picks_the_next_open_layer_number() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile {
            file_kind: thaum_painter_domain::SHARED_DOCUMENT_KIND.to_string(),
            schema_version: thaum_painter_domain::SHARED_DOCUMENT_SCHEMA_VERSION,
            document_id: "doc-1".to_string(),
            title: "Doc".to_string(),
            layers: vec![
                thaum_painter_domain::SharedDocumentLayer {
                    layer_id: "layer-1".to_string(),
                    name: "Layer 1".to_string(),
                    visible: true,
                    locked: false,
                    start_breath: 0,
                    length_breaths: 24,
                    property_tracks: vec![],
                },
                thaum_painter_domain::SharedDocumentLayer {
                    layer_id: "layer-3".to_string(),
                    name: "Layer 3".to_string(),
                    visible: true,
                    locked: false,
                    start_breath: 0,
                    length_breaths: 24,
                    property_tracks: vec![],
                },
            ],
            revision: 0,
            selection: SharedDocumentSelection::default(),
        });

        assert_eq!(create_layer(&mut runtime), "layer-2");
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
}

fn main() -> Result<()> {
    let mut config = BootConfig::default();
    config.asset_root = development_asset_root();
    config.window.title = "thaum-painter".to_string();

    let session_user_id = session_user_id();
    let session_state_path = painter_session_state_path(&session_user_id);
    let persisted_session = load_painter_user_session_state(&session_state_path)?;
    let shared_document_id = shared_document_id();
    let mut current_document_root = persisted_session
        .as_ref()
        .and_then(|session| session.painter.current_document_root.as_deref())
        .map(PathBuf::from);
    let (mut shared_document_paths, mut shared_document) = match current_document_root.as_deref() {
        Some(root) => load_document_from_root(root).unwrap_or_else(|_| {
            current_document_root = None;
            let fallback_paths = painter_shared_document_paths(&shared_document_id);
            let fallback_document = load_or_create_shared_document(
                &fallback_paths,
                default_shared_document(&shared_document_id),
            )
            .expect("failed to load fallback shared document");
            (fallback_paths, fallback_document)
        }),
        None => {
            let fallback_paths = painter_shared_document_paths(&shared_document_id);
            let fallback_document = load_or_create_shared_document(
                &fallback_paths,
                default_shared_document(&shared_document_id),
            )?;
            (fallback_paths, fallback_document)
        }
    };

    let mut state = boot_renderer(config)?;
    if let Some(session) = &persisted_session {
        session.renderer.camera.apply_to_runtime(&mut state.camera);
    }

    let mut modules = ModuleRegistry::new();
    let ui_palette = UiPalette::default();
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
            vec![
                ToolDef {
                    tool: PaintTool::Brush,
                    icon: '✎',
                    label: "Brush",
                },
                ToolDef {
                    tool: PaintTool::Erase,
                    icon: '◫',
                    label: "Erase",
                },
                ToolDef {
                    tool: PaintTool::Fill,
                    icon: '▧',
                    label: "Fill",
                },
            ],
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
    let mut selection_stroke: Option<SelectionStroke> = None;
    let mut active_layer_id = resolved_active_layer_id(
        &shared_document,
        persisted_session
            .as_ref()
            .and_then(|session| session.painter.active_layer_id.as_deref()),
    );
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
            match key {
                // Inverted from the camera's own right/up so the content
                // visually moves the way the key points, not the way the
                // camera's aim point moves.
                KeyCode::KeyA if hovering_canvas_bounds => state.camera.pan_focus_right(1),
                KeyCode::KeyD if hovering_canvas_bounds => state.camera.pan_focus_right(-1),
                KeyCode::KeyW if hovering_canvas_bounds => state.camera.pan_focus_up(-1),
                KeyCode::KeyS if hovering_canvas_bounds => state.camera.pan_focus_up(1),
                KeyCode::KeyA => pan_hud_and_focus_right(&mut state.camera, 1),
                KeyCode::KeyD => pan_hud_and_focus_right(&mut state.camera, -1),
                KeyCode::KeyW => pan_hud_and_focus_up(&mut state.camera, -1),
                KeyCode::KeyS => pan_hud_and_focus_up(&mut state.camera, 1),
                _ => {}
            }
        }
        for key in &frame.input.just_pressed_keys {
            match key {
                KeyCode::Numpad4 => reorient_camera_around_viewport_center(
                    &mut state.camera,
                    *paint_canvas_viewport.borrow(),
                    |camera| camera.swing_left(),
                ),
                KeyCode::Numpad6 => reorient_camera_around_viewport_center(
                    &mut state.camera,
                    *paint_canvas_viewport.borrow(),
                    |camera| camera.swing_right(),
                ),
                KeyCode::Numpad8 => reorient_camera_around_viewport_center(
                    &mut state.camera,
                    *paint_canvas_viewport.borrow(),
                    |camera| camera.swing_up(),
                ),
                KeyCode::Numpad2 => reorient_camera_around_viewport_center(
                    &mut state.camera,
                    *paint_canvas_viewport.borrow(),
                    |camera| camera.swing_down(),
                ),
                KeyCode::Numpad7 => reorient_camera_around_viewport_center(
                    &mut state.camera,
                    *paint_canvas_viewport.borrow(),
                    |camera| camera.roll = camera.roll.rotate_counter_clockwise(),
                ),
                KeyCode::Numpad9 => reorient_camera_around_viewport_center(
                    &mut state.camera,
                    *paint_canvas_viewport.borrow(),
                    |camera| camera.roll = camera.roll.rotate_clockwise(),
                ),
                KeyCode::Numpad1 => state.camera.pan_focus_depth(-1),
                KeyCode::Numpad3 => state.camera.pan_focus_depth(1),
                KeyCode::Minus | KeyCode::NumpadSubtract => state.camera.zoom_out(),
                KeyCode::Equal | KeyCode::NumpadAdd => state.camera.zoom_in(),
                KeyCode::Digit1 => selection.borrow_mut().set_mode(SelectionMode::Replace),
                KeyCode::Digit2 => selection.borrow_mut().set_mode(SelectionMode::Additive),
                KeyCode::Digit3 => selection.borrow_mut().set_mode(SelectionMode::Subtract),
                KeyCode::Digit4 => selection.borrow_mut().set_mode(SelectionMode::Intersect),
                KeyCode::KeyC => {
                    selection.borrow_mut().clear_plane();
                    commit_selection_channel(
                        &mut shared_document,
                        &shared_document_paths,
                        std::iter::empty(),
                        SelectionMode::Replace,
                    );
                }
                KeyCode::KeyI => {
                    selection.borrow_mut().invert_plane();
                    let points: Vec<CellPoint> =
                        selection.borrow().plane().iter().collect();
                    commit_selection_channel(
                        &mut shared_document,
                        &shared_document_paths,
                        points,
                        SelectionMode::Replace,
                    );
                }
                KeyCode::KeyX => {
                    selection.borrow_mut().select_all_plane();
                    let points: Vec<CellPoint> =
                        selection.borrow().plane().iter().collect();
                    commit_selection_channel(
                        &mut shared_document,
                        &shared_document_paths,
                        points,
                        SelectionMode::Replace,
                    );
                }
                KeyCode::KeyZ => apply_shared_history_action(
                    &mut shared_document,
                    &shared_document_paths,
                    &mut shared_action_counter,
                    &session_user_id,
                    &active_layer_id,
                    &mut canvas,
                    true,
                    timeline_state.borrow().current_breath,
                )?,
                KeyCode::KeyY => apply_shared_history_action(
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

        sync_canvas_bounds_to_camera(
            &paint_canvas_viewport,
            &paint_canvas_bounds,
            &selection,
            &state.camera,
        );

        let camera = state.camera;
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
        command_bar.set_nested_buttons("menu:modules", module_menu_buttons(&modules));
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
        );
        command_bar.update_layout(cell_clip_size, state.camera.hud_pan_offset);
        let command_bar_hover = frame.input.cursor_position.map(to_screen);
        command_bar.set_pointer_position(command_bar_hover.map(|screen| (screen.x, screen.y)));
        let paint_viewport = *paint_canvas_viewport.borrow();
        let paint_surface = PaintCanvasBoundsModule::content_rect(paint_viewport);

        if let Some(click) = frame.input.just_clicked {
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
            let handled_module = !handled_command_bar
                && modules
                    .dispatch_pointer_event_at(
                        screen.x,
                        screen.y,
                        ModulePointerEvent::Click {
                            x: screen.x,
                            y: screen.y,
                            button: ModulePointerButton::Left,
                        },
                    )
                    .is_some();
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
            );
            if handled_command_bar || handled_module || modules.is_pointer_captured() {
                selection_stroke = None;
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
                {
                    left_drag_position = Some(position);
                    if tool_state.borrow().hand_state(PaintHand::Left).target
                        == PaintTarget::Selection
                    {
                        let mode = match tool_state.borrow().tool_for_hand(PaintHand::Left) {
                            PaintTool::Erase => SelectionMode::Subtract,
                            PaintTool::Brush | PaintTool::Fill => selection.borrow().mode(),
                        };
                        let mut stroke = SelectionStroke::new(PaintHand::Left, mode);
                        stroke.extend(tool_state.borrow().selection_points_for_hand(
                            &canvas,
                            position,
                            PaintHand::Left,
                            bounds,
                        ));
                        selection_stroke = Some(stroke);
                    } else {
                        let target = tool_state.borrow().hand_state(PaintHand::Left).target;
                        let mut selection_state = selection.borrow_mut();
                        if target == PaintTarget::Image {
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
                                    &active_layer_id,
                                    &block_id,
                                );
                            }
                        } else {
                            tool_state.borrow_mut().apply_at_for_hand(
                                &mut canvas,
                                &mut selection_state,
                                position,
                                PaintHand::Left,
                                bounds,
                            );
                        }
                    }
                }
            }
        } else if let Some(click) = frame.input.just_right_clicked {
            let world = to_world(click);
            let screen = to_screen(click);
            let handled_command_bar = command_bar.contains(screen.x, screen.y);
            let handled_module = !handled_command_bar
                && modules
                    .dispatch_pointer_event_at(
                        screen.x,
                        screen.y,
                        ModulePointerEvent::Click {
                            x: screen.x,
                            y: screen.y,
                            button: ModulePointerButton::Right,
                        },
                    )
                    .is_some();
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
            );
            if handled_command_bar || handled_module || modules.is_pointer_captured() {
                selection_stroke = None;
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
                {
                    right_drag_position = Some(position);
                    if tool_state.borrow().hand_state(PaintHand::Right).target
                        == PaintTarget::Selection
                    {
                        let mode = match tool_state.borrow().tool_for_hand(PaintHand::Right) {
                            PaintTool::Erase => SelectionMode::Subtract,
                            PaintTool::Brush | PaintTool::Fill => selection.borrow().mode(),
                        };
                        let mut stroke = SelectionStroke::new(PaintHand::Right, mode);
                        stroke.extend(tool_state.borrow().selection_points_for_hand(
                            &canvas,
                            position,
                            PaintHand::Right,
                            bounds,
                        ));
                        selection_stroke = Some(stroke);
                    } else {
                        let target = tool_state.borrow().hand_state(PaintHand::Right).target;
                        let mut selection_state = selection.borrow_mut();
                        if target == PaintTarget::Image {
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
                                    &active_layer_id,
                                    &block_id,
                                );
                            }
                        } else {
                            tool_state.borrow_mut().apply_at_for_hand(
                                &mut canvas,
                                &mut selection_state,
                                position,
                                PaintHand::Right,
                                bounds,
                            );
                        }
                    }
                }
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
                );
                if command_bar.contains(screen.x, screen.y) || modules.is_pointer_captured() {
                    selection_stroke = None;
                    left_drag_position = None;
                } else {
                    let bounds = *paint_canvas_bounds.borrow();
                    let position = CellPoint {
                        x: world.x,
                        y: world.y,
                        z: world.z,
                    };
                    if paint_surface.contains(screen.x, screen.y) && bounds.contains(position) {
                        let stroke_positions = left_drag_position
                            .map(|last| interpolate_cell_path(last, position))
                            .unwrap_or_else(|| vec![position]);
                        if let Some(stroke) = selection_stroke.as_mut() {
                            if stroke.hand == PaintHand::Left {
                                let tool_state_ref = tool_state.borrow();
                                for anchor in &stroke_positions {
                                    stroke.extend(tool_state_ref.selection_points_for_hand(
                                        &canvas,
                                        *anchor,
                                        PaintHand::Left,
                                        bounds,
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
                    right_drag_position = None;
                } else {
                    let bounds = *paint_canvas_bounds.borrow();
                    let position = CellPoint {
                        x: world.x,
                        y: world.y,
                        z: world.z,
                    };
                    if paint_surface.contains(screen.x, screen.y) && bounds.contains(position) {
                        let stroke_positions = right_drag_position
                            .map(|last| interpolate_cell_path(last, position))
                            .unwrap_or_else(|| vec![position]);
                        if let Some(stroke) = selection_stroke.as_mut() {
                            if stroke.hand == PaintHand::Right {
                                let tool_state_ref = tool_state.borrow();
                                for anchor in &stroke_positions {
                                    stroke.extend(tool_state_ref.selection_points_for_hand(
                                        &canvas,
                                        *anchor,
                                        PaintHand::Right,
                                        bounds,
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
                );
            }
        }

        if frame.input.wheel_delta_x != 0.0 || frame.input.wheel_delta_y != 0.0 {
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
                        apply_drawing_space_scroll(
                            &mut state.camera,
                            *drawing_space_wheel_mode.borrow(),
                            frame.input.wheel_delta_x,
                            frame.input.wheel_delta_y,
                        );
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
            modules.dispatch_captured_pointer_up(screen.x, screen.y);
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
            commit_staged_paint_stroke(
                &mut shared_document,
                &shared_document_paths,
                &mut shared_action_counter,
                &session_user_id,
                &active_layer_id,
                &mut canvas,
                left_stroke_start.take(),
            )?;
            commit_staged_paint_stroke(
                &mut shared_document,
                &shared_document_paths,
                &mut shared_action_counter,
                &session_user_id,
                &active_layer_id,
                &mut canvas,
                right_stroke_start.take(),
            )?;
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
            current_document_root.as_deref(),
            Some(&active_layer_id),
            *drawing_space_wheel_mode.borrow(),
            selection.borrow().mode(),
            &tool_state.borrow(),
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

        let raw_breath = state.data_lanes.breath().unwrap_or(0).max(0) as u32;
        let mut groups = build_document_layer_cell_groups(&shared_document, timeline_state.borrow().current_breath);
        groups.extend(modules.iter().map(|module| module.draw()));
        let flash_on = (raw_breath / 6) % 2 == 0;
        groups.push(build_plane_selection_cell_group(
            &selection.borrow(),
            selection_stroke.as_ref(),
            flash_on,
        ));
        groups.push(command_bar.draw());
        state.composition = Composition::ordered(groups).with_natural_pass_order();
        Ok(())
    })
}
