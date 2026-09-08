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
        apply_drawing_space_scroll, apply_hud_scroll, sync_canvas_bounds_to_camera,
        INITIAL_PAINT_CANVAS_BOUNDS, INITIAL_PAINT_CANVAS_VIEWPORT,
    }, document_locations::{
        load_document_from_root, new_unsaved_document, resolve_painter_file_root,
    }, layers_runtime::{
        apply_layers_panel_action, build_selected_layer_property_rows, resolved_active_layer_id,
    }, layers_panel_module::LayersPanelAction, camera_actions::{apply_painter_camera_action, apply_painter_pan_action},
    render_space::build_document_layer_cell_groups, save_shared_document_snapshot, canvas_pointer::{CanvasPointerContext, CanvasPointerStrokes, StampHover}, session_document::{
        commit_staged_paint_stroke, apply_shared_history_action,
        recover_snapshot_conflict, stage_text_entry_change, sync_canvas_from_active_layer,
    }, text_entry::{cursor_overlay_group, TextEntryKey, TextEntryOutcome, TextEntryState}, Canvas, DrawingSpaceWheelMode, LayerRow, LayersPanelState,
    PaintCanvasBoundsModule, PaintHand, PaintTool,
    PainterSelection, PainterUserSessionState, painter_default_camera, PersistedPainterUiState,
    SelectionMode, SharedDocumentPaths, UnsupportedFileError,
    SessionIdentity, SharedDocumentRuntime, TimelineState, apply_painter_selection_action,
    ToolState, CanvasBounds, DEFAULT_SELECTION_CHANNEL_ID,
};
use thaum_renderer_boot::{
    boot_renderer, cell_clip_size_for_state, run_renderer_window_with_state_frame_provider,
    BootConfig, BootState,
};
use thaum_renderer_domain::{
    camera_view_orientation_for_camera, remap_surface_units_to_active_plane_world,
    remap_surface_units_to_flat_2d_local, shape_fade::fade::ShapeFade,
    shape_fade::font_tiles::FontSetTiles, ActionBindingMap, ActionName, CameraDepthLink,
    CameraLayersLink, CellPoint, GlyphFontSet, MAX_VISIBLE_PLANE_RADIUS, TooltipState,
    tooltip_card_group,
    CommandBar, CommandBarButton, CommandBarClickOutcome, Composition, ControlActionRow,
    ControlsProfile, effective_bindings,
    ModulePointerButton, ModulePointerEvent, ModuleRect, ModuleRegistry,
    PersistedRendererUiSessionState, RawInput, TypingMode, TypingRoute, UiColorRole,
    UiPalette,
};
use winit::keyboard::KeyCode;

mod painter_modules;
mod run_log;

/// Asset root: `THAUM_RENDERER_ASSET_ROOT` env override, else a
/// `renderer-assets/` folder next to the running exe (deployed portable
/// layout), else the compiled repo path for in-repo dev runs.
fn resolve_asset_root() -> PathBuf {
    if let Ok(path) = env::var("THAUM_RENDERER_ASSET_ROOT") {
        return PathBuf::from(path);
    }

    if let Ok(exe_path) = env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            let packaged_asset_root = exe_dir.join("renderer-assets");
            if packaged_asset_root.exists() {
                return packaged_asset_root;
            }
        }
    }

    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../thaum-renderer/orchestration/renderer-assets")
}

/// Assembles the canvas-pointer seam's session context from the event loop's
/// live locals. Built per pointer event; borrows never outlive the event.
fn canvas_pointer_context<'a>(
    tool_state: &'a RefCell<ToolState>,
    selection: &'a Rc<RefCell<PainterSelection>>,
    canvas: &'a mut Canvas,
    shared_document: &'a mut SharedDocumentRuntime,
    shared_document_paths: &'a SharedDocumentPaths,
    shared_action_counter: &'a mut u64,
    session_user_id: &'a str,
    active_layer_id: &'a mut String,
) -> CanvasPointerContext<'a> {
    CanvasPointerContext {
        tool_state,
        selection,
        canvas,
        document: shared_document,
        document_paths: shared_document_paths,
        action_counter: shared_action_counter,
        session_user_id,
        active_layer_id,
    }
}

/// Drains one pending layers-panel action into the session seams. The panel
/// enqueues actions on many pointer paths (clicks, captured drags, releases)
/// and every path must drain before the frame ends, or its commit is
/// stranded in the queue until some later unrelated event.
/// The interaction log artifact: one append-only file under the repo/dev root's
/// orchestration artifacts so J's test drives can be read back against what the
/// panel routed and what the document became.
fn interaction_log_path() -> PathBuf {
    painter_root().join("orchestration/artifacts/interaction-log/interaction-log.txt")
}

/// Appends non-empty line batches to the interaction log artifact.
fn append_interaction_log(lines: &[String]) {
    if lines.is_empty() {
        return;
    }
    use std::io::Write;
    let path = interaction_log_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        for line in lines {
            let _ = writeln!(file, "{line}");
        }
    }
}

/// Human-readable UTC timestamp from the system clock (no chrono dependency):
/// civil-from-days over the Unix epoch seconds.
fn interaction_timestamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (now / 86400) as i64;
    let secs = now % 86400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}",
        secs / 3600,
        secs % 3600 / 60,
        secs % 60,
    )
}

#[allow(clippy::too_many_arguments)] // session-bridge seam: one fn carries the live session state; struct-izing touches the entrypoint
fn drain_layers_panel_action(
    layers_panel_state: &Rc<RefCell<LayersPanelState>>,
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
    let action = layers_panel_state.borrow_mut().take_pending_action();
    let mut log_lines = layers_panel_state.borrow_mut().take_interaction_log();
    // Pure-UI actions (selection, playhead, auto-key) never touch the document,
    // so they don't need a shape line after the apply.
    let document_mutated = !matches!(
        action,
        Some(
            LayersPanelAction::Select(_)
                | LayersPanelAction::SelectProperty(..)
                | LayersPanelAction::ToggleAutoKey
                | LayersPanelAction::SetCurrentBreath(_)
                | LayersPanelAction::TogglePlay
                | LayersPanelAction::ToggleLoop
        )
    );
    if let Some(action) = &action {
        log_lines.push(format!("[{}] apply {action:?}", interaction_timestamp()));
    }
    apply_layers_panel_action(
        action,
        shared_document,
        shared_document_paths,
        shared_action_counter,
        session_user_id,
        active_layer_id,
        selected_property_id,
        canvas,
        timeline_state,
        selection,
    );
    if document_mutated {
        // The document shape after the mutation — this is the ground truth the
        // panel mirrors back, so bar-behavior surprises read straight out of it.
        log_lines.push(format!(
            "[{}] shape raster={} move={} layer={}",
            interaction_timestamp(),
            shared_document.property_track_shape(active_layer_id, "raster"),
            shared_document.property_track_shape(active_layer_id, "move"),
            active_layer_id,
        ));
    }
    append_interaction_log(&log_lines);
}

/// Runs one hand's canvas press through the shared seam when the click
/// lands on drawable canvas: inside the paint surface, inside the world
/// bounds, off every gizmo bar, and while no typing session owns the
/// surface. Installs the text tool's typing session when the press starts
/// one; every other press begins its stroke internally.
#[allow(clippy::too_many_arguments)]
fn begin_canvas_press_if_eligible(
    pointer_strokes: &mut CanvasPointerStrokes,
    tool_state: &Rc<RefCell<ToolState>>,
    selection: &Rc<RefCell<PainterSelection>>,
    canvas: &mut Canvas,
    shared_document: &mut SharedDocumentRuntime,
    shared_document_paths: &SharedDocumentPaths,
    shared_action_counter: &mut u64,
    session_user_id: &str,
    active_layer_id: &mut String,
    hand: PaintHand,
    world: thaum_renderer_domain::WorldPoint,
    screen: CellPoint,
    paint_surface: ModuleRect,
    paint_viewport: ModuleRect,
    bounds: CanvasBounds,
    view_orientation: thaum_renderer_domain::CameraViewOrientation,
    current_breath: u32,
    bindings: &ActionBindingMap,
    text_entry: &mut Option<TextEntryState>,
    text_stroke_start: &mut Option<(Canvas, String)>,
    typing_mode: &mut TypingMode,
) {
    let position = CellPoint {
        x: world.x,
        y: world.y,
        z: world.z,
    };
    if !paint_surface.contains(screen.x, screen.y)
        || !bounds.contains(position)
        || PaintCanvasBoundsModule::is_gizmo_hit(paint_viewport, screen.x, screen.y)
        || text_entry.as_ref().is_some_and(|entry| entry.is_active())
    {
        return;
    }
    // Tool dispatch lives on the shared canvas-pointer seam; only the text
    // tool's typing session comes back here.
    if let Some(typing) = pointer_strokes.begin_press(
        &mut canvas_pointer_context(
            tool_state,
            selection,
            canvas,
            shared_document,
            shared_document_paths,
            shared_action_counter,
            session_user_id,
            active_layer_id,
        ),
        hand,
        position,
        bounds,
        view_orientation,
        current_breath,
    ) {
        *text_entry = Some(typing.entry);
        *text_stroke_start = Some(typing.stroke_start);
        typing_mode.begin(typing_reserved_inputs(bindings));
    }
}

/// Routes one hand's held-pointer frame through the shared seam when the
/// drag belongs on the canvas: chrome hits cancel the pending stroke, and
/// only in-surface, in-bounds, non-typing positions continue it. Same
/// shape as `begin_canvas_press_if_eligible`, one hand per call.
#[allow(clippy::too_many_arguments)] // session-bridge seam: one fn carries the live session state; struct-izing touches the entrypoint
fn continue_canvas_drag_if_eligible(
    pointer_strokes: &mut CanvasPointerStrokes,
    tool_state: &Rc<RefCell<ToolState>>,
    selection: &Rc<RefCell<PainterSelection>>,
    canvas: &mut Canvas,
    shared_document: &mut SharedDocumentRuntime,
    shared_document_paths: &SharedDocumentPaths,
    shared_action_counter: &mut u64,
    session_user_id: &str,
    active_layer_id: &mut String,
    hand: PaintHand,
    world: thaum_renderer_domain::WorldPoint,
    screen: CellPoint,
    paint_surface: ModuleRect,
    bounds: CanvasBounds,
    view_orientation: thaum_renderer_domain::CameraViewOrientation,
    chrome_hit: bool,
    typing_owns_input: bool,
) {
    match route_drag(
        screen,
        CellPoint {
            x: world.x,
            y: world.y,
            z: world.z,
        },
        chrome_hit,
        paint_surface,
        bounds,
        typing_owns_input,
    ) {
        DragRoute::Chrome => pointer_strokes.cancel(hand),
        DragRoute::Canvas(position) => pointer_strokes.continue_drag(
            &mut canvas_pointer_context(
                tool_state,
                selection,
                canvas,
                shared_document,
                shared_document_paths,
                shared_action_counter,
                session_user_id,
                active_layer_id,
            ),
            hand,
            position,
            bounds,
            view_orientation,
        ),
        DragRoute::Ignored => {}
    }
}

/// Finishes one release frame (either hand): broadcasts Up to every
/// module, commits both hands' pointer strokes, then drains the
/// layers-panel queue — the pointer-up dispatch itself can commit a timing
/// action or block swap, and waiting for the next click/drag frame would
/// leave that commit stranded in the queue.
#[allow(clippy::too_many_arguments)] // session-bridge seam: one fn carries the live session state; struct-izing touches the entrypoint
fn finish_canvas_release(
    modules: &mut ModuleRegistry,
    pointer_strokes: &mut CanvasPointerStrokes,
    tool_state: &Rc<RefCell<ToolState>>,
    selection: &Rc<RefCell<PainterSelection>>,
    canvas: &mut Canvas,
    shared_document: &mut SharedDocumentRuntime,
    shared_document_paths: &SharedDocumentPaths,
    shared_action_counter: &mut u64,
    session_user_id: &str,
    active_layer_id: &mut String,
    selected_property_id: &mut Option<String>,
    layers_panel_state: &Rc<RefCell<LayersPanelState>>,
    timeline_state: &Rc<RefCell<TimelineState>>,
    screen: CellPoint,
    view_orientation: thaum_renderer_domain::CameraViewOrientation,
    current_breath: u32,
) {
    // Broadcast the release to every module, not just the captured one: a
    // drag that never requested capture (or lost it) would otherwise keep
    // following the pointer through hover moves forever, since its Up
    // never arrived.
    modules.dispatch_pointer_up_all(screen.x, screen.y);
    // One committed action per stroke: the drag's staged patches become a
    // single CellPatchSet record, written once on release (one undo per
    // stroke). Hands that didn't paint are no-ops. A lasso bound closes
    // here: the enclosed cells fill through the hand's tool state (image
    // target) or select through the hand's resolved mode (selection
    // target), then share the stroke commit.
    for error in pointer_strokes.finish_pointer_stroke(
        &mut canvas_pointer_context(
            tool_state,
            selection,
            canvas,
            shared_document,
            shared_document_paths,
            shared_action_counter,
            session_user_id,
            active_layer_id,
        ),
        view_orientation,
        current_breath,
    ) {
        eprintln!("stroke commit failed (kept in memory): {error:#}");
    }
    drain_layers_panel_action(
        layers_panel_state,
        shared_document,
        shared_document_paths,
        shared_action_counter,
        session_user_id,
        active_layer_id,
        selected_property_id,
        canvas,
        timeline_state,
        selection,
    );
}

/// Who owns a held pointer during a drag frame, classified once per hand.
/// Chrome (command bar or a captured module drag) wins over the canvas; the
/// canvas only receives drags inside its screen surface, inside its world
/// bounds, while no typing session owns the input surface. Multiplayer
/// note: only the `Canvas` route reaches the shared document, and only at
/// release — a drag stages locally and commits as one authored record, so
/// an interrupted drag never strands partial state for other writers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DragRoute {
    /// A chrome drag owns the pointer: the canvas stroke yields (cancels).
    Chrome,
    /// The canvas receives the drag at this world cell.
    Canvas(CellPoint),
    /// Nobody: the pointer is outside every drag surface.
    Ignored,
}

fn route_drag(
    screen: CellPoint,
    position: CellPoint,
    chrome_hit: bool,
    paint_surface: ModuleRect,
    bounds: CanvasBounds,
    typing_owns_input: bool,
) -> DragRoute {
    if chrome_hit {
        return DragRoute::Chrome;
    }
    if paint_surface.contains(screen.x, screen.y)
        && bounds.contains(position)
        && !typing_owns_input
    {
        DragRoute::Canvas(position)
    } else {
        DragRoute::Ignored
    }
}

/// Loads the stable session identity (random user id, generated once and
/// persisted under artifacts; see `domain/painter-session/identity/`).
/// `THAUM_SESSION_USER_ID` stays as the test escape hatch: when set it
/// overrides the user_id without persisting anything.
fn session_identity() -> SessionIdentity {
    if let Ok(user_id) = env::var("THAUM_SESSION_USER_ID") {
        return SessionIdentity::generate(None).with_user_id_for_tests(user_id);
    }
    let path = painter_root().join("orchestration/artifacts/session-identity.json");
    // Cosmetic display_name seed only; ownership never derives from it.
    let os_name = env::var("USER")
        .or_else(|_| env::var("USERNAME")) // Windows
        .ok();
    SessionIdentity::load_or_create(&path, os_name.as_deref())
        .unwrap_or_else(|error| {
            eprintln!("failed to load session identity ({error}); using an ephemeral one");
            SessionIdentity::generate(None)
        })
}

fn painter_session_state_path(user_id: &str) -> PathBuf {
    painter_root()
        .join("orchestration/artifacts/user-session-state")
        .join(format!("{user_id}.json"))
}

fn painter_shared_document_paths(document_id: &str) -> SharedDocumentPaths {
    SharedDocumentPaths::new(
        painter_root()
            .join("orchestration/artifacts/shared-documents")
            .join(document_id),
    )
}

/// Runtime root every painter-owned folder hangs off. `THAUM_PAINTER_ROOT`
/// wins, then the compiled repo root when it exists on disk (in-repo dev runs
/// keep today's repo layout byte-identical), else the exe's own folder so a
/// deployed copy is self-contained and portable across machines (Windows
/// included) with no compiled-in absolute paths left behind.
fn painter_root() -> PathBuf {
    if let Ok(path) = env::var("THAUM_PAINTER_ROOT") {
        return PathBuf::from(path);
    }

    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    if repo_root.join("Cargo.toml").exists() {
        return repo_root;
    }

    env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn painter_file_root() -> PathBuf {
    resolve_painter_file_root(&painter_root())
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

/// The typed-path terminal fallback exists for jobo's Linux portal failures
/// (xdg-desktop-portal / DBus) and must never run on Windows: the native
/// dialog's cancel path routes here, and blocking on hidden console stdin
/// reads as a hard crash (observed on Windows 2026-09-06). On every other
/// platform a dialog cancel simply cancels.
fn terminal_path_fallback_enabled() -> bool {
    cfg!(target_os = "linux")
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
    if !terminal_path_fallback_enabled() {
        return None;
    }
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

/// Quick-save target for a document with no file path yet: the slugified
/// document title under the default painter-files root, bumped with `-NN`
/// when that folder already holds a document (never silently overwrites).
/// Regular save must not depend on the native dialog stack at all: portal
/// backends can fail instantly on some Linux sessions (observed on jobo),
/// and a first save should never need more than one keystroke.
fn quick_save_document_root(file_root: &Path, title: &str) -> PathBuf {
    let mut stem = slugify_file_stem(title);
    if stem.is_empty() {
        stem = "untitled-document".to_string();
    }
    let mut candidate = file_root.join(&stem);
    let mut suffix = 2;
    while candidate.join("document.json").exists() {
        candidate = file_root.join(format!("{stem}-{suffix}"));
        suffix += 1;
        if suffix > 99 {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            candidate = file_root.join(format!("{stem}-{stamp}"));
            break;
        }
    }
    candidate
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
        .set_file_name(format!("{suggested}.json"))
        .save_file()
        .map(|path| save_as_root_from_dialog_path(&path));
    if let Some(path) = selected {
        return Some(path);
    }
    log_missing_dialog_selection("save", file_root);
    if !terminal_path_fallback_enabled() {
        return None;
    }
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
    let mut buttons: Vec<CommandBarButton> = [
        ("paint_canvas_bounds", "DRAWING SPACE"),
        ("painter_toolbox", "TOOLS"),
        ("painter_color_picker", "COLOR PICKER"),
        ("painter_color_block", "COLOR BLOCK"),
        ("painter_material_picker", "MATERIALS"),
        ("painter_layers_panel", "LAYERS"),
        ("painter_graphic_picker", "GRAPHICS"),
        ("painter_hand_settings", "PROPS"),
        ("painter_camera_perspective", "PERSPECTIVE"),
        ("painter_controls_panel", "CONTROLS"),
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
    .collect();
    buttons.push(CommandBarButton::new("layout:reset", "RESET LAYOUT"));
    buttons
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
    if button_id == "layout:reset" {
        // Reset layout: every live module returns to the shared default
        // layout — default rect, silence (seamless) off, open. Camera is
        // intentionally untouched.
        modules.apply_persisted_ui_state(&painter_modules::default_module_layout());
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
                let (next_paths, next_document) = match load_document_from_root(&next_root) {
                    Ok(loaded) => loaded,
                    Err(error) => {
                        // Unsupported-file gate: a saved file whose kind or schema
                        // version this build cannot read is rejected cleanly — the
                        // file does not open, no in-memory state changes, the
                        // session keeps running. Only this error class maps to a
                        // soft rejection; every other failure still propagates.
                        if error.downcast_ref::<UnsupportedFileError>().is_some() {
                            let message = format!("document not opened: {error:#}");
                            thaum_painter_domain::debug_log::error("file", &message);
                            eprintln!("{message}");
                            return Ok(());
                        }
                        return Err(error);
                    }
                };
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
                // Quick save: default-name folder under painter-files, no
                // dialog. Native save dialogs stay on SAVE AS (and OPEN), so
                // the common first save never touches the portal/GTK dialog
                // stack that fails on some Linux sessions.
                let next_root =
                    quick_save_document_root(&file_root, &shared_document.document.title);
                thaum_painter_domain::debug_log::info(
                    "file",
                    &format!("quick save target: {}", next_root.display()),
                );
                *shared_document_paths = SharedDocumentPaths::new(next_root.clone());
                *current_document_root = Some(next_root);
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
    use thaum_painter_domain::selection_stroke::interpolate_cell_path;
    use thaum_painter_domain::text_entry::TextEntryState;

    use thaum_renderer_domain::{camera_view_orientation_for_camera, CameraRoll, CameraSwing};

    #[test]
    fn arrow_keys_route_into_the_session_and_nudge_the_cursor() {
        // The app's exact key path for one owned press: reserved set from the
        // live bindings, route the raw label, translate the physical key,
        // feed the session. All four arrows must nudge the cursor one cell
        // in the screen direction the key names.
        let bindings = thaum_painter_domain::tai::painter_bindings();
        let mut typing_mode = TypingMode::default();
        typing_mode.begin(typing_reserved_inputs(&bindings));
        let orientation =
            camera_view_orientation_for_camera(CameraSwing::PosZ, CameraRoll::Deg0);
        let brush = thaum_painter_domain::brush::PaintedCell {
            graphic: thaum_renderer_domain::CellGraphic::Glyph('?'),
            color: thaum_painter_domain::paint_color::PaintColor::flat_rgb(255, 255, 255),
            weight_index: 1,
        };
        let mut entry = TextEntryState::begin(
            CellPoint { x: 5, y: 5, z: 0 },
            orientation,
            thaum_painter_domain::text::DEFAULT_TEXT_LAYOUT_OPTIONS,
            false,
            brush,
        );
        let steps: &[(KeyCode, CellPoint)] = &[
            (KeyCode::ArrowRight, CellPoint { x: 6, y: 5, z: 0 }),
            (KeyCode::ArrowDown, CellPoint { x: 6, y: 4, z: 0 }),
            (KeyCode::ArrowLeft, CellPoint { x: 5, y: 4, z: 0 }),
            (KeyCode::ArrowUp, CellPoint { x: 5, y: 5, z: 0 }),
        ];
        for (key, want) in steps {
            let route = raw_key_label(*key)
                .map(|label| typing_mode.route(&RawInput::Key(label)));
            assert_eq!(route, Some(TypingRoute::Owned), "{key:?} must be owned");
            let entry_key = text_entry_key_for_key(*key, false)
                .unwrap_or_else(|| panic!("{key:?} must translate"));
            entry.handle_key(entry_key);
            assert_eq!(entry.cursor_point(), *want, "{key:?} must nudge the cursor");
        }
    }

    fn surface_rect() -> ModuleRect {
        ModuleRect {
            x0: 0,
            y0: 0,
            x1: 100,
            y1: 100,
        }
    }

    fn canvas_bounds() -> CanvasBounds {
        CanvasBounds {
            x0: -16,
            y0: -16,
            x1: 16,
            y1: 16,
            z: 0,
            plane_axis: Default::default(),
        }
    }

    fn world_cell(x: i32, y: i32) -> CellPoint {
        CellPoint { x, y, z: 0 }
    }

    #[test]
    fn chrome_wins_over_the_canvas_even_inside_the_paint_surface() {
        let route = route_drag(
            world_cell(50, 50),
            world_cell(1, 1),
            true,
            surface_rect(),
            canvas_bounds(),
            false,
        );
        assert_eq!(route, DragRoute::Chrome);
    }

    #[test]
    fn canvas_route_requires_surface_bounds_and_no_typing() {
        let inside = (world_cell(50, 50), world_cell(1, 1));
        assert_eq!(
            route_drag(inside.0, inside.1, false, surface_rect(), canvas_bounds(), false),
            DragRoute::Canvas(inside.1)
        );
        // Outside the screen surface: ignored even if the world cell is valid.
        assert_eq!(
            route_drag(
                world_cell(150, 50),
                world_cell(1, 1),
                false,
                surface_rect(),
                canvas_bounds(),
                false
            ),
            DragRoute::Ignored
        );
        // Typing owns the input surface: the canvas receives nothing.
        assert_eq!(
            route_drag(inside.0, inside.1, false, surface_rect(), canvas_bounds(), true),
            DragRoute::Ignored
        );
    }

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
    fn quick_save_uses_slugified_title_under_the_file_root() {
        let file_root = std::env::temp_dir().join(format!(
            "painter-quick-save-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&file_root).unwrap();

        let root = quick_save_document_root(&file_root, "My Cool Sketch");
        assert_eq!(root, file_root.join("my-cool-sketch"));

        // No document inside yet: the same title reuses the same folder.
        assert_eq!(quick_save_document_root(&file_root, "My Cool Sketch"), root);

        // A document already in that folder: the next quick save bumps -02.
        std::fs::create_dir_all(root.join("document.json").parent().unwrap()).unwrap();
        std::fs::write(root.join("document.json"), "{}").unwrap();
        assert_eq!(
            quick_save_document_root(&file_root, "My Cool Sketch"),
            file_root.join("my-cool-sketch-2")
        );

        // Empty/blank titles fall back to the untitled default.
        assert_eq!(
            quick_save_document_root(&file_root, "   "),
            file_root.join("untitled-document")
        );

        let _ = std::fs::remove_dir_all(&file_root);
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
    fn text_entry_keys_pick_up_the_shifted_glyph() {
        assert_eq!(
            text_entry_key_for_key(KeyCode::KeyC, false),
            Some(TextEntryKey::Char('c'))
        );
        assert_eq!(
            text_entry_key_for_key(KeyCode::KeyC, true),
            Some(TextEntryKey::Char('C'))
        );
        assert_eq!(
            text_entry_key_for_key(KeyCode::Digit6, true),
            Some(TextEntryKey::Char('^'))
        );
        assert_eq!(
            text_entry_key_for_key(KeyCode::Digit7, true),
            Some(TextEntryKey::Char('&'))
        );
        assert_eq!(
            text_entry_key_for_key(KeyCode::Digit1, true),
            Some(TextEntryKey::Char('!'))
        );
        assert_eq!(
            text_entry_key_for_key(KeyCode::Minus, true),
            Some(TextEntryKey::Char('_'))
        );
        assert_eq!(
            text_entry_key_for_key(KeyCode::Minus, false),
            Some(TextEntryKey::Char('-'))
        );
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
/// `shift_held` picks the shifted glyph: uppercase for letters, the US-layout
/// symbol-row pair for digits and symbols (`1`→`!`, `-`→`_`, ...).
fn text_entry_key_for_key(key: KeyCode, shift_held: bool) -> Option<TextEntryKey> {
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
        KeyCode::Home => Some(TextEntryKey::Home),
        KeyCode::End => Some(TextEntryKey::End),
        _ => raw_key_label(key).and_then(|label| {
            let mut chars = label.chars();
            let ch = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            Some(TextEntryKey::Char(shifted_char(ch, shift_held)))
        }),
    }
}

/// US-layout shifted glyph for one printable base character: uppercase for
/// letters, the symbol-row pair otherwise. Only keys `raw_key_label` maps to
/// single chars reach this (letters, digits, `-`, `=`); the rest is future
/// proofing for wider key tables.
fn shifted_char(ch: char, shift_held: bool) -> char {
    if !shift_held {
        return ch.to_ascii_lowercase();
    }
    match ch {
        'a'..='z' => ch.to_ascii_uppercase(),
        '1' => '!',
        '2' => '@',
        '3' => '#',
        '4' => '$',
        '5' => '%',
        '6' => '^',
        '7' => '&',
        '8' => '*',
        '9' => '(',
        '0' => ')',
        '-' => '_',
        '=' => '+',
        other => other,
    }
}

/// Feeds one translated key into an open in-place number-field edit: digits
/// and `-` build the buffer, Backspace/Delete erase, Enter commits the
/// clamped value into the tool state, Escape cancels. Every other key is
/// consumed silently — the edit owns the surface while it is open. Caller
/// guards on an open edit.
fn handle_number_field_edit_key(
    entry_key: TextEntryKey,
    number_edit: &Rc<RefCell<Option<NumberFieldEdit>>>,
    tool_state: &RefCell<ToolState>,
) {
    match entry_key {
        TextEntryKey::Enter => {
            if let Some(edit) = number_edit.borrow_mut().take() {
                if let Some(value) = edit.commit(-9, 9) {
                    let mut tool_state = tool_state.borrow_mut();
                    if edit.row_id == "text_enter_step" {
                        tool_state.set_text_enter_step_axis(edit.field.min(2), value);
                    } else {
                        tool_state.set_text_char_step_axis(edit.field.min(2), value);
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

/// Bridges a `THAUM3D:` payload from the OS clipboard into the session
/// user's own buffer (cross-session copies ride the OS clipboard). OS
/// transport failures are silently ignored so the own buffer always works;
/// foreign text is rejected by the codec.
fn import_os_clipboard_into_own_buffer(
    user_clipboards: &mut thaum_painter_domain::clipboard::UserClipboards,
    user_id: &str,
) {
    let mut os_clipboard = match arboard::Clipboard::new() {
        Ok(value) => value,
        Err(err) => {
            eprintln!("[import-debug] clipboard open failed: {err}");
            return;
        }
    };
    let text = match os_clipboard.get_text() {
        Ok(value) => value,
        Err(err) => {
            eprintln!("[import-debug] clipboard read failed: {err}");
            return;
        }
    };
    eprintln!("[import-debug] read {} bytes: {text:.60}", text.len());
    match thaum_painter_domain::clipboard::decode_os_clipboard(&text) {
        Some(data) => {
            eprintln!("[import-debug] decoded {} cells", data.cells.len());
            user_clipboards.copy_for_user(user_id, data);
        }
        None => eprintln!("[import-debug] decode returned None"),
    }
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
    "painter_clipboard_copy",
    "painter_select_stamp",
    "painter_select_move",
    "painter_play_pause",
];

/// Presentation rows for the controls panel: the declared action list with
/// human labels. Drift tests keep these names locked to the registry.
pub(crate) fn painter_control_rows() -> Vec<ControlActionRow> {
    let rows = [
        ("tools", "Select Pencil", "painter_select_pencil"),
        ("tools", "Select Bucket", "painter_select_bucket"),
        ("tools", "Select Lasso", "painter_select_lasso"),
        ("tools", "Select Stamp", "painter_select_stamp"),
        ("tools", "Select Move", "painter_select_move"),
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
        ("clipboard", "Copy World", "painter_clipboard_copy"),
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
    // Machine-truth workaround (jobo): the rfd GTK save dialog enumerates trash
    // through gvfsd-trash, and that backend chokes on this machine (journal:
    // "GFileInfo created without standard::name" bursts) — the dialog freezes
    // and then the whole program dies. Forcing GIO's plain unix volume monitor
    // skips the gvfs trash backend entirely; it must be set before any GTK/GIO
    // call initializes. Policy twin lives in the operator workshop contract's
    // machine-hygiene note (never touch the desktop trash from agent flows).
    if std::env::var_os("GIO_USE_VOLUME_MONITOR").is_none() {
        std::env::set_var("GIO_USE_VOLUME_MONITOR", "unix");
    }
    // Per-run log folder: configures the shared debug-log sink into
    // context/debug-logging/<UTC-stamp>/run.log, culls older folders to the
    // newest five, and installs the panic hook. `THAUM_DEBUG` raises the floor.
    let run_log_dir = run_log::boot(&painter_root());
    let mut config = BootConfig::default();
    config.asset_root = resolve_asset_root();
    config.window.title = "thaum-painter".to_string();
    config.window.performance_log_path =
        run_log_dir.as_ref().map(|dir| dir.join("perf.jsonl"));

    let session_identity = session_identity();
    let session_user_id = session_identity.user_id.clone();
    let session_state_path = painter_session_state_path(&session_user_id);
    let persisted_session = load_painter_user_session_state(&session_state_path)?;
    // Boot to a blank unsaved document: the user opens or saves explicitly. Restoring
    // the last document from session state once pinned boot to a legacy file that
    // would break silently on schema changes.
    let mut shared_document = new_unsaved_document();
    let mut shared_document_paths = painter_shared_document_paths(&shared_document.document.document_id);

    // Multiplayer boot (env-driven v1; the session-module UI is the next
    // slice): THAUM_SESSION_HOST[=port] hosts a LAN session, or
    // THAUM_SESSION_JOIN=ip[:port] joins one. Joining replaces the blank boot
    // document with the host's snapshot (Figma's fresh-copy model); every
    // record that reaches this process afterwards is applied in host order.
    let mut session_net: Option<thaum_painter_workers::SessionNet> = None;
    match thaum_painter_workers::session_net_boot_from_env() {
        thaum_painter_workers::SessionNetBoot::Host(port) => {
            // Snapshot source freezes the boot document: the host log starts
            // empty at boot, so joiners rebuild exactly via snapshot + full
            // record replay. (Structure edits that still bypass the record
            // log are not replayed — closed by the structure-record slice.)
            let boot_document = shared_document.document.clone();
            let net = thaum_painter_workers::SessionNet::host(
                Box::new(move || boot_document.clone()),
                thaum_painter_workers::session_user_from_identity(&session_identity),
                port,
            );
            match net {
                Ok(net) => {
                    eprintln!("session hosting on port {port} as {}", net.user_id());
                    session_net = Some(net);
                }
                Err(error) => eprintln!("failed to host session on port {port}: {error}"),
            }
        }
        thaum_painter_workers::SessionNetBoot::Join(raw) => {
            let address = thaum_painter_workers::join_address(&raw);
            let net = thaum_painter_workers::SessionNet::join(
                &address,
                thaum_painter_workers::session_user_from_identity(&session_identity),
            );
            match net {
                Ok((net, snapshot)) => {
                    shared_document = SharedDocumentRuntime::new(snapshot);
                    shared_document_paths = painter_shared_document_paths(
                        &shared_document.document.document_id,
                    );
                    eprintln!("joined session at {address} as {}", net.user_id());
                    session_net = Some(net);
                }
                Err(error) => eprintln!("failed to join session at {address}: {error}"),
            }
        }
        thaum_painter_workers::SessionNetBoot::None => {}
    }
    let mut net_published: usize = 0;
    let mut current_document_root: Option<PathBuf> = None;

    let mut state = boot_renderer(config)?;
    // The shape-fade graph over the loaded typeface (J 2026-09-07): built
    // once at boot at the fade's canonical weight, then moved into the frame
    // loop and injected into the render path so the raster interpolate mode
    // walks matched cells' graphics through the gradient tour instead of the
    // halfway cutoff. `None` (no loadable font set) keeps the cutoff.
    let shape_fade: Option<ShapeFade> =
        GlyphFontSet::load_from_asset_root(&state.config.asset_root)
            .ok()
            .map(|font_set| ShapeFade::build(&FontSetTiles { font_set: &font_set }));
    if let Some(session) = &persisted_session {
        session.renderer.camera.apply_to_runtime(&mut state.camera);
    } else {
        // Fresh boot, nothing assigned: start at the painter's tuned default
        // camera. Module default rects are HUD-space and tuned against it.
        state.camera = painter_default_camera();
    }
    // The camera-perspective panel edits these shared cells; the frame sync
    // copies them into the live camera so projection picks them up every
    // frame. Parallax's pointer offset is host-fed from the frame input.
    let camera_perspective_profile = Rc::new(RefCell::new(state.camera.perspective));
    let camera_parallax_profile = Rc::new(RefCell::new(state.camera.parallax));
    // The perspective panel's depth row scrolls the render depth through this
    // link: the frame loop drains its pending wheel steps into the camera and
    // publishes the live focus depth back for the row's display.
    let camera_depth_link = CameraDepthLink::new();
    // The perspective panel's layers row steps the rendered-layer count
    // through this link: the frame loop drains its pending wheel steps into
    // the camera's visible_plane_radius (clamped) and publishes the live
    // count back for the row's display.
    let camera_layers_link = CameraLayersLink::new();

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

    // Shared in-place number-field edit state for the hand-settings panel:
    // the module opens it on a field click, the typing seam routes the keys,
    // and Enter commits the value through the tool state.
    let number_edit: Rc<RefCell<Option<NumberFieldEdit>>> = Rc::new(RefCell::new(None));
    let layers_panel_state = Rc::new(RefCell::new(LayersPanelState::default()));
    let mut modules = painter_modules::build_painter_modules(
        &tool_state,
        &selection,
        &number_edit,
        &layers_panel_state,
        &paint_canvas_viewport,
        &drawing_space_wheel_mode,
        &controls_profile,
        &effective_painter_bindings,
        &painter_bindings,
        &ui_palette,
        &camera_perspective_profile,
        &camera_parallax_profile,
        &camera_depth_link,
        &camera_layers_link,
    );

    if let Some(session) = &persisted_session {
        session.renderer.palette.apply_to_runtime(&ui_palette);
        modules.apply_persisted_ui_state(&session.renderer.modules);
    }
    sync_renderer_background_from_ui_palette(&mut state, &ui_palette);

    let mut left_pointer_was_down = false;
    let mut right_pointer_was_down = false;
    // Per-hand canvas pointer strokes: press dispatch, drag continuation, and
    // release commit live on the shared canvas-pointer seam; the entrypoint
    // keeps only screen-space hit testing and typing-session ownership.
    let mut pointer_strokes = CanvasPointerStrokes::new();
    // Live text-entry session: begun by a Text-tool canvas click, ended by
    // Escape/exit. Pending keystrokes commit as one 'Type Text' record per
    // segment (Enter starts a new segment), the old commit granularity.
    let mut text_entry: Option<TextEntryState> = None;
    let mut text_stroke_start: Option<(Canvas, String)> = None;
    // Per-user clipboard buffers (own-buffer paste, source-of-truth from J);
    // cross-session copies bridge through the OS clipboard into the own buffer.
    let mut user_clipboards = thaum_painter_domain::clipboard::UserClipboards::new();
    // Renderer-owned input-focus gate for typing sessions; declared reserved
    // inputs (camera/depth bindings) stay live, everything else is focused
    // into the session or suppressed.
    let mut typing_mode = TypingMode::default();
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
    // Tooltip dwell state (shared tooltip seam): hotspot cards for gizmos
    // and other interactive pieces. The clock measures irregular frame gaps
    // for the 400ms dwell; needs_frame keeps a demand-driven loop ticking
    // while a dwell is running or a card just appeared/disappeared.
    let mut tooltip_state = TooltipState::new();
    let mut tooltip_clock = Instant::now();
    let mut tooltip_needs_frame = false;
    // Demand-driven frame state: what the previous frame already consumed.
    // When nothing changed, the closure leaves `state.composition` untouched
    // and boot's scene cache reuses the built scene without any group
    // rebuild, projection, or upload.
    let mut last_frame_cursor: Option<[f32; 2]> = None;
    let mut last_pointer_down = false;
    let mut last_right_pointer_down = false;
    let mut session_dirty = true;
    // Producer-maintained composition generation: bumped on every full-path
    // rebuild so boot's scene cache identifies the composition in O(1)
    // instead of hashing its cells every frame.
    let mut composition_revision: u64 = 0;

    run_renderer_window_with_state_frame_provider(state, move |state, frame| {
        // Demand-driven frame gate: input, animation timers, or a pending
        // session save make the frame dirty; otherwise the last composition
        // stays in place and nothing rebuilds.
        let input_dirty = !frame.input.just_pressed_keys.is_empty()
            || !frame.input.pressed_keys.is_empty()
            || frame.input.just_clicked.is_some()
            || frame.input.just_right_clicked.is_some()
            || frame.input.wheel_delta_x != 0.0
            || frame.input.wheel_delta_y != 0.0
            || frame.input.cursor_position != last_frame_cursor
            || frame.input.pointer_down != last_pointer_down
            || frame.input.right_pointer_down != last_right_pointer_down;
        let playback_due = timeline_state.borrow().playing
            && last_playback_step.elapsed() >= PLAYBACK_BREATH_INTERVAL;
        let blink_due = typing_mode.is_active()
            && cursor_blink_at.elapsed() >= CURSOR_BLINK_INTERVAL;
        // Multiplayer frame seam: publish this frame's new local records
        // (strokes, undo/redo, selections — everything that appended to the
        // runtime's action log), then pull foreign ones in host order. Runs
        // before the demand gate: remote edits make the frame dirty alone.
        let mut network_applied = 0usize;
        if let Some(net) = session_net.as_mut() {
            if net.is_connected() {
                let total = shared_document.actions.len();
                while net_published < total {
                    let record = shared_document.actions[net_published].clone();
                    net_published += 1;
                    if let Err(error) = net.publish(record) {
                        eprintln!("session publish failed: {error}");
                        break;
                    }
                }
                match net.sync(&mut shared_document) {
                    Ok(applied) => network_applied = applied,
                    Err(error) => eprintln!("session sync failed: {error}"),
                }
            }
        }
        if !input_dirty
            && !playback_due
            && !blink_due
            && !session_dirty
            && !tooltip_needs_frame
            && network_applied == 0
        {
            return Ok(());
        }
        if network_applied > 0 {
            // Foreign records changed document truth: resync the live mirrors
            // (canvas, active layer, selection cache) the same way the
            // snapshot-conflict recovery path does.
            let current_breath = timeline_state.borrow().current_breath;
            active_layer_id =
                resolved_active_layer_id(&shared_document, Some(&active_layer_id));
            sync_canvas_from_active_layer(
                &shared_document,
                &active_layer_id,
                current_breath,
                &mut canvas,
            );
            selection.borrow_mut().replace_points(
                shared_document.selection_points(DEFAULT_SELECTION_CHANNEL_ID),
            );
        }
        last_frame_cursor = frame.input.cursor_position;
        last_pointer_down = frame.input.pointer_down;
        last_right_pointer_down = frame.input.right_pointer_down;

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
            // The seam only consumes pan actions; everything else a held key
            // might bind is inert here.
            let held_actions = painter_key_actions(&effective_painter_bindings.borrow(), *key);
            for action in &held_actions {
                apply_painter_pan_action(&mut state.camera, &action.0, hovering_canvas_bounds);
            }
        }
        for key in &frame.input.just_pressed_keys {
            // TEMP arrow-key diagnostics: which stage does each press reach?
            eprintln!("[input-debug] just_pressed {key:?}");
            // Shift state comes from the held-key set (winit reports the
            // modifier itself as a pressed physical key), so shifted glyphs
            // reach both the text session and open number-field edits.
            let shift_held = frame.input.pressed_keys.contains(&KeyCode::ShiftLeft)
                || frame.input.pressed_keys.contains(&KeyCode::ShiftRight);
            // While typing mode owns the input surface: session keys route
            // into the typing session, reserved camera/depth inputs fall
            // through to live dispatch, and everything else — including
            // module key capture — is suppressed.
            if typing_mode.is_active() {
                let route = raw_key_label(*key)
                    .map(|label| typing_mode.route(&RawInput::Key(label)))
                    .unwrap_or(TypingRoute::Suppressed);
                match route {
                    TypingRoute::Suppressed => {
                        eprintln!("[input-debug]   {key:?} -> suppressed");
                        continue;
                    }
                    TypingRoute::Reserved => {
                        eprintln!("[input-debug]   {key:?} -> reserved");
                    }
                    TypingRoute::Owned => {
                        let Some(entry_key) = text_entry_key_for_key(*key, shift_held) else {
                            eprintln!("[input-debug]   {key:?} -> owned but untranslated");
                            continue;
                        };
                        eprintln!("[input-debug]   {key:?} -> owned {entry_key:?}");
                        // An open number-field edit consumes the keys first.
                        if number_edit.borrow().is_some() {
                            handle_number_field_edit_key(entry_key, &number_edit, &tool_state);
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
                    TextEntryOutcome::Idle | TextEntryOutcome::Ignored => {
                        if matches!(
                            entry_key,
                            TextEntryKey::ArrowLeft
                                | TextEntryKey::ArrowRight
                                | TextEntryKey::ArrowUp
                                | TextEntryKey::ArrowDown
                        ) {
                            eprintln!(
                                "[input-debug]   cursor now {:?}",
                                text_entry.as_ref().map(|e| e.cursor_point())
                            );
                        }
                    }
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
                    // Equipping the stamp imports a THAUM3D OS-clipboard
                    // payload into the own buffer (cross-session copies ride
                    // the OS clipboard); the stamp then works from the own
                    // buffer only.
                    if action.0.as_str() == "painter_select_stamp" {
                        import_os_clipboard_into_own_buffer(
                            &mut user_clipboards,
                            &session_user_id,
                        );
                    }
                    continue;
                }
                // Camera actions (pan/swing/roll/depth/zoom) live on the
                // domain seam; the hover split of the pan route is owned there.
                if apply_painter_camera_action(
                    &mut state.camera,
                    &action.0,
                    *paint_canvas_viewport.borrow(),
                    hovering_canvas_bounds,
                ) {
                    continue;
                }
                // Selection mode/shape actions live on the domain seam; shape
                // changes commit their plane points to the shared channel there.
                if apply_painter_selection_action(
                    &action.0,
                    &selection,
                    &mut shared_document,
                    &shared_document_paths,
                    &mut shared_action_counter,
                    &mut active_layer_id,
                    timeline_state.borrow().current_breath,
                    &mut canvas,
                )? {
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
                    "painter_clipboard_copy" => {
                        // Copy the selection as a 3D world copy into the own
                        // buffer, and mirror it onto the OS clipboard so a copy
                        // can emerge in another session.
                        if let Some(data) = thaum_painter_domain::clipboard::copy_from_canvas(
                            &canvas,
                            &selection.borrow(),
                        ) {
                            if let Ok(mut os_clipboard) = arboard::Clipboard::new() {
                                let _ = os_clipboard.set_text(
                                    thaum_painter_domain::clipboard::encode_os_clipboard(&data),
                                );
                            }
                            user_clipboards.copy_for_user(&session_user_id, data);
                        }
                    }
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

        // Camera-perspective panel edits land here: the shared profile cells
        // are the live source, so projection always sees the panel's values.
        // The parallax pointer offset is fed first from the raw frame cursor
        // (clip space, -1..1, y up) so mouse movement anywhere on the screen
        // drives the drift while the panel keeps owning toggle and strength.
        camera_parallax_profile.borrow_mut().offset = frame
            .input
            .cursor_position
            .map(|[x, y]| [x.clamp(-1.0, 1.0), y.clamp(-1.0, 1.0)])
            .unwrap_or([0.0, 0.0]);
        state.camera.perspective = *camera_perspective_profile.borrow();
        state.camera.parallax = *camera_parallax_profile.borrow();
        // Depth-row wheel steps land here: drain them into the focus depth,
        // then publish the live depth back so the row shows camera truth.
        let depth_steps = camera_depth_link.drain_pending();
        if depth_steps != 0 {
            state.camera.pan_focus_depth(depth_steps);
            session_dirty = true;
        }
        camera_depth_link.set_current(state.camera.focus_depth());
        // Layers-row wheel steps land here: drain them into the camera's
        // visible_plane_radius (never below the focus plane alone, never
        // above the panel's cap), then publish the live count back.
        let layer_steps = camera_layers_link.drain_pending();
        if layer_steps != 0 {
            state.camera.visible_plane_radius = (state.camera.visible_plane_radius + layer_steps)
                .clamp(0, MAX_VISIBLE_PLANE_RADIUS);
            session_dirty = true;
        }
        camera_layers_link.set_current(state.camera.visible_plane_radius);
        let camera = state.camera;
        let view_orientation = camera_view_orientation_for_camera(camera.swing, camera.roll);
        let cell_clip_size = cell_clip_size_for_state(state, frame.surface_size);
        // Drawing accounts for the active layer's move (J 2026-09-07): the
        // cursor maps into DOCUMENT space — the active move offset is
        // subtracted here, once, so every tool (strokes, lasso, selection,
        // stamp, text, bounds eligibility) aims at the cell that renders back
        // under the cursor once the render path re-applies the move shift.
        // The offset is constant across a frame, so move-drag deltas are
        // unaffected — only the aim point is corrected.
        let active_move_offset = shared_document.move_offset_for_layer(
            &active_layer_id,
            timeline_state.borrow().current_breath,
        );
        let to_world = move |surface_units: [f32; 2]| {
            let world =
                remap_surface_units_to_active_plane_world(camera, surface_units, cell_clip_size);
            thaum_renderer_domain::WorldPoint {
                x: world.x - active_move_offset.x,
                y: world.y - active_move_offset.y,
                z: world.z - active_move_offset.z,
            }
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
                let hit = modules.dispatch_pointer_event_at(
                        screen.x,
                        screen.y,
                        ModulePointerEvent::Click {
                            x: screen.x,
                            y: screen.y,
                            button: ModulePointerButton::Left,
                        },
                    );
                eprintln!(
                    "[click-debug] surface=({:?}) screen=({},{}) hit={:?}",
                    click, screen.x, screen.y, hit
                );
                hit
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
            drain_layers_panel_action(
                &layers_panel_state,
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
                pointer_strokes.cancel(PaintHand::Left);
            } else {
                begin_canvas_press_if_eligible(
                    &mut pointer_strokes,
                    &tool_state,
                    &selection,
                    &mut canvas,
                    &mut shared_document,
                    &shared_document_paths,
                    &mut shared_action_counter,
                    &session_user_id,
                    &mut active_layer_id,
                    PaintHand::Left,
                    world,
                    screen,
                    paint_surface,
                    paint_viewport,
                    *paint_canvas_bounds.borrow(),
                    view_orientation,
                    timeline_state.borrow().current_breath,
                    &effective_painter_bindings.borrow(),
                    &mut text_entry,
                    &mut text_stroke_start,
                    &mut typing_mode,
                );
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
            drain_layers_panel_action(
                &layers_panel_state,
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
                pointer_strokes.cancel(PaintHand::Right);
            } else {
                begin_canvas_press_if_eligible(
                    &mut pointer_strokes,
                    &tool_state,
                    &selection,
                    &mut canvas,
                    &mut shared_document,
                    &shared_document_paths,
                    &mut shared_action_counter,
                    &session_user_id,
                    &mut active_layer_id,
                    PaintHand::Right,
                    world,
                    screen,
                    paint_surface,
                    paint_viewport,
                    *paint_canvas_bounds.borrow(),
                    view_orientation,
                    timeline_state.borrow().current_breath,
                    &effective_painter_bindings.borrow(),
                    &mut text_entry,
                    &mut text_stroke_start,
                    &mut typing_mode,
                );
            }
            // A number-field click just began an in-place edit; typing mode
            // owns the keyboard until a click or Escape ends the session.
            if !typing_mode.is_active() && number_edit.borrow().is_some() {
                typing_mode.begin(typing_reserved_inputs(
                    &effective_painter_bindings.borrow(),
                ));
            }
        } else if frame.input.pointer_down || frame.input.right_pointer_down {
            // One held-pointer path per hand: both buttons behave identically
            // apart from which hand's tool/stroke they carry, and left wins
            // when both are held (the original else-if ordering).
            let hand = if frame.input.pointer_down {
                PaintHand::Left
            } else {
                PaintHand::Right
            };
            if let Some(cursor) = frame.input.cursor_position {
                let world = to_world(cursor);
                let screen = to_screen(cursor);
                modules.dispatch_captured_pointer_move(screen.x, screen.y);
                // A captured pointer move can enqueue a layers-panel action,
                // and waiting for a click/release to apply it would strand
                // the commit — drain on every drag frame.
                drain_layers_panel_action(
                    &layers_panel_state,
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
                continue_canvas_drag_if_eligible(
                    &mut pointer_strokes,
                    &tool_state,
                    &selection,
                    &mut canvas,
                    &mut shared_document,
                    &shared_document_paths,
                    &mut shared_action_counter,
                    &session_user_id,
                    &mut active_layer_id,
                    hand,
                    world,
                    screen,
                    paint_surface,
                    *paint_canvas_bounds.borrow(),
                    view_orientation,
                    command_bar.contains(screen.x, screen.y) || modules.is_pointer_captured(),
                    text_entry.as_ref().is_some_and(|entry| entry.is_active()),
                );
            }
        }

        if pointer_strokes.has_selection_stroke()
            && !frame.input.pointer_down
            && !frame.input.right_pointer_down
        {
            pointer_strokes.finish_selection_stroke(
                &mut canvas_pointer_context(
                    &tool_state,
                    &selection,
                    &mut canvas,
                    &mut shared_document,
                    &shared_document_paths,
                    &mut shared_action_counter,
                    &session_user_id,
                    &mut active_layer_id,
                ),
                timeline_state.borrow().current_breath,
            );
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
            pointer_strokes.clear_drag_position(PaintHand::Left);
        }
        if !frame.input.right_pointer_down {
            pointer_strokes.clear_drag_position(PaintHand::Right);
        }

        // A release this frame (either hand): broadcast Up to every module,
        // then commit the canvas strokes. The pointer-up dispatch is where a
        // layers-panel drag commits its single timing action (or block swap),
        // so the pending action must be applied right here — waiting for the
        // next click/drag frame would leave the commit stranded in the queue.
        let left_released = left_pointer_was_down && !frame.input.pointer_down;
        let right_released = right_pointer_was_down && !frame.input.right_pointer_down;
        if left_released || right_released {
            let screen = frame
                .input
                .cursor_position
                .map(to_screen)
                .unwrap_or_else(|| to_screen([0.0, 0.0]));
            // The release folds one final move frame first: a focus-depth
            // scroll or view rotation after the last drag frame must still
            // land. Same eligibility as drag frames — only on-canvas release
            // points fold, so an off-canvas release adds no motion.
            let release_world = frame.input.cursor_position.map(to_world).filter(|world| {
                paint_canvas_bounds.borrow().contains(CellPoint {
                    x: world.x,
                    y: world.y,
                    z: world.z,
                })
            });
            if let Some(world) = release_world {
                let position = CellPoint {
                    x: world.x,
                    y: world.y,
                    z: world.z,
                };
                if left_released {
                    pointer_strokes.fold_move_release(PaintHand::Left, position, view_orientation);
                }
                if right_released {
                    pointer_strokes.fold_move_release(PaintHand::Right, position, view_orientation);
                }
            }
            // The breath reads into a binding first: a `timeline_state.borrow()`
            // temporary in this call's argument list would live through the whole
            // `finish_canvas_release` call, and the layers-panel drain inside it
            // borrow_muts the same state for queued SetCurrentBreath/Toggle
            // actions — the RefCell double-borrow behind the scrub-then-release
            // timeline crash.
            let current_breath = timeline_state.borrow().current_breath;
            finish_canvas_release(
                &mut modules,
                &mut pointer_strokes,
                &tool_state,
                &selection,
                &mut canvas,
                &mut shared_document,
                &shared_document_paths,
                &mut shared_action_counter,
                &session_user_id,
                &mut active_layer_id,
                &mut selected_property_id,
                &layers_panel_state,
                &timeline_state,
                screen,
                view_orientation,
                current_breath,
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
        let hovered_id = modules
            .update_hover_at(hover_point)
            .map(|id| id.to_string());
        // Cloned out so the closure below can borrow `modules` immutably
        // while `hovered_id` no longer ties to it.
        let hovered_id = hovered_id.clone();

        // Tooltip dwell (shared tooltip seam): a card only fires while
        // hovering a declared hotspot with no button held and no drag
        // capture — a press never fires one and dismisses any live one.
        let tooltip_dt = tooltip_clock.elapsed();
        tooltip_clock = Instant::now();
        let pointer_held = frame.input.pointer_down
            || frame.input.right_pointer_down
            || modules.is_pointer_captured();
        let hover_hotspot = if pointer_held {
            None
        } else {
            hover_point.and_then(|(x, y)| {
                let id = hovered_id.as_deref()?;
                modules
                    .iter()
                    .find(|module| module.id() == id)
                    .and_then(|module| {
                        module
                            .hotspots()
                            .into_iter()
                            .find(|hotspot| hotspot.rect.contains(x, y))
                    })
            })
        };
        let tooltip_frame = tooltip_state.tick(hover_hotspot.as_ref(), tooltip_dt);
        tooltip_needs_frame = tooltip_frame.dirty;

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
            if last_saved_session_text.as_ref() != Some(&session_text) {
                if last_save_at.elapsed() >= Duration::from_millis(150)
                    && save_painter_user_session_state(&session_state_path, &session_state).is_ok()
                    {
                        last_saved_session_text = Some(session_text);
                        last_save_at = Instant::now();
                        session_dirty = false;
                    }
                    // Save failed or throttled: stay dirty so a later idle
                    // frame still comes back to persist the change.
            } else {
                session_dirty = false;
            }
        }

        // Stamp hover: while a hand equips the stamp and the cursor hovers
        // the drawing surface, the paste preview follows the cursor. The
        // own buffer feeds it (the OS clipboard imports on V); with an empty
        // buffer there is no preview and a press is a no-op.
        let stamp_hover = {
            let tool_state_borrowed = tool_state.borrow();
            let active = tool_state_borrowed.active_hand;
            let stamp_hand = if tool_state_borrowed.tool_for_hand(active) == PaintTool::Stamp {
                Some(active)
            } else if tool_state_borrowed.tool_for_hand(active.opposite()) == PaintTool::Stamp {
                Some(active.opposite())
            } else {
                None
            };
            drop(tool_state_borrowed);
            stamp_hand.and_then(|hand| {
                if !hovering_canvas_bounds {
                    return None;
                }
                let data = user_clipboards.clipboard_for_user(&session_user_id)?.clone();
                let cursor = frame.input.cursor_position?;
                let anchor_world = remap_surface_units_to_active_plane_world(
                    state.camera,
                    cursor,
                    cell_clip_size_for_state(state, frame.surface_size),
                );
                // Same document-space correction as `to_world`: the stamp
                // ghost must sit under the cursor on the moved layer.
                let anchor_world = thaum_renderer_domain::WorldPoint {
                    x: anchor_world.x - active_move_offset.x,
                    y: anchor_world.y - active_move_offset.y,
                    z: anchor_world.z - active_move_offset.z,
                };
                Some(StampHover {
                    hand,
                    anchor: CellPoint {
                        x: anchor_world.x,
                        y: anchor_world.y,
                        z: anchor_world.z,
                    },
                    data,
                })
            })
        };
        pointer_strokes.set_stamp_hover(stamp_hover);

        // The in-flight vector move previews through the render path itself:
        // the active layer's cells shift by the drag's pending delta, live.
        let pending_move_offset = pointer_strokes
            .pending_move_offset()
            .map(|delta| (active_layer_id.clone(), delta));
        let mut groups = build_document_layer_cell_groups(
            &shared_document,
            timeline_state.borrow().current_breath,
            pending_move_offset
                .as_ref()
                .map(|(layer_id, delta)| (layer_id.as_str(), *delta)),
            shape_fade.as_ref(),
        );
        groups.extend(modules.iter().map(|module| module.draw()));
        // In-progress stroke overlays live on the seam: plane selection
        // preview, and any open lasso bound with its live interior preview.
        groups.extend(pointer_strokes.overlay_cell_groups(
            &mut canvas_pointer_context(
                &tool_state,
                &selection,
                &mut canvas,
                &mut shared_document,
                &shared_document_paths,
                &mut shared_action_counter,
                &session_user_id,
                &mut active_layer_id,
            ),
            view_orientation,
            ui_palette.get(UiColorRole::Vivid),
        ));
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
        // Tooltip card: topmost overlay, display-only (never hit-tested),
        // anchored to the hovered hotspot and clamped to the visible screen.
        if let Some(card) = tooltip_frame.card.as_ref() {
            // The visible screen rect in Flat2d local space: cursor/clip space
            // is -1..1 (x right, y up, origin center), the same space
            // `to_screen` consumes, so the screen corners are its ±1 corners —
            // never pixel surface size, which would misplace the clamp.
            let corners = [
                to_screen([-1.0, -1.0]),
                to_screen([1.0, -1.0]),
                to_screen([-1.0, 1.0]),
                to_screen([1.0, 1.0]),
            ];
            let screen_rect = ModuleRect {
                x0: corners.iter().map(|point| point.x).min().unwrap_or(0),
                y0: corners.iter().map(|point| point.y).min().unwrap_or(0),
                x1: corners.iter().map(|point| point.x).max().unwrap_or(0),
                y1: corners.iter().map(|point| point.y).max().unwrap_or(0),
            };
            groups.push(tooltip_card_group(card, screen_rect, &ui_palette));
        }
        // Painter HUD panels are a screen-locked 2D layer: module origins are
        // camera-unit offsets from the focus target and hud pan moves only the
        // HUD (with the wheel/keys compensating focus). This is the shared
        // scene's Flat2d opt-in; default stays world-anchored.
        composition_revision = composition_revision.wrapping_add(1);
        state.composition = Composition::ordered(groups)
            .with_natural_pass_order()
            .with_revision(composition_revision)
            .with_flat_2d_screen_locked(true);
        Ok(())
    })
}
