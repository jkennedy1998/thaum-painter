use std::{
    cell::RefCell,
    rc::Rc,
    time::Instant,
};

use crate::pieces::{cell_type_of, classify_bar_piece, BarPiece, CellType};
use thaum_renderer_domain::{
    char_button_cell, push_text_cells, title_hotspot, Cell, CellGraphic, CellGroup,
    CellGroupIntakeBehavior, CellPoint, CellWeight, DoubleClick, GizmoBar, GizmoClickOutcome,
    GizmoKind, GizmoState, Hotspot, Module, ModulePointerButton, ModulePointerEvent, ModuleRect,
    PanelChrome, PersistedModuleUiState,
    UiColorRole, UiPalette, WorldPoint, text_entry::TextEntryField,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerPropertyKind {
    Raster,
    Move,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PropertyTrackBlock {
    pub id: String,
    pub start_breath: u32,
    pub length_breaths: u32,
    pub is_blank: bool,
    /// The empty's interpolation mode, mirrored raw from the document's
    /// interpretation slot; resolve through `interp_mode::resolve_mode`.
    pub interp_mode: Option<String>,
    /// Ease strengths for the empty's ends (percent steps; unset = 0% linear).
    pub ease_out_percent: Option<u8>,
    pub ease_in_percent: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyTrackRow {
    pub layer_id: String,
    pub property_id: String,
    pub label: String,
    pub kind: LayerPropertyKind,
    pub blocks: Vec<PropertyTrackBlock>,
}

/// One row in the layer list: a saved layer's id, display name, visibility/lock state, and its
/// own timeline bar (the breath range it occupies), all reflected from the real document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerRow {
    pub id: String,
    pub name: String,
    pub visible: bool,
    pub locked: bool,
    pub start_breath: u32,
    pub length_breaths: u32,
}

/// A user action requested through the panel, for the orchestration layer to
/// apply to the real document/session and then reflect back through
/// `LayersPanelState::sync`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayersPanelAction {
    Select(String),
    SelectProperty(String, String),
    AddRequested,
    ToggleVisible(String),
    ToggleLocked(String),
    Delete(String),
    /// Inline rename commit from the layer row's name field (double-click
    /// the name to start, J 2026-09-12).
    Rename(String, String),
    ToggleAutoKey,
    SetCurrentBreath(u32),
    SetLayerTiming(String, u32, u32),
    SetPropertyBlockTiming(String, String, String, u32, u32),
    SetPropertyBlockTimingPushed(String, String, String, u32, u32),
    SetPropertyBlockTimingDestructive(String, String, String, u32, u32),
    BlankPropertyBlock(String, String, String),
    SwapPropertyBlocks(String, String, String, String),
    /// Double-left duplicate (solid single only): the block's full span and content
    /// copy to its right, pushing later bars right (non-destructive, duplicates always
    /// land on the right — J 2026-09-07).
    DuplicatePropertyBlock(String, String, String),
    /// Double-left split (solid center): the bar separates at the double-clicked
    /// breath — the left half keeps the id, the right half gets the rest, and the
    /// track's total content length is unchanged (J 2026-09-07).
    SplitPropertyBlock(String, String, String, u32),
    /// Double-right on an empty bar (any piece, right heads mirrored): the empty
    /// merges into the adjacent content block, preferring the left side, falling back
    /// right, and rejecting when the track has no content at all (J-flip 2026-09-07:
    /// double-right, matching the content-head merge and the right-click delete
    /// family; double-left on empties is reserved for keyframing).
    MergeEmptyPropertyBlock(String, String, String, bool),
    /// Double-left on an empty center/single: cycles the empty's interpolation
    /// mode (interpolate → hold → loop out → loop in → interpolate, J
    /// 2026-09-07). Cycling onto a mode that doesn't use the ease ends clears
    /// both stored ease strengths. First pass is UX-only — playback routes
    /// everything to hold.
    CycleEmptyInterpMode(String, String, String),
    /// Double-left on an empty left head: cycles ease-out strength
    /// 0 → 33 → 66 → 100 → 0 (J 2026-09-07). Rejected at the storage seam when
    /// the mode doesn't use the ease ends.
    CycleEmptyEaseOut(String, String, String),
    /// Double-left on an empty right head: cycles ease-in strength — the
    /// ease-out mirror (J 2026-09-07).
    CycleEmptyEaseIn(String, String, String),
    /// Commits a dragged loop-window bar: the document's active timeline span.
    /// The bar itself cannot be split or deleted, so this is the only edit it
    /// supports beyond hover styling.
    SetLoopWindow(u32, u32),
    /// Play/pause the animation over the loop window (Space is bound to this
    /// in the registry; the bar row's PLAY button toggles it too).
    TogglePlay,
    /// Whether playback wraps at the window edges or stops at the end.
    ToggleLoop,
}

/// Shared state between `LayersPanelModule` and its orchestration caller. The module only ever
/// reads/writes this small struct — it never touches the real document or the timeline session
/// state directly, which keeps it unit-testable without any storage/session plumbing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LayersPanelState {
    pub rows: Vec<LayerRow>,
    pub property_rows: Vec<PropertyTrackRow>,
    pub selected_id: Option<String>,
    pub selected_property_id: Option<String>,
    pub current_breath: u32,
    pub auto_key_enabled: bool,
    /// The document's active timeline span (loop window), mirrored from the
    /// real document each frame.
    pub loop_window_start: u32,
    pub loop_window_end: u32,
    /// Live playback session state, mirrored from the timeline session.
    pub playing: bool,
    pub loop_enabled: bool,
    pending_action: Option<LayersPanelAction>,
    /// Interaction-matrix trace lines the panel pushes while routing pointer
    /// events (which piece × cell type was hit and which branch fired). The
    /// orchestration layer drains and appends them to the interaction log
    /// artifact each frame so J can test drives and the operator can read back
    /// what the panel actually routed.
    interaction_log: Vec<String>,
}

/// The panel log is drained every frame; the cap only bounds a worst case
/// where nobody drains (headless tests, a wedged frame loop).
const INTERACTION_LOG_CAP: usize = 256;

impl LayersPanelState {
    /// Pushes one interaction-trace line onto the panel's log buffer.
    fn log_interaction(&mut self, line: String) {
        if self.interaction_log.len() >= INTERACTION_LOG_CAP {
            self.interaction_log.remove(0);
        }
        self.interaction_log.push(line);
    }

    /// Drains the interaction trace lines for the orchestration layer to
    /// append to the interaction log artifact.
    pub fn take_interaction_log(&mut self) -> Vec<String> {
        std::mem::take(&mut self.interaction_log)
    }

    /// Refreshes the displayed rows/selection/timeline readout from the real document and
    /// session. Called once per frame by the orchestration layer.
    pub fn sync(
        &mut self,
        rows: Vec<LayerRow>,
        property_rows: Vec<PropertyTrackRow>,
        selected_id: Option<String>,
        selected_property_id: Option<String>,
        current_breath: u32,
        auto_key_enabled: bool,
        loop_window_start: u32,
        loop_window_end: u32,
        playing: bool,
        loop_enabled: bool,
    ) {
        self.rows = rows;
        self.property_rows = property_rows;
        self.selected_id = selected_id;
        self.selected_property_id = selected_property_id;
        self.current_breath = current_breath;
        self.auto_key_enabled = auto_key_enabled;
        self.loop_window_start = loop_window_start;
        self.loop_window_end = loop_window_end;
        self.playing = playing;
        self.loop_enabled = loop_enabled;
    }

    /// Takes the pending action, if any, for the orchestration layer to apply. At most one
    /// action is queued per frame.
    pub fn take_pending_action(&mut self) -> Option<LayersPanelAction> {
        self.pending_action.take()
    }

    fn queue_action(&mut self, action: LayersPanelAction) {
        self.pending_action = Some(action);
    }
}

#[cfg(test)]
const ROW_AUTO_KEY: usize = 0;
#[cfg(test)]
const ROW_RULER: usize = 1;
#[cfg(test)]
const ROW_LOOP_BAR: usize = 2;
#[cfg(test)]
const ROW_ADD_LAYER: usize = 3;

const COL_VISIBLE: i32 = 1;
const COL_LOCK: i32 = 3;
/// The delete button lives with the other per-layer controls in the left
/// column instead of alone at the row's right edge (J 2026-09-12).
const COL_DELETE: i32 = 5;
const COL_MARKER: i32 = 7;
const COL_NAME: i32 = 9;
const NAME_WIDTH: i32 = 8;
const TIMELINE_START: i32 = COL_NAME + NAME_WIDTH + 1;
const RENAME_MAX_CHARS: usize = 20;

/// PLAY/LOOP transport toggles on the loop-bar row, left of the timeline
/// (loop window start/end labels sit above the bar on the auto-key row; the
/// transport lives beside the bar it drives).
const PLAY_BUTTON_START: i32 = 1;
const PLAY_BUTTON_END: i32 = 8;
const LOOP_BUTTON_START: i32 = 10;
const LOOP_BUTTON_END: i32 = 17;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PanelRow {
    AutoKeyToggle,
    BreathRuler,
    LoopBar,
    AddLayer,
    Layer(usize),
    Property(usize),
}

/// The layer's own timeline bar (`bar_drag` below) is scaffolded to move/trim like a property
/// block, but nothing yet starts that drag — clicking a layer row's timeline space just
/// selects the layer (see `PanelRow::Layer` handling). Wiring an actual layer-bar drag is out
/// of scope for the property-block click/drag parity pass that split this from
/// `PropertyDragMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
enum BarDragMode {
    Move,
    TrimStart,
    TrimEnd,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BarDrag {
    layer_id: String,
    mode: BarDragMode,
    orig_start: u32,
    orig_length: u32,
    anchor_breath: u32,
}

/// Where on a property bar a pointer event landed, classified with the shared
/// `pieces/` seam: which UX piece of the covering bar × which cell type. The
/// 48-branch interaction matrix keys off exactly this pair (J 2026-09-07).
#[derive(Debug, Clone, PartialEq, Eq)]
struct PropertyBlockHit {
    layer_id: String,
    property_id: String,
    block_id: String,
    breath: u32,
    piece: BarPiece,
    cell_type: CellType,
}

/// How a property-block press-drag reshapes the block, mirroring the old system's
/// left/right-click raster drag modes (`resolve_groups_raster_drag_mode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PropertyDragMode {
    /// Drag on the body: moves the whole block, keeping its length (destructive when
    /// sliding over neighbors).
    Move,
    /// Drag on the left edge: reshapes that edge, anchored at the other edge.
    TrimStart,
    /// Drag on the right edge: reshapes that edge, anchored at the start.
    TrimEnd,
    /// Drag on a single-breath block's body: grows/shrinks from whichever side the
    /// pointer moves toward, flipping freely through the block.
    DynamicResize,
    /// Drag on a multi-breath body (or a blank's center): previews swapping this block's
    /// breath range with whatever other block the pointer is over, committed on release.
    Swap,
}

/// Which mutation seam a property-block drag routes through. Both are legal by
/// construction in `domain/painter-document/properties/`, and the runtime applies the
/// queued action every drag frame — so the live document itself is the preview:
/// pushed neighbors visibly ripple and destructive victims visibly yield.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PropertyTimingResolution {
    /// Time-preserving: neighbors on the dragged side shift by the same delta
    /// (`pushed_breath_span`) — nothing is overwritten, spacing is preserved.
    Pushed,
    /// Destructive: the block takes its full requested span and covered neighbors
    /// truncate, vanish, or split around it (`destructive_breath_span`). A shrink
    /// overlaps nothing, so it acts as a plain trim.
    Destructive,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PropertyBlockDrag {
    layer_id: String,
    property_id: String,
    block_id: String,
    mode: PropertyDragMode,
    resolution: PropertyTimingResolution,
    orig_start: u32,
    orig_length: u32,
    anchor_breath: u32,
    swap_target_block_id: Option<String>,
    /// Most recent requested (start, length) computed during the drag. The drag never
    /// mutates the document mid-flight — the panel only previews — and this final span
    /// is committed exactly once on pointer-up through the queued timing action.
    last_requested: Option<(u32, u32)>,
    /// Set by the move handler when the requested span differs from the previous
    /// frame's, so the interaction log records distinct drag frames only.
    last_requested_changed: bool,
}

/// Resolves what a press-drag on `hit` should do, keyed by piece × cell type per the
/// 48-branch interaction matrix (`context/bars-binary-design-truth.md`, J dictation
/// 2026-09-07). Solid and empty heads drag identically (same pushed/destructive code);
/// solid and empty centers swap on left and destructively slide on right (solid) or
/// nothing (empty); a solid single destructively repositions on left and destructively
/// resizes from whichever side the pointer moves toward on right; an empty single is
/// unused on left and destructively resizes on right.
fn resolve_property_drag_mode(
    hit: &PropertyBlockHit,
    button: ModulePointerButton,
) -> Option<(PropertyDragMode, PropertyTimingResolution)> {
    let is_right = button == ModulePointerButton::Right;
    match (hit.cell_type, hit.piece) {
        // Solid single: left drag is a destructive positional move; right drag is a
        // destructive resize that grows from whichever side the pointer moves toward.
        (CellType::Solid, BarPiece::Single) => Some(if is_right {
            (
                PropertyDragMode::DynamicResize,
                PropertyTimingResolution::Destructive,
            )
        } else {
            (
                PropertyDragMode::Move,
                PropertyTimingResolution::Destructive,
            )
        }),
        // Heads (empty and solid alike): left drag preserves the timing of the bars
        // around it (pushed ripple); right drag overwrites destructively. Same code
        // for both cell types by design.
        (_, BarPiece::LeftHead) => Some(if is_right {
            (
                PropertyDragMode::TrimStart,
                PropertyTimingResolution::Destructive,
            )
        } else {
            (
                PropertyDragMode::TrimStart,
                PropertyTimingResolution::Pushed,
            )
        }),
        (_, BarPiece::RightHead) => Some(if is_right {
            (
                PropertyDragMode::TrimEnd,
                PropertyTimingResolution::Destructive,
            )
        } else {
            (PropertyDragMode::TrimEnd, PropertyTimingResolution::Pushed)
        }),
        // Center: left drag swaps keyframe content with the bar the drag lands on —
        // empty centers swap too, for predictability (J 2026-09-07). Right drag
        // destructively slides a solid center; an empty center is unused on right.
        (CellType::Solid, BarPiece::Center) => Some(if is_right {
            (
                PropertyDragMode::Move,
                PropertyTimingResolution::Destructive,
            )
        } else {
            (
                PropertyDragMode::Swap,
                PropertyTimingResolution::Destructive,
            )
        }),
        (CellType::Empty, BarPiece::Center) => {
            if is_right {
                None
            } else {
                Some((
                    PropertyDragMode::Swap,
                    PropertyTimingResolution::Destructive,
                ))
            }
        }
        // Empty single: left interactions are unused; right drag is the same
        // destructive resize a solid single gets.
        (CellType::Empty, BarPiece::Single) => {
            if is_right {
                Some((
                    PropertyDragMode::DynamicResize,
                    PropertyTimingResolution::Destructive,
                ))
            } else {
                None
            }
        }
    }
}

/// How a press-drag on the loop-window bar reshapes the document's active
/// timeline span. The bar has no swap notion, so left and right body drags
/// behave identically (unlike the property-track bars below); only the edges
/// resize, matching the old system's edge drag without its destructive
/// rewrite (the window is one bar, there is nothing to overwrite).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoopWindowDragMode {
    Move,
    TrimStart,
    TrimEnd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoopWindowHitMode {
    EdgeStart,
    EdgeEnd,
    Body,
}

/// One in-flight loop-window drag. The document is untouched mid-flight —
/// the bar previews at `preview_start/preview_end` — and exactly one
/// `SetLoopWindow` commit is queued on pointer-up.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LoopWindowDrag {
    mode: LoopWindowDragMode,
    orig_start: u32,
    orig_end: u32,
    anchor_breath: u32,
    preview_start: u32,
    preview_end: u32,
}

/// The double-click subject for a property-block click: the button plus every
/// hit identity the 48-branch matrix distinguishes (breath is excluded — two
/// clicks in one block still read as a double-click when they land on the
/// same piece × cell type).
type PropertyBlockClickSubject = (ModulePointerButton, String, String, String, BarPiece, CellType);

/// One in-progress layer rename: which layer's name is being edited and the
/// shared single-line draft field riding the registry's key-capture seam.
struct RenameSession {
    layer_id: String,
    field: TextEntryField,
}

pub struct LayersPanelModule {
    id: String,
    rect: ModuleRect,
    state: Rc<RefCell<LayersPanelState>>,
    palette: UiPalette,
    gizmos: GizmoBar,
    gizmo_state: GizmoState,
    hidden: bool,
    scrubbing_ruler: bool,
    bar_drag: Option<BarDrag>,
    property_block_drag: Option<PropertyBlockDrag>,
    hovered_property_block: Option<PropertyBlockHit>,
    loop_window_drag: Option<LoopWindowDrag>,
    hovered_loop_window: bool,
    hovered_playhead: bool,
    /// Shared click-timing seam detectors (J 2026-09-12): one for property
    /// blocks keyed on the full hit subject, one for layer names keyed on
    /// the layer id.
    raster_double_click: DoubleClick<PropertyBlockClickSubject>,
    name_double_click: DoubleClick<String>,
    /// Some while a layer name is being edited through the key-capture seam.
    rename: Option<RenameSession>,
    /// Last observed pointer position in panel-local coordinates, driving
    /// per-button hover highlight.
    hover_local: Option<(i32, i32)>,
}

impl LayersPanelModule {
    pub fn new(
        id: impl Into<String>,
        rect: ModuleRect,
        state: Rc<RefCell<LayersPanelState>>,
    ) -> Self {
        Self {
            id: id.into(),
            rect,
            state,
            palette: UiPalette::default(),
            gizmos: GizmoBar::standard(),
            gizmo_state: GizmoState::new(),
            hidden: false,
            scrubbing_ruler: false,
            bar_drag: None,
            property_block_drag: None,
            hovered_property_block: None,
            loop_window_drag: None,
            hovered_loop_window: false,
            hovered_playhead: false,
            raster_double_click: DoubleClick::new(),
            name_double_click: DoubleClick::new(),
            rename: None,
            hover_local: None,
        }
    }

    pub fn with_palette(mut self, palette: UiPalette) -> Self {
        self.palette = palette;
        self
    }

    fn row_y(&self, index: usize) -> i32 {
        let (_, content_height) = PanelChrome::content_size(self.rect);
        let (_, content_y) = PanelChrome::content_origin();
        content_y + content_height - 1 - index as i32
    }

    /// Absolute screen-space hotspot over one content-row span (inclusive
    /// local x0..x1), matching row hit-testing conventions.
    fn row_hotspot(
        &self,
        row_index: usize,
        x0: i32,
        x1: i32,
        title: &str,
        description: &str,
    ) -> Hotspot {
        let (content_x, _) = PanelChrome::content_origin();
        let y = self.rect.y0 + self.row_y(row_index);
        Hotspot::new(
            ModuleRect {
                x0: self.rect.x0 + (x0.max(content_x)),
                y0: y,
                x1: self.rect.x0 + x1.max(x0),
                y1: y,
            },
            title,
            description,
        )
    }

    fn visible_rows(&self) -> Vec<PanelRow> {
        let state = self.state.borrow();
        let mut rows = vec![
            PanelRow::AutoKeyToggle,
            PanelRow::BreathRuler,
            PanelRow::LoopBar,
            PanelRow::AddLayer,
        ];
        for (index, row) in state.rows.iter().enumerate() {
            rows.push(PanelRow::Layer(index));
            if state.selected_id.as_deref() == Some(row.id.as_str()) {
                rows.extend(
                    state
                        .property_rows
                        .iter()
                        .enumerate()
                        .filter(|(_, property)| property.layer_id == row.id)
                        .map(|(property_index, _)| PanelRow::Property(property_index)),
                );
            }
        }
        rows
    }

    fn row_at(&self, x: i32, y: i32) -> Option<PanelRow> {
        let local_x = x - self.rect.x0;
        let local_y = y - self.rect.y0;
        let (content_x, _) = PanelChrome::content_origin();
        let (content_width, _) = PanelChrome::content_size(self.rect);
        if local_x < content_x || local_x >= content_x + content_width {
            return None;
        }
        self.visible_rows()
            .into_iter()
            .enumerate()
            .find_map(|(index, row)| (local_y == self.row_y(index)).then_some(row))
    }

    fn content_right(&self) -> i32 {
        let (content_x, _) = PanelChrome::content_origin();
        let (content_width, _) = PanelChrome::content_size(self.rect);
        content_x + (content_width - 1).max(0)
    }

    fn timeline_bounds(&self) -> (i32, i32) {
        let content_right = self.content_right();
        (TIMELINE_START, (content_right - 1).max(TIMELINE_START))
    }

    fn breath_at_x(&self, local_x: i32) -> u32 {
        let (start, end) = self.timeline_bounds();
        local_x.clamp(start, end).saturating_sub(start) as u32
    }

    fn x_for_breath(&self, breath: u32) -> i32 {
        let (start, end) = self.timeline_bounds();
        (start + breath as i32).clamp(start, end)
    }

    fn visible_timeline_end_breath(&self) -> u32 {
        let (start, end) = self.timeline_bounds();
        end.saturating_sub(start) as u32
    }

    /// The loop-window span to draw and hit-test right now: the in-flight
    /// drag's preview span when one is active, otherwise the mirrored
    /// document window (end clamped to never sit below the start).
    fn loop_window_span(&self) -> (u32, u32) {
        if let Some(drag) = &self.loop_window_drag {
            return (drag.preview_start, drag.preview_end.max(drag.preview_start));
        }
        let state = self.state.borrow();
        (
            state.loop_window_start,
            state.loop_window_end.max(state.loop_window_start),
        )
    }

    /// Playhead styling: rests bright at weight 2, hovers vivid at weight 3,
    /// and while its scrub drag is live goes vivid at weight 4.
    fn playhead_style(
        &self,
    ) -> (
        thaum_renderer_domain::CellColor,
        thaum_renderer_domain::CellWeight,
    ) {
        if self.scrubbing_ruler {
            return (
                self.palette.get(UiColorRole::Vivid),
                thaum_renderer_domain::CellWeight::from_index_clamped(4),
            );
        }
        if self.hovered_playhead {
            return (
                self.palette.get(UiColorRole::Vivid),
                thaum_renderer_domain::CellWeight::from_index_clamped(3),
            );
        }
        (
            self.palette.get(UiColorRole::Bright),
            thaum_renderer_domain::CellWeight::from_index_clamped(2),
        )
    }

    fn loop_window_hit_at(&self, x: i32, y: i32) -> Option<LoopWindowHitMode> {
        if !matches!(self.row_at(x, y), Some(PanelRow::LoopBar)) {
            return None;
        }
        let local_x = x - self.rect.x0;
        // Left of the timeline is the PLAY/LOOP transport, never the bar.
        if local_x < TIMELINE_START {
            return None;
        }
        let (start, end) = self.loop_window_span();
        let breath = self.breath_at_x(local_x);
        if breath < start || breath > end {
            return None;
        }
        let start_x = self.x_for_breath(start);
        let end_x = self.x_for_breath(end);
        if start_x == end_x {
            return Some(LoopWindowHitMode::Body);
        }
        if local_x - start_x <= 1 {
            return Some(LoopWindowHitMode::EdgeStart);
        }
        if end_x - local_x <= 1 {
            return Some(LoopWindowHitMode::EdgeEnd);
        }
        Some(LoopWindowHitMode::Body)
    }

    fn property_bar_bounds(&self, property: &PropertyTrackRow) -> Option<(u32, u32)> {
        let start = property
            .blocks
            .iter()
            .map(|block| block.start_breath)
            .min()?;
        let end = property
            .blocks
            .iter()
            .map(|block| block.start_breath + block.length_breaths.saturating_sub(1))
            .max()?;
        Some((start, end))
    }

    fn layer_visible_for_property(
        &self,
        state: &LayersPanelState,
        property: &PropertyTrackRow,
    ) -> bool {
        state
            .rows
            .iter()
            .find(|row| row.id == property.layer_id)
            .map(|row| row.visible)
            .unwrap_or(true)
    }

    fn property_block_hit_at(&self, x: i32, y: i32) -> Option<PropertyBlockHit> {
        let PanelRow::Property(property_index) = self.row_at(x, y)? else {
            return None;
        };
        let state = self.state.borrow();
        let property = state.property_rows.get(property_index)?;
        let local_x = x - self.rect.x0;
        let breath = self.breath_at_x(local_x);
        let block = property.blocks.iter().find(|block| {
            let end = block.start_breath + block.length_breaths.max(1).saturating_sub(1);
            breath >= block.start_breath && breath <= end
        });
        // Past every stored block the track resolves as the trailing blank (the
        // infinite empty region — J 2026-09-07): hit the last block when it is the
        // trailing blank, otherwise the implicit blank region past a solid tail.
        let (block, force_center) = match block {
            Some(block) => (block, false),
            None => {
                let last = property.blocks.last()?;
                if !(last.is_blank && breath > last.start_breath) {
                    return None;
                }
                (last, true)
            }
        };
        Some(PropertyBlockHit {
            layer_id: property.layer_id.clone(),
            property_id: property.property_id.clone(),
            block_id: block.id.clone(),
            breath,
            piece: if force_center {
                BarPiece::Center
            } else {
                classify_bar_piece(block.start_breath, block.length_breaths, breath)
            },
            cell_type: cell_type_of(block.is_blank),
        })
    }

    fn property_block_interaction_style(
        &self,
        property: &PropertyTrackRow,
        block: &PropertyTrackBlock,
        breath: u32,
    ) -> (
        thaum_renderer_domain::CellColor,
        thaum_renderer_domain::CellWeight,
    ) {
        let hover_matches_block = self
            .hovered_property_block
            .as_ref()
            .map(|hover| {
                hover.layer_id == property.layer_id
                    && hover.property_id == property.property_id
                    && hover.block_id == block.id
            })
            .unwrap_or(false);
        let hover_matches_breath = self
            .hovered_property_block
            .as_ref()
            .map(|hover| hover_matches_block && hover.breath == breath)
            .unwrap_or(false);
        let drag_matches_block = self
            .property_block_drag
            .as_ref()
            .map(|drag| {
                (drag.layer_id == property.layer_id
                    && drag.property_id == property.property_id
                    && drag.block_id == block.id)
                    || (drag.mode == PropertyDragMode::Swap
                        && drag.property_id == property.property_id
                        && drag.layer_id == property.layer_id
                        && drag.swap_target_block_id.as_deref() == Some(block.id.as_str()))
            })
            .unwrap_or(false);

        let base_color = match property.kind {
            LayerPropertyKind::Raster => self.palette.get(UiColorRole::Bright),
            LayerPropertyKind::Move => self.palette.get(UiColorRole::Medium),
        };
        // Weight bands (J 2026-09-07): blank bars live at weights 0–1 (rest 0,
        // highlight 1) and content bars at 2–3 (rest 2, highlight 3), so the
        // empty keyframe glyphs only ever render inside the blank band.
        let (base_weight, highlight_weight) = if block.is_blank { (0, 1) } else { (2, 3) };
        if drag_matches_block {
            let is_swap_target = self
                .property_block_drag
                .as_ref()
                .map(|drag| {
                    drag.mode == PropertyDragMode::Swap
                        && drag.swap_target_block_id.as_deref() == Some(block.id.as_str())
                })
                .unwrap_or(false);
            let highlight = if is_swap_target {
                self.palette.get(UiColorRole::Medium)
            } else {
                self.palette.get(UiColorRole::Vivid)
            };
            return (
                highlight,
                thaum_renderer_domain::CellWeight::from_index_clamped(highlight_weight),
            );
        }
        if hover_matches_breath {
            return (
                self.palette.get(UiColorRole::Vivid),
                thaum_renderer_domain::CellWeight::from_index_clamped(highlight_weight),
            );
        }
        if hover_matches_block {
            return (
                self.palette.get(UiColorRole::Vivid),
                thaum_renderer_domain::CellWeight::from_index_clamped(highlight_weight),
            );
        }
        (
            base_color,
            thaum_renderer_domain::CellWeight::from_index_clamped(base_weight),
        )
    }

    fn find_property_row(&self, hit: &PropertyBlockHit) -> Option<PropertyTrackRow> {
        self.state
            .borrow()
            .property_rows
            .iter()
            .find(|property| {
                property.layer_id == hit.layer_id && property.property_id == hit.property_id
            })
            .cloned()
    }

    fn find_property_block(&self, hit: &PropertyBlockHit) -> Option<PropertyTrackBlock> {
        self.find_property_row(hit).and_then(|property| {
            property
                .blocks
                .into_iter()
                .find(|block| block.id == hit.block_id)
        })
    }

    fn handle_property_block_click(&mut self, hit: PropertyBlockHit, button: ModulePointerButton) {
        let now = Instant::now();
        let subject: PropertyBlockClickSubject = (
            button,
            hit.layer_id.clone(),
            hit.property_id.clone(),
            hit.block_id.clone(),
            hit.piece,
            hit.cell_type,
        );
        let is_double_click = self.raster_double_click.note_click(subject, now);

        if is_double_click {
            self.property_block_drag = None;
            self.handle_property_block_double_click(hit, button);
            return;
        }

        // Selection rule (J 2026-09-07): any interaction on a property row selects that
        // layer's property first — "unused" branches mean no additional action, never
        // no selection.
        self.state
            .borrow_mut()
            .queue_action(LayersPanelAction::SelectProperty(
                hit.layer_id.clone(),
                hit.property_id.clone(),
            ));

        let Some((mode, resolution)) = resolve_property_drag_mode(&hit, button) else {
            self.state.borrow_mut().log_interaction(format!(
                "click {} {}/{} breath={} block={} {:?}/{:?} -> unused (select only)",
                button_name(button),
                hit.layer_id,
                hit.property_id,
                hit.breath,
                hit.block_id,
                hit.piece,
                hit.cell_type,
            ));
            return;
        };
        let Some(block) = self.find_property_block(&hit) else {
            return;
        };
        self.state.borrow_mut().log_interaction(format!(
            "click {} {}/{} breath={} block={} {:?}/{:?} -> drag {:?}/{:?}",
            button_name(button),
            hit.layer_id,
            hit.property_id,
            hit.breath,
            hit.block_id,
            hit.piece,
            hit.cell_type,
            mode,
            resolution,
        ));
        self.property_block_drag = Some(PropertyBlockDrag {
            layer_id: hit.layer_id,
            property_id: hit.property_id,
            block_id: hit.block_id,
            mode,
            resolution,
            orig_start: block.start_breath,
            orig_length: block.length_breaths.max(1),
            anchor_breath: hit.breath,
            swap_target_block_id: None,
            last_requested: None,
            last_requested_changed: false,
        });
    }

    /// Dispatches the double-click branches of the 48-branch interaction matrix. Every
    /// branch already selected the property through the first click of the pair.
    fn handle_property_block_double_click(
        &mut self,
        hit: PropertyBlockHit,
        button: ModulePointerButton,
    ) {
        let trace_prefix = format!(
            "dblclick {} {}/{} breath={} block={} {:?}/{:?}",
            button_name(button),
            hit.layer_id,
            hit.property_id,
            hit.breath,
            hit.block_id,
            hit.piece,
            hit.cell_type,
        );
        let trace = |panel: &Self, outcome: String| {
            panel
                .state
                .borrow_mut()
                .log_interaction(format!("{trace_prefix} -> {outcome}"));
        };
        match (hit.cell_type, hit.piece, button) {
            // Solid center: double-left splits the bar at the double-clicked breath —
            // total content length is preserved, the bar just separates there (J
            // 2026-09-07). A solid single cannot split (length 1), so double-left
            // duplicates to the right instead; double-right replaces the span with
            // empties — the new delete.
            (CellType::Solid, BarPiece::Center, ModulePointerButton::Left) => {
                trace(self, format!("SplitPropertyBlock at breath={}", hit.breath));
                self.state
                    .borrow_mut()
                    .queue_action(LayersPanelAction::SplitPropertyBlock(
                        hit.layer_id,
                        hit.property_id,
                        hit.block_id,
                        hit.breath,
                    ));
            }
            (CellType::Solid, BarPiece::Single, ModulePointerButton::Left) => {
                trace(self, "DuplicatePropertyBlock".into());
                self.state
                    .borrow_mut()
                    .queue_action(LayersPanelAction::DuplicatePropertyBlock(
                        hit.layer_id,
                        hit.property_id,
                        hit.block_id,
                    ));
            }
            (CellType::Solid, BarPiece::Single, ModulePointerButton::Right)
            | (CellType::Solid, BarPiece::Center, ModulePointerButton::Right) => {
                trace(self, "BlankPropertyBlock".into());
                self.state
                    .borrow_mut()
                    .queue_action(LayersPanelAction::BlankPropertyBlock(
                        hit.layer_id,
                        hit.property_id,
                        hit.block_id,
                    ));
            }
            // Solid heads: double-RIGHT merges the adjacent bar into this one — the
            // double-clicked bar's content stays, implemented as a destructive resize
            // to the neighbor's far edge (J-flip 2026-09-07: moved off double-left so
            // right-click stays the destructive family and double-left on content ends
            // stays free for keyframe interpolation between empties and this cell).
            // Right heads mirror: the bar on the right merges in. Rejected at the span
            // edge (no neighbor to run into). Double-left on heads is unused for now.
            (CellType::Solid, BarPiece::LeftHead, ModulePointerButton::Right) => {
                self.queue_content_head_merge(&hit, false, &trace);
            }
            (CellType::Solid, BarPiece::RightHead, ModulePointerButton::Right) => {
                self.queue_content_head_merge(&hit, true, &trace);
            }
            // Empty bars: double-RIGHT merges the empty into the adjacent content
            // block (J-flip 2026-09-07: moved off double-left so right-click stays
            // the delete/merge family and double-left on empties is reserved for
            // keyframing). One seam, left-preferred with right fallback, rejecting
            // on a fully-empty track; right heads mirror to prefer the right. All
            // pieces route here: single, center, both heads.
            (CellType::Empty, _, ModulePointerButton::Right) => {
                let prefer_left = hit.piece != BarPiece::RightHead;
                trace(
                    self,
                    format!("MergeEmptyPropertyBlock prefer_left={prefer_left}"),
                );
                self.state
                    .borrow_mut()
                    .queue_action(LayersPanelAction::MergeEmptyPropertyBlock(
                        hit.layer_id,
                        hit.property_id,
                        hit.block_id,
                        prefer_left,
                    ));
            }
            // Empty bars, left button: the keyframing surface (J 2026-09-07).
            // Center/single cycles the empty's interpolation mode; the left end
            // cycles ease-out strength, the right end ease-in strength. The
            // right-dblclick merge above stays the destructive family.
            (CellType::Empty, BarPiece::Center | BarPiece::Single, ModulePointerButton::Left) => {
                trace(self, "CycleEmptyInterpMode".into());
                self.state
                    .borrow_mut()
                    .queue_action(LayersPanelAction::CycleEmptyInterpMode(
                        hit.layer_id,
                        hit.property_id,
                        hit.block_id,
                    ));
            }
            (CellType::Empty, BarPiece::LeftHead, ModulePointerButton::Left) => {
                trace(self, "CycleEmptyEaseOut".into());
                self.state
                    .borrow_mut()
                    .queue_action(LayersPanelAction::CycleEmptyEaseOut(
                        hit.layer_id,
                        hit.property_id,
                        hit.block_id,
                    ));
            }
            (CellType::Empty, BarPiece::RightHead, ModulePointerButton::Left) => {
                trace(self, "CycleEmptyEaseIn".into());
                self.state
                    .borrow_mut()
                    .queue_action(LayersPanelAction::CycleEmptyEaseIn(
                        hit.layer_id,
                        hit.property_id,
                        hit.block_id,
                    ));
            }
            _ => {
                trace(self, "unused".into());
            }
        }
    }

    /// Resolves a solid head's double-left merge into a destructive resize that spans
    /// from the adjacent neighbor's far edge through this bar's own span. With no
    /// adjacent block (the bar sits at the layer-span edge) the interaction rejects —
    /// under tiling a non-edge bar always has something to run into.
    fn queue_content_head_merge(
        &mut self,
        hit: &PropertyBlockHit,
        mirror_right: bool,
        trace: &dyn Fn(&Self, String),
    ) {
        let Some(row) = self.find_property_row(hit) else {
            return;
        };
        let Some(block) = row.blocks.iter().find(|block| block.id == hit.block_id) else {
            return;
        };
        let block_end = block.start_breath + block.length_breaths.max(1) - 1;
        let merged = if mirror_right {
            let neighbor = row
                .blocks
                .iter()
                .find(|other| other.start_breath == block_end.saturating_add(1));
            let Some(neighbor) = neighbor else {
                trace(self, "merge head reject: no right neighbor".into());
                return; // right span edge — no neighbor to merge in
            };
            let neighbor_end =
                neighbor.start_breath + neighbor.length_breaths.max(1).saturating_sub(1);
            (block.start_breath, neighbor_end - block.start_breath + 1)
        } else {
            let neighbor = row.blocks.iter().find(|other| {
                other.start_breath + other.length_breaths.max(1) == block.start_breath
            });
            let Some(neighbor) = neighbor else {
                trace(self, "merge head reject: no left neighbor".into());
                return; // left span edge — no neighbor to merge in
            };
            (neighbor.start_breath, block_end - neighbor.start_breath + 1)
        };
        trace(
            self,
            format!(
                "merge head mirror_right={mirror_right} -> destructive span=({}, {})",
                merged.0, merged.1,
            ),
        );
        self.state
            .borrow_mut()
            .queue_action(LayersPanelAction::SetPropertyBlockTimingDestructive(
                hit.layer_id.clone(),
                hit.property_id.clone(),
                hit.block_id.clone(),
                merged.0,
                merged.1,
            ));
    }
}

/// Short button tag for the interaction trace lines.
fn button_name(button: ModulePointerButton) -> &'static str {
    if button == ModulePointerButton::Right {
        "R"
    } else {
        "L"
    }
}

fn property_track_cell_graphic(
    is_blank: bool,
    is_visible: bool,
    length_breaths: u32,
    local_index: u32,
) -> char {
    let is_single = length_breaths <= 1;
    let is_first = local_index == 0;
    let is_last = local_index + 1 >= length_breaths;

    if is_blank {
        if is_single {
            return '▪';
        }
        if is_first {
            return '◧';
        }
        if is_last {
            return '◨';
        }
        return '▪';
    }

    if is_visible {
        if is_single {
            return '█';
        }
        if is_first {
            return '█';
        }
        if is_last {
            return '▦';
        }
        return '▥';
    }

    if is_single {
        return '▒';
    }
    if is_first {
        return '╺';
    }
    if is_last {
        return '╸';
    }
    '╌'
}

/// The blank-bar keyframe graphics (J 2026-09-07): centers/singles show the
/// interpolation mode, the ends show the ease strengths (or the fixed
/// non-adjustable ◧/◨ when the mode doesn't use the ease ends). Only used on
/// property-track blanks — blank bars never render above weight 1, and these
/// glyphs have no weight-2/3 art yet.
fn property_blank_cell_graphic(
    interp_mode: Option<&str>,
    ease_out_percent: Option<u8>,
    ease_in_percent: Option<u8>,
    graphic_length: u32,
    local_index: u32,
) -> char {
    let mode = crate::interp_mode::resolve_mode(interp_mode);
    let is_single = graphic_length <= 1;
    let is_first = local_index == 0;
    let is_last = local_index + 1 >= graphic_length;
    if is_single || !(is_first || is_last) {
        return crate::interp_mode::mode_center_glyph(mode);
    }
    if !crate::interp_mode::mode_is_ease_adjustable(mode) {
        // A single empty never reaches here (handled above); a 2+ bar's ends
        // render the fixed non-adjustable glyphs.
        return if is_first { '◧' } else { '◨' };
    }
    if is_first {
        crate::interp_mode::ease_out_glyph(ease_out_percent)
    } else {
        crate::interp_mode::ease_in_glyph(ease_in_percent)
    }
}

impl Module for LayersPanelModule {
    fn id(&self) -> &str {
        &self.id
    }

    fn rect(&self) -> ModuleRect {
        self.rect
    }

    /// Tooltip hotspots: the module's gizmo bar plus one hotspot per custom
    /// control row, so every panel control explains itself through the
    /// shared tooltip implementation.
    fn hotspots(&self) -> Vec<Hotspot> {
        let (timeline_start, timeline_end) = self.timeline_bounds();
        let rows = self.visible_rows();
        let state = self.state.borrow();
        let mut custom = vec![title_hotspot(
            self.rect,
            self.gizmos.title_start_x(),
            "layers module",
            "layers are 3d and temporal. all layers are animated and in 3d space. layers override graphics from other layers -- not render over.",
        )];
        custom.push(self.row_hotspot(
            0,
            1,
            timeline_start - 2,
            "auto key",
            "click to toggle auto-key: canvas drags author keyframes at the playhead",
        ));
        custom.push(self.row_hotspot(
            1,
            timeline_start,
            timeline_end,
            "breath ruler",
            "click or drag to move the playhead",
        ));
        custom.push(self.row_hotspot(
            2,
            PLAY_BUTTON_START,
            PLAY_BUTTON_END,
            "play",
            "plays or stops playback over the loop window",
        ));
        custom.push(self.row_hotspot(
            2,
            LOOP_BUTTON_START,
            LOOP_BUTTON_END,
            "loop",
            "toggles wrap-at-edges playback inside the loop window",
        ));
        custom.push(self.row_hotspot(
            2,
            timeline_start,
            timeline_end,
            "loop window",
            "drag the bar to move the window, grab an end to trim it",
        ));
        custom.push(self.row_hotspot(3, 1, 12, "new layer", "adds a new layer to the document"));
        for (index, row_kind) in rows.into_iter().enumerate().skip(4) {
            match row_kind {
                PanelRow::Layer(i) => {
                    let row = state.rows.get(i);
                    let name = row
                        .map(|row| row.name.as_str())
                        .unwrap_or("layer");
                    let (visible_title, visible_desc) = match row.map(|row| row.visible) {
                        Some(true) => ("visible (o)", "click to hide this layer"),
                        Some(false) => ("hidden (-)", "click to show this layer"),
                        None => ("visibility", "click to toggle this layer's visibility"),
                    };
                    let (lock_title, lock_desc) = match row.map(|row| row.locked) {
                        Some(true) => ("locked (L)", "click to unlock edits on this layer"),
                        Some(false) => ("unlocked (U)", "click to lock edits on this layer"),
                        None => ("lock", "click to toggle edit lock on this layer"),
                    };
                    custom.push(self.row_hotspot(
                        index,
                        COL_VISIBLE,
                        COL_VISIBLE,
                        visible_title,
                        visible_desc,
                    ));
                    custom.push(self.row_hotspot(
                        index,
                        COL_LOCK,
                        COL_LOCK,
                        lock_title,
                        lock_desc,
                    ));
                    custom.push(self.row_hotspot(
                        index,
                        COL_DELETE,
                        COL_DELETE,
                        "delete (X)",
                        "click to delete this layer",
                    ));
                    custom.push(self.row_hotspot(
                        index,
                        COL_NAME,
                        timeline_start - 2,
                        name,
                        "double-click the name to rename; the timeline columns to the right are the layer's bar",
                    ));
                }
                PanelRow::Property(i) => {
                    let label = state
                        .property_rows
                        .get(i)
                        .map(|row| row.label.as_str())
                        .unwrap_or("property");
                    custom.push(self.row_hotspot(
                        index,
                        1,
                        timeline_end,
                        label,
                        "property track: click a bar to select, drag to move or trim keyframes",
                    ));
                }
                _ => {}
            }
        }
        drop(state);
        self.gizmos.hotspots_with(self.rect, custom)
    }

    fn draw(&self) -> CellGroup {
        let origin = WorldPoint {
            x: self.rect.x0,
            y: self.rect.y0,
            z: 0,
        };
        let mut cells: Vec<Cell> = if self.gizmo_state.is_seamless() {
            Vec::new()
        } else {
            self.gizmo_state
                .decorate_panel_chrome(
                    PanelChrome::new(self.rect, &self.palette)
                        .with_title("LAYERS")
                        .with_title_start_x(self.gizmos.title_start_x()),
                    &self.palette,
                )
                .cells()
        };
        if self.gizmo_state.should_draw_gizmo_bar() {
            cells.extend(
                self.gizmos
                    .cells(self.rect, &self.gizmo_state, &self.palette),
            );
        }

        let state = self.state.borrow();
        let content_right = self.content_right();
        let visible_rows = self.visible_rows();

        // Name text follows the char-button weight seam (J 2026-09-12):
        // weight 1 at rest, weight 2 while highlighted (hover or the live
        // rename edit) — never weight 0, which reads as invisible chrome.
        let name_hovered = |name_end_x: i32, row_y: i32| {
            self.hover_local
                .is_some_and(|(x, hy)| hy == row_y && x >= COL_NAME && x <= name_end_x)
        };
        let name_weight =
            |name_end_x: i32, row_y: i32, highlighted: bool| {
                if highlighted || name_hovered(name_end_x, row_y) {
                    CellWeight::from_index_clamped(2)
                } else {
                    CellWeight::from_index_clamped(1)
                }
            };

        let (timeline_start, timeline_end) = self.timeline_bounds();
        let playhead_x = self.x_for_breath(state.current_breath);

        for (index, row_kind) in visible_rows.into_iter().enumerate() {
            let y = self.row_y(index);
            match row_kind {
                PanelRow::AutoKeyToggle => {
                    let auto_key_glyph = if state.auto_key_enabled { 'x' } else { ' ' };
                    push_text_cells(
                        &mut cells,
                        1,
                        y,
                        &format!("[{auto_key_glyph}] AUTO KEY"),
                        self.palette.get(UiColorRole::Medium),
                        CellWeight::from_index_clamped(1),
                        timeline_start - 2,
                    );

                    let end_breath = self.visible_timeline_end_breath();
                    let start_label = "0";
                    let current_label = state.current_breath.to_string();
                    let end_label = end_breath.to_string();
                    push_text_cells(
                        &mut cells,
                        timeline_start,
                        y,
                        start_label,
                        self.palette.get(UiColorRole::Dimmest),
                        CellWeight::from_index_clamped(1),
                        timeline_end,
                    );
                    let end_start = (timeline_end - end_label.len() as i32 + 1).max(timeline_start);
                    push_text_cells(
                        &mut cells,
                        end_start,
                        y,
                        &end_label,
                        self.palette.get(UiColorRole::Dimmest),
                        CellWeight::from_index_clamped(1),
                        timeline_end,
                    );
                    // Loop-window edge labels sit above the bar's ends; the
                    // current-breath label below draws last so it wins any
                    // column collision, like the old system's label priority.
                    let (loop_start, loop_end) = self.loop_window_span();
                    if loop_end > loop_start {
                        let loop_start_label = loop_start.to_string();
                        let loop_end_label = loop_end.to_string();
                        let loop_start_x = (self.x_for_breath(loop_start)
                            - (loop_start_label.len() as i32 / 2))
                            .clamp(
                                timeline_start,
                                timeline_end - loop_start_label.len() as i32 + 1,
                            );
                        push_text_cells(
                            &mut cells,
                            loop_start_x,
                            y,
                            &loop_start_label,
                            self.palette.get(UiColorRole::Medium),
                            CellWeight::from_index_clamped(1),
                            timeline_end,
                        );
                        let loop_end_x = (self.x_for_breath(loop_end)
                            - (loop_end_label.len() as i32 / 2))
                            .clamp(
                                timeline_start,
                                timeline_end - loop_end_label.len() as i32 + 1,
                            );
                        push_text_cells(
                            &mut cells,
                            loop_end_x,
                            y,
                            &loop_end_label,
                            self.palette.get(UiColorRole::Medium),
                            CellWeight::from_index_clamped(1),
                            timeline_end,
                        );
                    }
                    let current_start = (playhead_x - (current_label.len() as i32 / 2)).clamp(
                        timeline_start,
                        timeline_end - current_label.len() as i32 + 1,
                    );
                    let (playhead_color, playhead_weight) = self.playhead_style();
                    let mut current_cell = Cell {
                        position: CellPoint {
                            x: current_start,
                            y,
                            z: 0,
                        },
                        graphic: CellGraphic::Glyph(' '),
                        color: playhead_color,
                        weight: playhead_weight,
                        ..Cell::default()
                    };
                    for (offset, ch) in current_label.chars().enumerate() {
                        current_cell.position.x = current_start + offset as i32;
                        current_cell.graphic = CellGraphic::Glyph(ch);
                        cells.push(current_cell.clone());
                    }
                }
                PanelRow::BreathRuler => {
                    for x in timeline_start..=timeline_end {
                        cells.push(Cell {
                            position: CellPoint { x, y, z: 0 },
                            graphic: CellGraphic::Glyph('─'),
                            color: self.palette.get(UiColorRole::Dimmest),
                            ..Cell::default()
                        });
                    }
                    let (playhead_color, playhead_weight) = self.playhead_style();
                    cells.push(Cell {
                        position: CellPoint {
                            x: playhead_x,
                            y,
                            z: 0,
                        },
                        graphic: CellGraphic::Glyph('║'),
                        color: playhead_color,
                        weight: playhead_weight,
                        ..Cell::default()
                    });
                }
                PanelRow::LoopBar => {
                    // Transport toggles left of the timeline, next to the bar
                    // they drive: PLAY pauses/resumes playback over the
                    // window, LOOP toggles wrap-at-edges.
                    let play_label = if state.playing {
                        "[||] STOP"
                    } else {
                        "[>] PLAY"
                    };
                    push_text_cells(
                        &mut cells,
                        PLAY_BUTTON_START,
                        y,
                        play_label,
                        if state.playing {
                            self.palette.get(UiColorRole::Vivid)
                        } else {
                            self.palette.get(UiColorRole::Medium)
                        },
                        CellWeight::from_index_clamped(1),
                        PLAY_BUTTON_END,
                    );
                    let loop_label = if state.loop_enabled {
                        "[x] LOOP"
                    } else {
                        "[ ] LOOP"
                    };
                    push_text_cells(
                        &mut cells,
                        LOOP_BUTTON_START,
                        y,
                        loop_label,
                        if state.loop_enabled {
                            self.palette.get(UiColorRole::Medium)
                        } else {
                            self.palette.get(UiColorRole::Dimmest)
                        },
                        CellWeight::from_index_clamped(1),
                        LOOP_BUTTON_END,
                    );
                    for x in timeline_start..=timeline_end {
                        cells.push(Cell {
                            position: CellPoint { x, y, z: 0 },
                            graphic: CellGraphic::Glyph('·'),
                            color: self.palette.get(UiColorRole::Dimmest),
                            ..Cell::default()
                        });
                    }
                    let (loop_start, loop_end) = self.loop_window_span();
                    let length = loop_end - loop_start + 1;
                    let loop_start_x = self.x_for_breath(loop_start);
                    let highlighted = self.loop_window_drag.is_some() || self.hovered_loop_window;
                    let (color, weight) = if highlighted {
                        (
                            self.palette.get(UiColorRole::Vivid),
                            thaum_renderer_domain::CellWeight::from_index_clamped(2),
                        )
                    } else {
                        (
                            self.palette.get(UiColorRole::Bright),
                            thaum_renderer_domain::CellWeight::from_index_clamped(1),
                        )
                    };
                    for local_index in 0..length {
                        let x = loop_start_x + local_index as i32;
                        if x > timeline_end {
                            break;
                        }
                        cells.push(Cell {
                            position: CellPoint { x, y, z: 0 },
                            graphic: CellGraphic::Glyph(property_track_cell_graphic(
                                false,
                                true,
                                length,
                                local_index,
                            )),
                            color,
                            weight,
                            ..Cell::default()
                        });
                    }
                }
                PanelRow::AddLayer => {
                    push_text_cells(
                        &mut cells,
                        1,
                        y,
                        "+ NEW LAYER",
                        self.palette.get(UiColorRole::Medium),
                        CellWeight::from_index_clamped(1),
                        content_right,
                    );
                }
                PanelRow::Layer(layer_index) => {
                    let row = &state.rows[layer_index];
                    let is_selected = state.selected_id.as_deref() == Some(row.id.as_str());
                    let text_color = if is_selected {
                        self.palette.get(UiColorRole::Bright)
                    } else {
                        self.palette.get(UiColorRole::Medium)
                    };
                    // Per-layer single-character buttons via the shared
                    // char-button seam: bright weight 1 at rest, vivid
                    // weight 2 on hover (J 2026-09-12).
                    let o_glyph = if row.visible { 'o' } else { '-' };
                    let lock_glyph = if row.locked { 'L' } else { 'U' };
                    let marker = if is_selected { '*' } else { '-' };
                    let button_hovered = |col: i32| self.hover_local == Some((col, y));

                    cells.push(char_button_cell(
                        COL_VISIBLE,
                        y,
                        o_glyph,
                        &self.palette,
                        button_hovered(COL_VISIBLE),
                    ));
                    cells.push(char_button_cell(
                        COL_LOCK,
                        y,
                        lock_glyph,
                        &self.palette,
                        button_hovered(COL_LOCK),
                    ));
                    cells.push(char_button_cell(
                        COL_DELETE,
                        y,
                        'X',
                        &self.palette,
                        button_hovered(COL_DELETE),
                    ));
                    cells.push(Cell {
                        position: CellPoint {
                            x: COL_MARKER,
                            y,
                            z: 0,
                        },
                        graphic: CellGraphic::Glyph(marker),
                        color: text_color,
                        ..Cell::default()
                    });
                    if self
                        .rename
                        .as_ref()
                        .is_some_and(|session| session.layer_id == row.id)
                    {
                        push_text_cells(
                            &mut cells,
                            COL_NAME,
                            y,
                            &self.rename.as_ref().expect("rename session").field.display(),
                            self.palette.get(UiColorRole::Vivid),
                            name_weight(timeline_start - 2, y, true),
                            timeline_start - 2,
                        );
                    } else {
                        push_text_cells(
                            &mut cells,
                            COL_NAME,
                            y,
                            &row.name,
                            text_color,
                            name_weight(timeline_start - 2, y, false),
                            timeline_start - 2,
                        );
                    }

                    for x in timeline_start..=timeline_end {
                        cells.push(Cell {
                            position: CellPoint { x, y, z: 0 },
                            graphic: CellGraphic::Glyph(' '),
                            color: self.palette.get(UiColorRole::Dimmest),
                            ..Cell::default()
                        });
                    }
                    cells.push(Cell {
                        position: CellPoint {
                            x: playhead_x,
                            y,
                            z: 0,
                        },
                        graphic: CellGraphic::Glyph('│'),
                        color: self.palette.get(UiColorRole::Dimmest),
                        ..Cell::default()
                    });
                }
                PanelRow::Property(property_index) => {
                    let property = &state.property_rows[property_index];
                    let is_selected = state.selected_property_id.as_deref()
                        == Some(property.property_id.as_str());
                    let is_visible = self.layer_visible_for_property(&state, property);
                    let text_color = if is_selected {
                        self.palette.get(UiColorRole::Vivid)
                    } else {
                        self.palette.get(UiColorRole::Medium)
                    };
                    let marker = if is_selected { '>' } else { ':' };
                    cells.push(Cell {
                        position: CellPoint {
                            x: COL_MARKER,
                            y,
                            z: 0,
                        },
                        graphic: CellGraphic::Glyph(marker),
                        color: text_color,
                        ..Cell::default()
                    });
                    push_text_cells(
                        &mut cells,
                        COL_NAME,
                        y,
                        &property.label,
                        text_color,
                        name_weight(timeline_start - 2, y, is_selected),
                        timeline_start - 2,
                    );
                    for x in timeline_start..=timeline_end {
                        cells.push(Cell {
                            position: CellPoint { x, y, z: 0 },
                            graphic: CellGraphic::Glyph('·'),
                            color: self.palette.get(UiColorRole::Dimmest),
                            ..Cell::default()
                        });
                    }
                    let drag_preview = self
                        .property_block_drag
                        .as_ref()
                        .filter(|drag| {
                            drag.layer_id == property.layer_id
                                && drag.property_id == property.property_id
                                && drag.mode != PropertyDragMode::Swap
                        })
                        .map(|drag| (drag.block_id.clone(), drag.last_requested));
                    // Two passes so the dragged bar previews on top (J 2026-09-07):
                    // victims draw at their stored spans first, then the dragged bar
                    // overdraws its requested span, so a rightward destructive drag
                    // shows the final state instead of the victim covering it.
                    let mut preview_block: Option<(&PropertyTrackBlock, (u32, u32))> = None;
                    for (block_index, block) in property.blocks.iter().enumerate() {
                        // While a timing drag is in flight the document is untouched,
                        // so the dragged bar previews at its requested span instead of
                        // its stored one; victims stay intact until the release commit.
                        let (draw_start, draw_length) = match (&drag_preview, block) {
                            (Some((drag_block_id, Some((start, length)))), previewed)
                                if drag_block_id == &previewed.id =>
                            {
                                preview_block = Some((previewed, (*start, *length)));
                                continue;
                            }
                            _ => (block.start_breath, block.length_breaths.max(1)),
                        };
                        // The trailing blank is the infinite empty region (J
                        // 2026-09-07): it renders through to the panel's right edge
                        // with no right head — it has no end. The stored extent is
                        // only a finite representative.
                        let is_trailing_blank =
                            block_index + 1 == property.blocks.len() && block.is_blank;
                        let graphic_length = if is_trailing_blank {
                            u32::MAX / 2
                        } else {
                            draw_length
                        };
                        let block_start_x = self.x_for_breath(draw_start);
                        for local_index in 0..graphic_length {
                            let breath = draw_start + local_index;
                            let x = block_start_x + local_index as i32;
                            if x > timeline_end {
                                break;
                            }
                            let (color, weight) =
                                self.property_block_interaction_style(property, block, breath);
                            let graphic = if block.is_blank {
                                property_blank_cell_graphic(
                                    block.interp_mode.as_deref(),
                                    block.ease_out_percent,
                                    block.ease_in_percent,
                                    graphic_length,
                                    local_index,
                                )
                            } else {
                                property_track_cell_graphic(
                                    block.is_blank,
                                    is_visible,
                                    graphic_length,
                                    local_index,
                                )
                            };
                            cells.push(Cell {
                                position: CellPoint { x, y, z: 0 },
                                graphic: CellGraphic::Glyph(graphic),
                                color,
                                weight,
                                ..Cell::default()
                            });
                        }
                    }
                    if let Some((block, (draw_start, draw_length))) = preview_block {
                        let block_start_x = self.x_for_breath(draw_start);
                        for local_index in 0..draw_length {
                            let breath = draw_start + local_index;
                            let x = block_start_x + local_index as i32;
                            if x > timeline_end {
                                break;
                            }
                            let (color, weight) =
                                self.property_block_interaction_style(property, block, breath);
                            let graphic = if block.is_blank {
                                property_blank_cell_graphic(
                                    block.interp_mode.as_deref(),
                                    block.ease_out_percent,
                                    block.ease_in_percent,
                                    draw_length,
                                    local_index,
                                )
                            } else {
                                property_track_cell_graphic(
                                    block.is_blank,
                                    is_visible,
                                    draw_length,
                                    local_index,
                                )
                            };
                            cells.push(Cell {
                                position: CellPoint { x, y, z: 0 },
                                graphic: CellGraphic::Glyph(graphic),
                                color,
                                weight,
                                ..Cell::default()
                            });
                        }
                    }
                    if let Some((start, end)) = self.property_bar_bounds(property) {
                        if state.current_breath >= start && state.current_breath <= end {
                            cells.push(Cell {
                                position: CellPoint {
                                    x: playhead_x,
                                    y,
                                    z: 0,
                                },
                                graphic: CellGraphic::Glyph('║'),
                                color: self.palette.get(UiColorRole::Vivid),
                                ..Cell::default()
                            });
                        }
                    }
                }
            }
        }

        CellGroup::from_cells(origin, cells).with_intake_behavior(CellGroupIntakeBehavior::Flat2d)
    }

    fn on_pointer_event(&mut self, event: ModulePointerEvent) {
        match event {
            ModulePointerEvent::Click { x, y, button } => {
                if let Some(outcome) = self.gizmo_state.handle_click(&self.gizmos, self.rect, x, y)
                {
                    if outcome == GizmoClickOutcome::Gizmo(GizmoKind::Close) {
                        self.hidden = true;
                    }
                    return;
                }
                let Some(row_kind) = self.row_at(x, y) else {
                    return;
                };
                // Clicking anywhere outside the renaming layer's name area
                // ends the rename (its commit/escape runs through the
                // key-capture seam instead).
                let local_x = x - self.rect.x0;
                let renaming_here = match (&self.rename, &row_kind) {
                    (Some(session), PanelRow::Layer(i)) => self
                        .state
                        .borrow()
                        .rows
                        .get(*i)
                        .is_some_and(|row| row.id == session.layer_id),
                    _ => false,
                };
                if self.rename.is_some() && !(renaming_here && local_x >= COL_NAME) {
                    self.rename = None;
                }
                match row_kind {
                    PanelRow::AutoKeyToggle => {
                        if button == ModulePointerButton::Left {
                            self.state
                                .borrow_mut()
                                .queue_action(LayersPanelAction::ToggleAutoKey);
                        }
                    }
                    PanelRow::BreathRuler => {
                        if button == ModulePointerButton::Left {
                            let local_x = x - self.rect.x0;
                            let breath = self.breath_at_x(local_x);
                            self.state
                                .borrow_mut()
                                .queue_action(LayersPanelAction::SetCurrentBreath(breath));
                            self.scrubbing_ruler = true;
                        }
                    }
                    PanelRow::LoopBar => {
                        let local_x = x - self.rect.x0;
                        let (timeline_start, _) = self.timeline_bounds();
                        // Left of the timeline: the PLAY and LOOP transport
                        // toggles that live next to the bar they drive.
                        if local_x < timeline_start {
                            if button == ModulePointerButton::Left {
                                if (1..=PLAY_BUTTON_END).contains(&local_x) {
                                    self.state
                                        .borrow_mut()
                                        .queue_action(LayersPanelAction::TogglePlay);
                                } else if (LOOP_BUTTON_START..=LOOP_BUTTON_END).contains(&local_x) {
                                    self.state
                                        .borrow_mut()
                                        .queue_action(LayersPanelAction::ToggleLoop);
                                }
                            }
                            return;
                        }
                        let Some(hit) = self.loop_window_hit_at(x, y) else {
                            return;
                        };
                        let (start, end) = self.loop_window_span();
                        let anchor = self.breath_at_x(local_x);
                        let mode = match hit {
                            LoopWindowHitMode::EdgeStart => LoopWindowDragMode::TrimStart,
                            LoopWindowHitMode::EdgeEnd => LoopWindowDragMode::TrimEnd,
                            // No swaps on this bar: both buttons move it.
                            LoopWindowHitMode::Body => LoopWindowDragMode::Move,
                        };
                        self.loop_window_drag = Some(LoopWindowDrag {
                            mode,
                            orig_start: start,
                            orig_end: end,
                            anchor_breath: anchor,
                            preview_start: start,
                            preview_end: end,
                        });
                    }
                    PanelRow::AddLayer => {
                        if button == ModulePointerButton::Left {
                            self.state
                                .borrow_mut()
                                .queue_action(LayersPanelAction::AddRequested);
                        }
                    }
                    PanelRow::Layer(row_index) => {
                        if button != ModulePointerButton::Left {
                            return;
                        }
                        let local_x = x - self.rect.x0;
                        let (timeline_start, _) = self.timeline_bounds();
                        let row = self.state.borrow().rows.get(row_index).cloned();
                        let Some(row) = row else {
                            return;
                        };
                        if local_x == COL_VISIBLE {
                            self.state
                                .borrow_mut()
                                .queue_action(LayersPanelAction::ToggleVisible(row.id));
                            return;
                        }
                        if local_x == COL_LOCK {
                            self.state
                                .borrow_mut()
                                .queue_action(LayersPanelAction::ToggleLocked(row.id));
                            return;
                        }
                        if local_x == COL_DELETE {
                            self.state
                                .borrow_mut()
                                .queue_action(LayersPanelAction::Delete(row.id));
                            return;
                        }
                        // The name area: the first click selects, a
                        // double-click starts an inline rename (J
                        // 2026-09-12).
                        if (COL_NAME..timeline_start - 1).contains(&local_x) {
                            // Shared click-timing seam: the first click
                            // selects, a double-click starts an inline
                            // rename (J 2026-09-12).
                            if self.name_double_click.note_click(row.id.clone(), Instant::now()) {
                                let mut field = TextEntryField::new(RENAME_MAX_CHARS, "name + enter");
                                field.focus();
                                self.rename = Some(RenameSession {
                                    layer_id: row.id.clone(),
                                    field,
                                });
                                return;
                            }
                        }
                        self.state
                            .borrow_mut()
                            .queue_action(LayersPanelAction::Select(row.id));
                    }
                    PanelRow::Property(property_index) => {
                        let property = self
                            .state
                            .borrow()
                            .property_rows
                            .get(property_index)
                            .cloned();
                        if let Some(hit) = self.property_block_hit_at(x, y) {
                            self.handle_property_block_click(hit, button);
                        } else if let Some(property) = property {
                            if button == ModulePointerButton::Left {
                                self.state.borrow_mut().queue_action(
                                    LayersPanelAction::SelectProperty(
                                        property.layer_id,
                                        property.property_id,
                                    ),
                                );
                            }
                        }
                    }
                }
            }
            ModulePointerEvent::Move { x, y } => {
                self.hover_local = Some((x - self.rect.x0, y - self.rect.y0));
                self.gizmo_state.note_pointer(&self.gizmos, self.rect, x, y);
                let next_hover = self.property_block_hit_at(x, y);
                if next_hover != self.hovered_property_block {
                    let mut state = self.state.borrow_mut();
                    match &next_hover {
                        Some(hit) => state.log_interaction(format!(
                            "hover {}/{} breath={} block={} {:?}/{:?}",
                            hit.layer_id,
                            hit.property_id,
                            hit.breath,
                            hit.block_id,
                            hit.piece,
                            hit.cell_type,
                        )),
                        None => state.log_interaction("hover cleared".into()),
                    }
                }
                self.hovered_property_block = next_hover;
                self.hovered_loop_window = self.loop_window_hit_at(x, y).is_some();
                self.hovered_playhead = matches!(self.row_at(x, y), Some(PanelRow::BreathRuler))
                    && (x - self.rect.x0) == self.x_for_breath(self.state.borrow().current_breath);
                if let Some(next_rect) = self.gizmo_state.drag_rect(x, y) {
                    self.rect = next_rect;
                }
                if self.scrubbing_ruler {
                    let local_x = x - self.rect.x0;
                    let breath = self.breath_at_x(local_x);
                    self.state
                        .borrow_mut()
                        .queue_action(LayersPanelAction::SetCurrentBreath(breath));
                }
                // Compute the breath up front: `breath_at_x` borrows all of
                // `self`, which conflicts with the mutable drag borrow below.
                let loop_current = self
                    .loop_window_drag
                    .as_ref()
                    .map(|_| (x - self.rect.x0, self.breath_at_x(x - self.rect.x0) as i64));
                if let (Some((_, current)), Some(drag)) =
                    (loop_current, self.loop_window_drag.as_mut())
                {
                    let delta = current - drag.anchor_breath as i64;
                    let (new_start, new_end) = match drag.mode {
                        LoopWindowDragMode::Move => {
                            let length = drag.orig_end - drag.orig_start;
                            let new_start = (drag.orig_start as i64 + delta).max(0) as u32;
                            (new_start, new_start + length)
                        }
                        LoopWindowDragMode::TrimStart => {
                            let new_start = (drag.orig_start as i64 + delta)
                                .clamp(0, drag.orig_end as i64)
                                as u32;
                            (new_start, drag.orig_end)
                        }
                        LoopWindowDragMode::TrimEnd => {
                            let new_end =
                                (drag.orig_end as i64 + delta).max(drag.orig_start as i64) as u32;
                            (drag.orig_start, new_end)
                        }
                    };
                    drag.preview_start = new_start;
                    drag.preview_end = new_end;
                }
                if let Some(drag) = &self.bar_drag {
                    let local_x = x - self.rect.x0;
                    let current_breath = self.breath_at_x(local_x) as i64;
                    let delta = current_breath - drag.anchor_breath as i64;
                    let (new_start, new_length) = match drag.mode {
                        BarDragMode::Move => {
                            let new_start = (drag.orig_start as i64 + delta).max(0) as u32;
                            (new_start, drag.orig_length)
                        }
                        BarDragMode::TrimStart => {
                            let max_start = drag.orig_start + drag.orig_length - 1;
                            let new_start =
                                (drag.orig_start as i64 + delta).clamp(0, max_start as i64) as u32;
                            let new_length = drag.orig_start + drag.orig_length - new_start;
                            (new_start, new_length)
                        }
                        BarDragMode::TrimEnd => {
                            let orig_end = drag.orig_start + drag.orig_length - 1;
                            let new_end =
                                (orig_end as i64 + delta).max(drag.orig_start as i64) as u32;
                            let new_length = new_end - drag.orig_start + 1;
                            (drag.orig_start, new_length)
                        }
                    };
                    self.state
                        .borrow_mut()
                        .queue_action(LayersPanelAction::SetLayerTiming(
                            drag.layer_id.clone(),
                            new_start,
                            new_length,
                        ));
                }
                // Compute the breath up front: `breath_at_x` borrows all of
                // `self`, which conflicts with the mutable drag borrow below.
                let current_breath = self.property_block_drag.as_ref().map(|_| {
                    let local_x = x - self.rect.x0;
                    self.breath_at_x(local_x) as i64
                });
                if let (Some(current_breath), Some(drag)) =
                    (current_breath, self.property_block_drag.as_mut())
                {
                    let delta = current_breath - drag.anchor_breath as i64;
                    if drag.mode == PropertyDragMode::Swap {
                        drag.swap_target_block_id = self
                            .hovered_property_block
                            .as_ref()
                            .filter(|hover| {
                                hover.layer_id == drag.layer_id
                                    && hover.property_id == drag.property_id
                                    && hover.block_id != drag.block_id
                            })
                            .map(|hover| hover.block_id.clone());
                    } else {
                        // The pushed/destructive rules live in
                        // `domain/painter-document/properties/` and are legal by
                        // construction, but the drag itself never mutates the document:
                        // it only tracks the requested span so the bar can preview at
                        // the pointer, and one commit action is queued on pointer-up.
                        // Per-frame destructive applies would chop every bar the
                        // pointer passed over, so nothing is written until release.
                        let (requested_start, requested_length) = match drag.mode {
                            PropertyDragMode::Move => {
                                let new_start = (drag.orig_start as i64 + delta).max(0) as u32;
                                (new_start, drag.orig_length)
                            }
                            PropertyDragMode::TrimStart => {
                                let max_start = drag.orig_start + drag.orig_length - 1;
                                let new_start = (drag.orig_start as i64 + delta)
                                    .clamp(0, max_start as i64)
                                    as u32;
                                let new_length = drag.orig_start + drag.orig_length - new_start;
                                (new_start, new_length)
                            }
                            PropertyDragMode::TrimEnd => {
                                let orig_end = drag.orig_start + drag.orig_length - 1;
                                let new_end =
                                    (orig_end as i64 + delta).max(drag.orig_start as i64) as u32;
                                let new_length = new_end - drag.orig_start + 1;
                                (drag.orig_start, new_length)
                            }
                            PropertyDragMode::DynamicResize => {
                                let orig_end = drag.orig_start + drag.orig_length - 1;
                                if current_breath < drag.anchor_breath as i64 {
                                    let new_start = (drag.orig_start as i64 + delta).max(0) as u32;
                                    (new_start, orig_end - new_start + 1)
                                } else {
                                    let new_end = (orig_end as i64 + delta)
                                        .max(drag.orig_start as i64)
                                        as u32;
                                    (drag.orig_start, new_end - drag.orig_start + 1)
                                }
                            }
                            PropertyDragMode::Swap => unreachable!(),
                        };
                        drag.last_requested_changed =
                            drag.last_requested != Some((requested_start, requested_length));
                        drag.last_requested = Some((requested_start, requested_length));
                        if drag.last_requested_changed {
                            self.state.borrow_mut().log_interaction(format!(
                                "drag {:?} block={} requested=({}, {})",
                                drag.mode, drag.block_id, requested_start, requested_length,
                            ));
                        }
                    }
                }
            }
            ModulePointerEvent::Up { x, y } => {
                self.gizmo_state.end_drag();
                self.scrubbing_ruler = false;
                self.bar_drag = None;
                if let Some(drag) = self.loop_window_drag.take() {
                    self.state
                        .borrow_mut()
                        .queue_action(LayersPanelAction::SetLoopWindow(
                            drag.preview_start,
                            drag.preview_end.max(drag.preview_start),
                        ));
                }
                if let Some(drag) = self.property_block_drag.take() {
                    if drag.mode == PropertyDragMode::Swap {
                        if let Some(target_block_id) = drag.swap_target_block_id {
                            self.state.borrow_mut().log_interaction(format!(
                                "commit swap {} -> {}",
                                drag.block_id, target_block_id,
                            ));
                            self.state.borrow_mut().queue_action(
                                LayersPanelAction::SwapPropertyBlocks(
                                    drag.layer_id,
                                    drag.property_id,
                                    drag.block_id,
                                    target_block_id,
                                ),
                            );
                        } else {
                            self.state.borrow_mut().log_interaction(
                                "commit swap dropped: released over nothing".into(),
                            );
                        }
                    } else if let Some((start, length)) = drag.last_requested {
                        // One commit per drag: the previewed span is applied exactly
                        // here, so destructive drags only yield the bars under the
                        // release position instead of everything crossed mid-flight.
                        self.state.borrow_mut().log_interaction(format!(
                            "commit {:?} block={} span=({}, {})",
                            drag.resolution, drag.block_id, start, length,
                        ));
                        let action = match drag.resolution {
                            PropertyTimingResolution::Pushed => {
                                LayersPanelAction::SetPropertyBlockTimingPushed(
                                    drag.layer_id,
                                    drag.property_id,
                                    drag.block_id,
                                    start,
                                    length,
                                )
                            }
                            PropertyTimingResolution::Destructive => {
                                LayersPanelAction::SetPropertyBlockTimingDestructive(
                                    drag.layer_id,
                                    drag.property_id,
                                    drag.block_id,
                                    start,
                                    length,
                                )
                            }
                        };
                        self.state.borrow_mut().queue_action(action);
                    }
                }
                self.hovered_property_block = self.property_block_hit_at(x, y);
            }
            ModulePointerEvent::Enter => self.gizmo_state.set_hovered(true),
            ModulePointerEvent::Leave => {
                self.gizmo_state.set_hovered(false);
                self.hovered_property_block = None;
                self.hover_local = None;
            }
            ModulePointerEvent::Down { .. } => {}
        }
    }

    /// Focused-layer-rename key entry through the registry's key-capture
    /// seam: keys are consumed only while a rename is active; otherwise
    /// every key falls through to normal binding dispatch.
    fn on_key_capture(&mut self, label: &str) -> bool {
        if self.hidden {
            return false;
        }
        let Some(session) = self.rename.as_mut() else {
            return false;
        };
        let consumed = session.field.handle_key_label(label);
        if let Some(name) = session.field.take_commit() {
            let session = self.rename.take().expect("rename session just checked");
            let name = name.trim().to_string();
            if !name.is_empty() {
                self.state
                    .borrow_mut()
                    .queue_action(LayersPanelAction::Rename(session.layer_id, name));
            }
        } else if label == "ESCAPE" {
            self.rename = None;
        }
        consumed
    }

    fn wants_pointer_capture(&self) -> bool {
        self.gizmo_state.wants_pointer_capture()
            || self.scrubbing_ruler
            || self.bar_drag.is_some()
            || self.property_block_drag.is_some()
            || self.loop_window_drag.is_some()
    }

    fn is_hidden(&self) -> bool {
        self.hidden
    }

    fn set_hidden(&mut self, hidden: bool) {
        self.hidden = hidden;
    }

    fn persisted_ui_state(&self) -> Option<PersistedModuleUiState> {
        Some(PersistedModuleUiState::new(
            self.id(),
            self.rect,
            self.gizmo_state.is_seamless(),
            self.hidden,
        ))
    }

    fn apply_persisted_ui_state(&mut self, state: &PersistedModuleUiState) {
        self.rect = state.rect.to_runtime();
        self.gizmo_state.set_seamless(state.is_seamless);
        self.hidden = state.is_hidden;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> ModuleRect {
        ModuleRect {
            x0: 0,
            y0: 0,
            x1: 50,
            y1: 12,
        }
    }

    fn row(id: &str, name: &str, start_breath: u32, length_breaths: u32) -> LayerRow {
        LayerRow {
            id: id.to_string(),
            name: name.to_string(),
            visible: true,
            locked: false,
            start_breath,
            length_breaths,
        }
    }

    fn property(
        layer_id: &str,
        property_id: &str,
        label: &str,
        kind: LayerPropertyKind,
        blocks: Vec<PropertyTrackBlock>,
    ) -> PropertyTrackRow {
        PropertyTrackRow {
            layer_id: layer_id.to_string(),
            property_id: property_id.to_string(),
            label: label.to_string(),
            kind,
            blocks,
        }
    }

    fn state_with_rows() -> Rc<RefCell<LayersPanelState>> {
        let state = Rc::new(RefCell::new(LayersPanelState::default()));
        state.borrow_mut().sync(
            vec![
                row("layer-1", "Layer 1", 0, 5),
                row("layer-2", "Layer 2", 2, 1),
            ],
            vec![
                property(
                    "layer-1",
                    "raster",
                    "RASTER",
                    LayerPropertyKind::Raster,
                    vec![PropertyTrackBlock {
                        id: "block-1".to_string(),
                        start_breath: 0,
                        length_breaths: 5,
                        is_blank: false,
                        interp_mode: None,
                        ease_out_percent: None,
                        ease_in_percent: None,
                    }],
                ),
                property("layer-1", "move", "MOVE", LayerPropertyKind::Move, vec![]),
            ],
            Some("layer-1".to_string()),
            Some("raster".to_string()),
            0,
            false,
            0,
            23,
            false,
            true,
        );
        state
    }

    #[test]
    fn clicking_a_layer_name_queues_a_select_action_for_that_layer() {
        let state = state_with_rows();
        let mut panel = LayersPanelModule::new("layers_panel", rect(), state.clone());

        panel.on_pointer_event(ModulePointerEvent::Click {
            x: COL_NAME,
            y: panel.row_y(4),
            button: ModulePointerButton::Left,
        });

        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(LayersPanelAction::Select("layer-1".to_string()))
        );
    }

    #[test]
    fn double_click_then_typing_enter_queues_a_rename() {
        let state = state_with_rows();
        let mut panel = LayersPanelModule::new("layers_panel", rect(), state.clone());
        let x = COL_NAME;
        let y = panel.row_y(4);

        panel.on_pointer_event(ModulePointerEvent::Click { x, y, button: ModulePointerButton::Left });
        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(LayersPanelAction::Select("layer-1".to_string()))
        );
        panel.on_pointer_event(ModulePointerEvent::Click { x, y, button: ModulePointerButton::Left });
        assert_eq!(state.borrow_mut().take_pending_action(), None, "second click starts rename, no action");

        assert!(panel.on_key_capture("J"));
        assert!(panel.on_key_capture("ENTER"));
        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(LayersPanelAction::Rename("layer-1".to_string(), "J".to_string()))
        );
    }

    #[test]
    fn layer_names_render_at_weight_1_at_rest_and_weight_2_when_highlighted() {
        let state = state_with_rows();
        let mut panel = LayersPanelModule::new("layers_panel", rect(), state.clone());
        let y = panel.row_y(4);

        // At rest the layer name reads at weight 1, never the invisible
        // weight 0 chrome.
        let group = panel.draw();
        let cell = group
            .cells
            .get(&CellPoint { x: COL_NAME, y, z: 0 })
            .unwrap();
        assert_eq!(cell.graphic, CellGraphic::Glyph('L'));
        assert_eq!(cell.weight, CellWeight::from_index_clamped(1));

        // Hovering the name region bumps it to weight 2, like the
        // char-button seam.
        panel.on_pointer_event(ModulePointerEvent::Move { x: COL_NAME, y });
        let group = panel.draw();
        let cell = group
            .cells
            .get(&CellPoint { x: COL_NAME, y, z: 0 })
            .unwrap();
        assert_eq!(cell.weight, CellWeight::from_index_clamped(2));

        // While the inline rename is live the edit text stays at weight 2.
        panel.on_pointer_event(ModulePointerEvent::Click { x: COL_NAME, y, button: ModulePointerButton::Left });
        panel.on_pointer_event(ModulePointerEvent::Click { x: COL_NAME, y, button: ModulePointerButton::Left });
        assert!(panel.on_key_capture("J"));
        let group = panel.draw();
        let cell = group
            .cells
            .get(&CellPoint { x: COL_NAME, y, z: 0 })
            .unwrap();
        assert_eq!(cell.graphic, CellGraphic::Glyph('J'));
        assert_eq!(cell.weight, CellWeight::from_index_clamped(2));
    }

    #[test]
    fn clicking_a_property_row_queues_a_select_property_action() {
        let state = state_with_rows();
        let mut panel = LayersPanelModule::new("layers_panel", rect(), state.clone());

        panel.on_pointer_event(ModulePointerEvent::Click {
            x: COL_NAME,
            y: panel.row_y(5),
            button: ModulePointerButton::Left,
        });

        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(LayersPanelAction::SelectProperty(
                "layer-1".to_string(),
                "raster".to_string(),
            ))
        );
    }

    #[test]
    fn drawing_the_selected_layer_renders_property_rows_beneath_it() {
        let state = state_with_rows();
        let panel = LayersPanelModule::new("layers_panel", rect(), state);
        let group = panel.draw();
        let (timeline_start, _) = panel.timeline_bounds();

        assert_eq!(
            group
                .cells
                .get(&CellPoint {
                    x: COL_NAME,
                    y: panel.row_y(5),
                    z: 0,
                })
                .map(|cell| cell.graphic.clone()),
            Some(CellGraphic::Glyph('R'))
        );
        assert_eq!(
            group
                .cells
                .get(&CellPoint {
                    x: timeline_start,
                    y: panel.row_y(5),
                    z: 0,
                })
                .map(|cell| cell.graphic.clone()),
            Some(CellGraphic::Glyph('║'))
        );
    }

    #[test]
    fn raster_property_blocks_use_solid_endcaps_and_new_empty_graphics() {
        assert_eq!(property_track_cell_graphic(false, true, 1, 0), '█');
        assert_eq!(property_track_cell_graphic(false, true, 2, 0), '█');
        assert_eq!(property_track_cell_graphic(false, true, 2, 1), '▦');
        assert_eq!(property_track_cell_graphic(false, true, 4, 1), '▥');
        // J's 2026-09-07 empty graphics: ◧ left empty head, ◨ right empty head,
        // ▪ empty center + empty single.
        assert_eq!(property_track_cell_graphic(true, true, 1, 0), '▪');
        assert_eq!(property_track_cell_graphic(true, true, 2, 0), '◧');
        assert_eq!(property_track_cell_graphic(true, true, 2, 1), '◨');
        assert_eq!(property_track_cell_graphic(true, true, 4, 1), '▪');
    }

    #[test]
    fn hidden_property_blocks_use_old_system_hidden_caps_and_midsections() {
        assert_eq!(property_track_cell_graphic(false, false, 1, 0), '▒');
        assert_eq!(property_track_cell_graphic(false, false, 2, 0), '╺');
        assert_eq!(property_track_cell_graphic(false, false, 2, 1), '╸');
        assert_eq!(property_track_cell_graphic(false, false, 4, 1), '╌');
    }

    #[test]
    fn hovering_a_raster_block_highlights_it_with_vivid_color_and_weight() {
        let state = state_with_rows();
        let mut panel = LayersPanelModule::new("layers_panel", rect(), state);
        let (timeline_start, _) = panel.timeline_bounds();

        panel.on_pointer_event(ModulePointerEvent::Move {
            x: timeline_start + 1,
            y: panel.row_y(5),
        });
        let group = panel.draw();
        let cell = group
            .cells
            .get(&CellPoint {
                x: timeline_start + 1,
                y: panel.row_y(5),
                z: 0,
            })
            .unwrap();

        assert_eq!(cell.color, UiPalette::default().get(UiColorRole::Vivid));
        assert_eq!(
            cell.weight,
            thaum_renderer_domain::CellWeight::from_index_clamped(3)
        );
    }

    #[test]
    fn left_click_dragging_an_edge_previews_then_commits_the_pushed_timing_on_release() {
        let state = state_with_rows();
        let mut panel = LayersPanelModule::new("layers_panel", rect(), state.clone());
        let (timeline_start, _) = panel.timeline_bounds();
        let y = panel.row_y(5);

        panel.on_pointer_event(ModulePointerEvent::Click {
            x: timeline_start,
            y,
            button: ModulePointerButton::Left,
        });
        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(LayersPanelAction::SelectProperty(
                "layer-1".to_string(),
                "raster".to_string(),
            ))
        );
        assert!(panel.wants_pointer_capture());

        // Mid-drag frames only preview: the document is untouched, so no action is
        // queued until release.
        panel.on_pointer_event(ModulePointerEvent::Move {
            x: timeline_start + 2,
            y,
        });
        assert_eq!(state.borrow_mut().take_pending_action(), None);

        panel.on_pointer_event(ModulePointerEvent::Up {
            x: timeline_start + 2,
            y,
        });
        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(LayersPanelAction::SetPropertyBlockTimingPushed(
                "layer-1".to_string(),
                "raster".to_string(),
                "block-1".to_string(),
                2,
                3,
            ))
        );
        assert!(!panel.wants_pointer_capture());
    }

    fn state_with_two_gapped_content_blocks() -> Rc<RefCell<LayersPanelState>> {
        let state = Rc::new(RefCell::new(LayersPanelState::default()));
        state.borrow_mut().sync(
            vec![row("layer-1", "Layer 1", 0, 20)],
            vec![property(
                "layer-1",
                "raster",
                "RASTER",
                LayerPropertyKind::Raster,
                vec![
                    PropertyTrackBlock {
                        id: "block-1".to_string(),
                        start_breath: 0,
                        length_breaths: 5,
                        is_blank: false,
                        interp_mode: None,
                        ease_out_percent: None,
                        ease_in_percent: None,
                    },
                    PropertyTrackBlock {
                        id: "block-2".to_string(),
                        start_breath: 10,
                        length_breaths: 5,
                        is_blank: false,
                        interp_mode: None,
                        ease_out_percent: None,
                        ease_in_percent: None,
                    },
                ],
            )],
            Some("layer-1".to_string()),
            Some("raster".to_string()),
            0,
            false,
            0,
            23,
            false,
            true,
        );
        state
    }

    #[test]
    fn right_click_dragging_the_body_destructively_moves_and_commits_once_on_release() {
        let state = state_with_two_gapped_content_blocks();
        let mut panel = LayersPanelModule::new("layers_panel", rect(), state.clone());
        let (timeline_start, _) = panel.timeline_bounds();
        let y = panel.row_y(5);

        panel.on_pointer_event(ModulePointerEvent::Click {
            x: timeline_start + 2,
            y,
            button: ModulePointerButton::Right,
        });
        state.borrow_mut().take_pending_action();

        // Drag block-1 over block-2: mid-drag frames stay preview-only so nothing is
        // chopped until the release commit resolves the covered victim.
        panel.on_pointer_event(ModulePointerEvent::Move {
            x: timeline_start + 12,
            y,
        });
        assert_eq!(state.borrow_mut().take_pending_action(), None);

        panel.on_pointer_event(ModulePointerEvent::Up {
            x: timeline_start + 12,
            y,
        });
        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(LayersPanelAction::SetPropertyBlockTimingDestructive(
                "layer-1".to_string(),
                "raster".to_string(),
                "block-1".to_string(),
                10,
                5,
            ))
        );
    }

    #[test]
    fn double_left_clicking_a_solid_center_splits_it_at_the_clicked_breath() {
        let state = state_with_rows();
        let mut panel = LayersPanelModule::new("layers_panel", rect(), state.clone());
        let (timeline_start, _) = panel.timeline_bounds();
        let y = panel.row_y(5);

        panel.on_pointer_event(ModulePointerEvent::Click {
            x: timeline_start + 2,
            y,
            button: ModulePointerButton::Left,
        });
        state.borrow_mut().take_pending_action();
        panel.on_pointer_event(ModulePointerEvent::Click {
            x: timeline_start + 2,
            y,
            button: ModulePointerButton::Left,
        });

        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(LayersPanelAction::SplitPropertyBlock(
                "layer-1".to_string(),
                "raster".to_string(),
                "block-1".to_string(),
                2,
            ))
        );
    }

    #[test]
    fn double_left_clicking_a_solid_single_duplicates_it_too() {
        let state = state_with_two_single_breath_blocks();
        let mut panel = LayersPanelModule::new("layers_panel", rect(), state.clone());
        let (timeline_start, _) = panel.timeline_bounds();
        let y = panel.row_y(5);

        panel.on_pointer_event(ModulePointerEvent::Click {
            x: timeline_start + 3,
            y,
            button: ModulePointerButton::Left,
        });
        state.borrow_mut().take_pending_action();
        panel.on_pointer_event(ModulePointerEvent::Click {
            x: timeline_start + 3,
            y,
            button: ModulePointerButton::Left,
        });

        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(LayersPanelAction::DuplicatePropertyBlock(
                "layer-1".to_string(),
                "raster".to_string(),
                "block-1".to_string(),
            ))
        );
    }

    #[test]
    fn right_double_clicking_a_solid_left_head_merges_the_bar_on_the_left_into_it() {
        let state = state_with_two_adjacent_content_blocks();
        let mut panel = LayersPanelModule::new("layers_panel", rect(), state.clone());
        let (timeline_start, _) = panel.timeline_bounds();
        let y = panel.row_y(5);

        // block-2 starts at breath 5: its left head. Double-right merges block-1
        // into it as a destructive resize spanning block-1's start through
        // block-2's end (J-flip 2026-09-07: merges live on the right button).
        panel.on_pointer_event(ModulePointerEvent::Click {
            x: timeline_start + 5,
            y,
            button: ModulePointerButton::Right,
        });
        state.borrow_mut().take_pending_action();
        panel.on_pointer_event(ModulePointerEvent::Click {
            x: timeline_start + 5,
            y,
            button: ModulePointerButton::Right,
        });

        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(LayersPanelAction::SetPropertyBlockTimingDestructive(
                "layer-1".to_string(),
                "raster".to_string(),
                "block-2".to_string(),
                0,
                10,
            ))
        );
    }

    #[test]
    fn right_double_clicking_a_solid_right_head_merges_the_bar_on_the_right_into_it() {
        let state = state_with_two_adjacent_content_blocks();
        let mut panel = LayersPanelModule::new("layers_panel", rect(), state.clone());
        let (timeline_start, _) = panel.timeline_bounds();
        let y = panel.row_y(5);

        // Mirror of the left-head merge: block-1's right head (breath 4) merges
        // block-2 in destructively through block-2's end.
        panel.on_pointer_event(ModulePointerEvent::Click {
            x: timeline_start + 4,
            y,
            button: ModulePointerButton::Right,
        });
        state.borrow_mut().take_pending_action();
        panel.on_pointer_event(ModulePointerEvent::Click {
            x: timeline_start + 4,
            y,
            button: ModulePointerButton::Right,
        });

        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(LayersPanelAction::SetPropertyBlockTimingDestructive(
                "layer-1".to_string(),
                "raster".to_string(),
                "block-1".to_string(),
                0,
                10,
            ))
        );
    }

    #[test]
    fn right_double_clicking_a_solid_left_head_at_the_span_edge_rejects() {
        let state = state_with_two_adjacent_content_blocks();
        let mut panel = LayersPanelModule::new("layers_panel", rect(), state.clone());
        let (timeline_start, _) = panel.timeline_bounds();
        let y = panel.row_y(5);

        // block-1 starts at the layer span start: no left neighbor, so the merge
        // interaction is rejected.
        panel.on_pointer_event(ModulePointerEvent::Click {
            x: timeline_start,
            y,
            button: ModulePointerButton::Right,
        });
        state.borrow_mut().take_pending_action();
        panel.on_pointer_event(ModulePointerEvent::Click {
            x: timeline_start,
            y,
            button: ModulePointerButton::Right,
        });

        assert_eq!(state.borrow_mut().take_pending_action(), None);
    }

    #[test]
    fn right_double_clicking_a_solid_center_blanks_its_span() {
        let state = state_with_rows();
        let mut panel = LayersPanelModule::new("layers_panel", rect(), state.clone());
        let (timeline_start, _) = panel.timeline_bounds();
        let y = panel.row_y(5);

        panel.on_pointer_event(ModulePointerEvent::Click {
            x: timeline_start + 2,
            y,
            button: ModulePointerButton::Right,
        });
        state.borrow_mut().take_pending_action();
        panel.on_pointer_event(ModulePointerEvent::Click {
            x: timeline_start + 2,
            y,
            button: ModulePointerButton::Right,
        });

        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(LayersPanelAction::BlankPropertyBlock(
                "layer-1".to_string(),
                "raster".to_string(),
                "block-1".to_string(),
            ))
        );
    }

    #[test]
    fn left_double_clicking_a_solid_head_is_unused() {
        let state = state_with_two_adjacent_content_blocks();
        let mut panel = LayersPanelModule::new("layers_panel", rect(), state.clone());
        let (timeline_start, _) = panel.timeline_bounds();
        let y = panel.row_y(5);

        // J-flip 2026-09-07: head merges moved to double-right; double-left on a
        // content end is reserved for keyframe interpolation later.
        panel.on_pointer_event(ModulePointerEvent::Click {
            x: timeline_start + 5,
            y,
            button: ModulePointerButton::Left,
        });
        state.borrow_mut().take_pending_action();
        panel.on_pointer_event(ModulePointerEvent::Click {
            x: timeline_start + 5,
            y,
            button: ModulePointerButton::Left,
        });

        assert_eq!(state.borrow_mut().take_pending_action(), None);
    }

    fn state_with_a_blank_between_two_content_blocks() -> Rc<RefCell<LayersPanelState>> {
        let state = Rc::new(RefCell::new(LayersPanelState::default()));
        state.borrow_mut().sync(
            vec![row("layer-1", "Layer 1", 0, 11)],
            vec![property(
                "layer-1",
                "raster",
                "RASTER",
                LayerPropertyKind::Raster,
                vec![
                    PropertyTrackBlock {
                        id: "block-1".to_string(),
                        start_breath: 0,
                        length_breaths: 3,
                        is_blank: false,
                        interp_mode: None,
                        ease_out_percent: None,
                        ease_in_percent: None,
                    },
                    PropertyTrackBlock {
                        id: "block-2".to_string(),
                        start_breath: 3,
                        length_breaths: 5,
                        is_blank: true,
                        interp_mode: None,
                        ease_out_percent: None,
                        ease_in_percent: None,
                    },
                    PropertyTrackBlock {
                        id: "block-3".to_string(),
                        start_breath: 8,
                        length_breaths: 3,
                        is_blank: false,
                        interp_mode: None,
                        ease_out_percent: None,
                        ease_in_percent: None,
                    },
                ],
            )],
            Some("layer-1".to_string()),
            Some("raster".to_string()),
            0,
            false,
            0,
            23,
            false,
            true,
        );
        state
    }

    #[test]
    fn left_click_dragging_an_empty_center_swaps_keyframes_like_a_solid_center() {
        let state = state_with_a_blank_between_two_content_blocks();
        let mut panel = LayersPanelModule::new("layers_panel", rect(), state.clone());
        let (timeline_start, _) = panel.timeline_bounds();
        let y = panel.row_y(5);

        // Empty center L-drag swaps keyframe content exactly like a solid center —
        // predictability (J 2026-09-07). The old scrub-on-blank behavior is gone.
        panel.on_pointer_event(ModulePointerEvent::Click {
            x: timeline_start + 5,
            y,
            button: ModulePointerButton::Left,
        });
        assert!(matches!(
            state.borrow_mut().take_pending_action(),
            Some(LayersPanelAction::SelectProperty(..))
        ));
        assert!(panel.wants_pointer_capture());

        panel.on_pointer_event(ModulePointerEvent::Move {
            x: timeline_start + 1,
            y,
        });
        assert_eq!(state.borrow_mut().take_pending_action(), None);

        panel.on_pointer_event(ModulePointerEvent::Up {
            x: timeline_start + 1,
            y,
        });
        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(LayersPanelAction::SwapPropertyBlocks(
                "layer-1".to_string(),
                "raster".to_string(),
                "block-2".to_string(),
                "block-1".to_string(),
            ))
        );
    }

    #[test]
    fn left_clicking_an_empty_single_and_right_clicking_an_empty_center_start_no_drag() {
        let state = state_with_a_blank_between_two_content_blocks();
        let mut panel = LayersPanelModule::new("layers_panel", rect(), state.clone());
        let (timeline_start, _) = panel.timeline_bounds();
        let y = panel.row_y(5);

        // Empty center: right click is unused beyond selection.
        panel.on_pointer_event(ModulePointerEvent::Click {
            x: timeline_start + 5,
            y,
            button: ModulePointerButton::Right,
        });
        panel.on_pointer_event(ModulePointerEvent::Up {
            x: timeline_start + 5,
            y,
        });
        assert!(matches!(
            state.borrow_mut().take_pending_action(),
            Some(LayersPanelAction::SelectProperty(..))
        ));
        assert!(!panel.wants_pointer_capture());

        // Empty single: left click is unused beyond selection too.
        let state = state_with_solid_and_empty_singles();
        let mut panel = LayersPanelModule::new("layers_panel", rect(), state.clone());
        let (timeline_start, _) = panel.timeline_bounds();
        let y = panel.row_y(5);
        panel.on_pointer_event(ModulePointerEvent::Click {
            x: timeline_start + 6,
            y,
            button: ModulePointerButton::Left,
        });
        panel.on_pointer_event(ModulePointerEvent::Up {
            x: timeline_start + 6,
            y,
        });
        assert!(matches!(
            state.borrow_mut().take_pending_action(),
            Some(LayersPanelAction::SelectProperty(..))
        ));
        assert!(!panel.wants_pointer_capture());
    }

    #[test]
    fn the_trailing_blank_renders_past_the_layer_span_to_the_panel_edge() {
        let state = state_with_content_then_trailing_blank();
        let mut panel = LayersPanelModule::new("layers_panel", rect(), state.clone());
        let (timeline_start, _) = panel.timeline_bounds();
        let y = panel.row_y(5);

        // Layer span is 0..10 but the panel is 50 wide: past the span the infinite
        // trailing blank keeps rendering as blank cells, never as ruler dots, and
        // never shows a right endcap — it has no end.
        let group = panel.draw();
        for breath in [10u32, 15, 20, 25, 29] {
            let cell = group
                .cells
                .get(&CellPoint {
                    x: timeline_start + breath as i32,
                    y,
                    z: 0,
                })
                .unwrap();
            assert_eq!(cell.graphic, CellGraphic::Glyph('▣'), "breath {breath}");
        }
    }

    #[test]
    fn clicking_past_the_layer_span_hits_the_trailing_blank_as_an_empty_center() {
        let state = state_with_content_then_trailing_blank();
        let mut panel = LayersPanelModule::new("layers_panel", rect(), state.clone());
        let (timeline_start, _) = panel.timeline_bounds();
        let y = panel.row_y(5);

        // Breath 30 sits well past the 10-breath layer span: the infinite trailing
        // blank region hit-tests as that blank, center piece, empty cell type.
        let hit = panel.property_block_hit_at(timeline_start + 30, y).unwrap();
        assert_eq!(hit.block_id, "block-2");
        assert_eq!(hit.piece, BarPiece::Center);
        assert_eq!(hit.cell_type, CellType::Empty);

        // And the merge interaction works out there like on any empty center.
        panel.on_pointer_event(ModulePointerEvent::Click {
            x: timeline_start + 30,
            y,
            button: ModulePointerButton::Right,
        });
        state.borrow_mut().take_pending_action();
        panel.on_pointer_event(ModulePointerEvent::Click {
            x: timeline_start + 30,
            y,
            button: ModulePointerButton::Right,
        });
        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(LayersPanelAction::MergeEmptyPropertyBlock(
                "layer-1".to_string(),
                "raster".to_string(),
                "block-2".to_string(),
                true,
            ))
        );
    }

    #[test]
    fn right_double_clicking_an_empty_center_merges_it_into_the_left_content_block() {
        let state = state_with_a_blank_between_two_content_blocks();
        let mut panel = LayersPanelModule::new("layers_panel", rect(), state.clone());
        let (timeline_start, _) = panel.timeline_bounds();
        let y = panel.row_y(5);

        panel.on_pointer_event(ModulePointerEvent::Click {
            x: timeline_start + 5,
            y,
            button: ModulePointerButton::Right,
        });
        state.borrow_mut().take_pending_action();
        panel.on_pointer_event(ModulePointerEvent::Click {
            x: timeline_start + 5,
            y,
            button: ModulePointerButton::Right,
        });

        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(LayersPanelAction::MergeEmptyPropertyBlock(
                "layer-1".to_string(),
                "raster".to_string(),
                "block-2".to_string(),
                true,
            ))
        );
    }

    #[test]
    fn right_double_clicking_an_empty_right_head_prefers_the_right_side() {
        let state = state_with_content_then_trailing_blank();
        let mut panel = LayersPanelModule::new("layers_panel", rect(), state.clone());
        let (timeline_start, _) = panel.timeline_bounds();
        let y = panel.row_y(5);

        // The mirror: a right-head empty merges into the block on its right first.
        panel.on_pointer_event(ModulePointerEvent::Click {
            x: timeline_start + 9,
            y,
            button: ModulePointerButton::Right,
        });
        state.borrow_mut().take_pending_action();
        panel.on_pointer_event(ModulePointerEvent::Click {
            x: timeline_start + 9,
            y,
            button: ModulePointerButton::Right,
        });

        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(LayersPanelAction::MergeEmptyPropertyBlock(
                "layer-1".to_string(),
                "raster".to_string(),
                "block-2".to_string(),
                false,
            ))
        );
    }

    #[test]
    fn left_double_clicking_an_empty_center_cycles_its_interpolation_mode() {
        let state = state_with_a_blank_between_two_content_blocks();
        let mut panel = LayersPanelModule::new("layers_panel", rect(), state.clone());
        let (timeline_start, _) = panel.timeline_bounds();
        let y = panel.row_y(5);

        // The keyframing surface (J 2026-09-07): double-left on an empty center
        // queues the mode cycle — storage owns the interpolate → hold → loop
        // order and clears the ease ends on non-adjustable modes.
        panel.on_pointer_event(ModulePointerEvent::Click {
            x: timeline_start + 5,
            y,
            button: ModulePointerButton::Left,
        });
        state.borrow_mut().take_pending_action();
        panel.on_pointer_event(ModulePointerEvent::Click {
            x: timeline_start + 5,
            y,
            button: ModulePointerButton::Left,
        });

        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(LayersPanelAction::CycleEmptyInterpMode(
                "layer-1".to_string(),
                "raster".to_string(),
                "block-2".to_string(),
            ))
        );
    }

    #[test]
    fn left_double_clicking_an_empty_left_head_cycles_ease_out() {
        let state = state_with_a_blank_between_two_content_blocks();
        let mut panel = LayersPanelModule::new("layers_panel", rect(), state.clone());
        let (timeline_start, _) = panel.timeline_bounds();
        let y = panel.row_y(5);

        // The left end of an empty is the ease-out dial (J 2026-09-07).
        panel.on_pointer_event(ModulePointerEvent::Click {
            x: timeline_start + 3,
            y,
            button: ModulePointerButton::Left,
        });
        state.borrow_mut().take_pending_action();
        panel.on_pointer_event(ModulePointerEvent::Click {
            x: timeline_start + 3,
            y,
            button: ModulePointerButton::Left,
        });

        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(LayersPanelAction::CycleEmptyEaseOut(
                "layer-1".to_string(),
                "raster".to_string(),
                "block-2".to_string(),
            ))
        );
    }

    #[test]
    fn left_double_clicking_an_empty_right_head_cycles_ease_in() {
        let state = state_with_content_then_trailing_blank();
        let mut panel = LayersPanelModule::new("layers_panel", rect(), state.clone());
        let (timeline_start, _) = panel.timeline_bounds();
        let y = panel.row_y(5);

        // The mirror: the right end of an empty is the ease-in dial.
        panel.on_pointer_event(ModulePointerEvent::Click {
            x: timeline_start + 9,
            y,
            button: ModulePointerButton::Left,
        });
        state.borrow_mut().take_pending_action();
        panel.on_pointer_event(ModulePointerEvent::Click {
            x: timeline_start + 9,
            y,
            button: ModulePointerButton::Left,
        });

        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(LayersPanelAction::CycleEmptyEaseIn(
                "layer-1".to_string(),
                "raster".to_string(),
                "block-2".to_string(),
            ))
        );
    }

    #[test]
    fn right_click_dragging_an_empty_single_resizes_destructively() {
        let state = state_with_solid_and_empty_singles();
        let mut panel = LayersPanelModule::new("layers_panel", rect(), state.clone());
        let (timeline_start, _) = panel.timeline_bounds();
        let y = panel.row_y(5);

        // block-2 (breath 6) is an empty single: right drag grows it destructively,
        // overwriting whatever it expands over.
        panel.on_pointer_event(ModulePointerEvent::Click {
            x: timeline_start + 6,
            y,
            button: ModulePointerButton::Right,
        });
        state.borrow_mut().take_pending_action();
        panel.on_pointer_event(ModulePointerEvent::Move {
            x: timeline_start + 8,
            y,
        });
        panel.on_pointer_event(ModulePointerEvent::Up {
            x: timeline_start + 8,
            y,
        });
        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(LayersPanelAction::SetPropertyBlockTimingDestructive(
                "layer-1".to_string(),
                "raster".to_string(),
                "block-2".to_string(),
                6,
                3,
            ))
        );
    }

    #[test]
    fn right_double_clicking_an_empty_single_merges_into_the_left_content_block() {
        let state = state_with_solid_and_empty_singles();
        let mut panel = LayersPanelModule::new("layers_panel", rect(), state.clone());
        let (timeline_start, _) = panel.timeline_bounds();
        let y = panel.row_y(5);

        panel.on_pointer_event(ModulePointerEvent::Click {
            x: timeline_start + 6,
            y,
            button: ModulePointerButton::Right,
        });
        state.borrow_mut().take_pending_action();
        panel.on_pointer_event(ModulePointerEvent::Click {
            x: timeline_start + 6,
            y,
            button: ModulePointerButton::Right,
        });

        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(LayersPanelAction::MergeEmptyPropertyBlock(
                "layer-1".to_string(),
                "raster".to_string(),
                "block-2".to_string(),
                true,
            ))
        );
    }

    #[test]
    fn left_click_dragging_an_empty_left_head_previews_the_pushed_timing() {
        let state = state_with_content_then_trailing_blank();
        let mut panel = LayersPanelModule::new("layers_panel", rect(), state.clone());
        let (timeline_start, _) = panel.timeline_bounds();
        let y = panel.row_y(5);

        // Empty heads drag exactly like solid heads: pushed on left, destructive on
        // right (same code — J 2026-09-07).
        panel.on_pointer_event(ModulePointerEvent::Click {
            x: timeline_start + 5,
            y,
            button: ModulePointerButton::Left,
        });
        state.borrow_mut().take_pending_action();
        panel.on_pointer_event(ModulePointerEvent::Move {
            x: timeline_start + 7,
            y,
        });
        assert_eq!(state.borrow_mut().take_pending_action(), None);
        panel.on_pointer_event(ModulePointerEvent::Up {
            x: timeline_start + 7,
            y,
        });
        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(LayersPanelAction::SetPropertyBlockTimingPushed(
                "layer-1".to_string(),
                "raster".to_string(),
                "block-2".to_string(),
                7,
                3,
            ))
        );
    }

    fn state_with_solid_and_empty_singles() -> Rc<RefCell<LayersPanelState>> {
        let state = Rc::new(RefCell::new(LayersPanelState::default()));
        state.borrow_mut().sync(
            vec![row("layer-1", "Layer 1", 0, 10)],
            vec![property(
                "layer-1",
                "raster",
                "RASTER",
                LayerPropertyKind::Raster,
                vec![
                    PropertyTrackBlock {
                        id: "block-1".to_string(),
                        start_breath: 3,
                        length_breaths: 1,
                        is_blank: false,
                        interp_mode: None,
                        ease_out_percent: None,
                        ease_in_percent: None,
                    },
                    PropertyTrackBlock {
                        id: "block-2".to_string(),
                        start_breath: 6,
                        length_breaths: 1,
                        is_blank: true,
                        interp_mode: None,
                        ease_out_percent: None,
                        ease_in_percent: None,
                    },
                ],
            )],
            Some("layer-1".to_string()),
            Some("raster".to_string()),
            0,
            false,
            0,
            23,
            false,
            true,
        );
        state
    }

    fn state_with_content_then_trailing_blank() -> Rc<RefCell<LayersPanelState>> {
        let state = Rc::new(RefCell::new(LayersPanelState::default()));
        state.borrow_mut().sync(
            vec![row("layer-1", "Layer 1", 0, 10)],
            vec![property(
                "layer-1",
                "raster",
                "RASTER",
                LayerPropertyKind::Raster,
                vec![
                    PropertyTrackBlock {
                        id: "block-1".to_string(),
                        start_breath: 0,
                        length_breaths: 5,
                        is_blank: false,
                        interp_mode: None,
                        ease_out_percent: None,
                        ease_in_percent: None,
                    },
                    PropertyTrackBlock {
                        id: "block-2".to_string(),
                        start_breath: 5,
                        length_breaths: 5,
                        is_blank: true,
                        interp_mode: None,
                        ease_out_percent: None,
                        ease_in_percent: None,
                    },
                ],
            )],
            Some("layer-1".to_string()),
            Some("raster".to_string()),
            0,
            false,
            0,
            23,
            false,
            true,
        );
        state
    }

    fn state_with_two_adjacent_content_blocks() -> Rc<RefCell<LayersPanelState>> {
        let state = Rc::new(RefCell::new(LayersPanelState::default()));
        state.borrow_mut().sync(
            vec![row("layer-1", "Layer 1", 0, 10)],
            vec![property(
                "layer-1",
                "raster",
                "RASTER",
                LayerPropertyKind::Raster,
                vec![
                    PropertyTrackBlock {
                        id: "block-1".to_string(),
                        start_breath: 0,
                        length_breaths: 5,
                        is_blank: false,
                        interp_mode: None,
                        ease_out_percent: None,
                        ease_in_percent: None,
                    },
                    PropertyTrackBlock {
                        id: "block-2".to_string(),
                        start_breath: 5,
                        length_breaths: 5,
                        is_blank: false,
                        interp_mode: None,
                        ease_out_percent: None,
                        ease_in_percent: None,
                    },
                ],
            )],
            Some("layer-1".to_string()),
            Some("raster".to_string()),
            0,
            false,
            0,
            23,
            false,
            true,
        );
        state
    }

    #[test]
    fn left_click_dragging_the_body_previews_and_commits_a_swap_with_the_hovered_block() {
        let state = state_with_two_adjacent_content_blocks();
        let mut panel = LayersPanelModule::new("layers_panel", rect(), state.clone());
        let (timeline_start, _) = panel.timeline_bounds();
        let y = panel.row_y(5);

        panel.on_pointer_event(ModulePointerEvent::Click {
            x: timeline_start + 2,
            y,
            button: ModulePointerButton::Left,
        });
        state.borrow_mut().take_pending_action();
        assert!(panel.wants_pointer_capture());

        panel.on_pointer_event(ModulePointerEvent::Move {
            x: timeline_start + 7,
            y,
        });
        assert_eq!(state.borrow_mut().take_pending_action(), None);

        panel.on_pointer_event(ModulePointerEvent::Up {
            x: timeline_start + 7,
            y,
        });
        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(LayersPanelAction::SwapPropertyBlocks(
                "layer-1".to_string(),
                "raster".to_string(),
                "block-1".to_string(),
                "block-2".to_string(),
            ))
        );
        assert!(!panel.wants_pointer_capture());
    }

    #[test]
    fn right_click_dragging_a_single_bar_resizes_destructively_in_both_directions() {
        let state = state_with_two_single_breath_blocks();
        let mut panel = LayersPanelModule::new("layers_panel", rect(), state.clone());
        let (timeline_start, _) = panel.timeline_bounds();
        let y = panel.row_y(5);

        // Right-drag block-1 (breath 3) leftward: it grows left destructively.
        panel.on_pointer_event(ModulePointerEvent::Click {
            x: timeline_start + 3,
            y,
            button: ModulePointerButton::Right,
        });
        state.borrow_mut().take_pending_action();
        panel.on_pointer_event(ModulePointerEvent::Move {
            x: timeline_start + 1,
            y,
        });
        assert_eq!(state.borrow_mut().take_pending_action(), None);
        panel.on_pointer_event(ModulePointerEvent::Up {
            x: timeline_start + 1,
            y,
        });
        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(LayersPanelAction::SetPropertyBlockTimingDestructive(
                "layer-1".to_string(),
                "raster".to_string(),
                "block-1".to_string(),
                1,
                3,
            ))
        );

        // Right-drag block-2 (breath 6) rightward: it grows right destructively.
        panel.on_pointer_event(ModulePointerEvent::Click {
            x: timeline_start + 6,
            y,
            button: ModulePointerButton::Right,
        });
        state.borrow_mut().take_pending_action();
        panel.on_pointer_event(ModulePointerEvent::Move {
            x: timeline_start + 8,
            y,
        });
        panel.on_pointer_event(ModulePointerEvent::Up {
            x: timeline_start + 8,
            y,
        });
        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(LayersPanelAction::SetPropertyBlockTimingDestructive(
                "layer-1".to_string(),
                "raster".to_string(),
                "block-2".to_string(),
                6,
                3,
            ))
        );
    }

    #[test]
    fn left_click_dragging_a_single_bar_destructively_moves_and_commits_once_on_release() {
        let state = state_with_two_single_breath_blocks();
        let mut panel = LayersPanelModule::new("layers_panel", rect(), state.clone());
        let (timeline_start, _) = panel.timeline_bounds();
        let y = panel.row_y(5);

        panel.on_pointer_event(ModulePointerEvent::Click {
            x: timeline_start + 3,
            y,
            button: ModulePointerButton::Left,
        });
        state.borrow_mut().take_pending_action();

        panel.on_pointer_event(ModulePointerEvent::Move {
            x: timeline_start + 8,
            y,
        });
        assert_eq!(state.borrow_mut().take_pending_action(), None);

        panel.on_pointer_event(ModulePointerEvent::Up {
            x: timeline_start + 8,
            y,
        });
        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(LayersPanelAction::SetPropertyBlockTimingDestructive(
                "layer-1".to_string(),
                "raster".to_string(),
                "block-1".to_string(),
                8,
                1,
            ))
        );
    }

    fn state_with_two_single_breath_blocks() -> Rc<RefCell<LayersPanelState>> {
        let state = Rc::new(RefCell::new(LayersPanelState::default()));
        state.borrow_mut().sync(
            vec![row("layer-1", "Layer 1", 0, 10)],
            vec![property(
                "layer-1",
                "raster",
                "RASTER",
                LayerPropertyKind::Raster,
                vec![
                    PropertyTrackBlock {
                        id: "block-1".to_string(),
                        start_breath: 3,
                        length_breaths: 1,
                        is_blank: false,
                        interp_mode: None,
                        ease_out_percent: None,
                        ease_in_percent: None,
                    },
                    PropertyTrackBlock {
                        id: "block-2".to_string(),
                        start_breath: 6,
                        length_breaths: 1,
                        is_blank: false,
                        interp_mode: None,
                        ease_out_percent: None,
                        ease_in_percent: None,
                    },
                ],
            )],
            Some("layer-1".to_string()),
            Some("raster".to_string()),
            0,
            false,
            0,
            23,
            false,
            true,
        );
        state
    }

    #[test]
    fn clicking_the_add_row_queues_an_add_requested_action() {
        let state = state_with_rows();
        let mut panel = LayersPanelModule::new("layers_panel", rect(), state.clone());

        panel.on_pointer_event(ModulePointerEvent::Click {
            x: 1,
            y: panel.row_y(ROW_ADD_LAYER),
            button: ModulePointerButton::Left,
        });

        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(LayersPanelAction::AddRequested)
        );
    }

    #[test]
    fn clicking_the_visible_icon_queues_a_toggle_visible_action() {
        let state = state_with_rows();
        let mut panel = LayersPanelModule::new("layers_panel", rect(), state.clone());

        panel.on_pointer_event(ModulePointerEvent::Click {
            x: COL_VISIBLE,
            y: panel.row_y(7),
            button: ModulePointerButton::Left,
        });

        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(LayersPanelAction::ToggleVisible("layer-2".to_string()))
        );
    }

    #[test]
    fn clicking_the_lock_icon_queues_a_toggle_locked_action() {
        let state = state_with_rows();
        let mut panel = LayersPanelModule::new("layers_panel", rect(), state.clone());

        panel.on_pointer_event(ModulePointerEvent::Click {
            x: COL_LOCK,
            y: panel.row_y(4),
            button: ModulePointerButton::Left,
        });

        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(LayersPanelAction::ToggleLocked("layer-1".to_string()))
        );
    }

    #[test]
    fn clicking_the_delete_icon_queues_a_delete_action() {
        let state = state_with_rows();
        let mut panel = LayersPanelModule::new("layers_panel", rect(), state.clone());

        panel.on_pointer_event(ModulePointerEvent::Click {
            x: COL_DELETE,
            y: panel.row_y(4),
            button: ModulePointerButton::Left,
        });

        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(LayersPanelAction::Delete("layer-1".to_string()))
        );
    }

    #[test]
    fn clicking_the_auto_key_row_queues_a_toggle_auto_key_action() {
        let state = state_with_rows();
        let mut panel = LayersPanelModule::new("layers_panel", rect(), state.clone());

        panel.on_pointer_event(ModulePointerEvent::Click {
            x: 1,
            y: panel.row_y(ROW_AUTO_KEY),
            button: ModulePointerButton::Left,
        });

        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(LayersPanelAction::ToggleAutoKey)
        );
    }

    #[test]
    fn clicking_the_ruler_queues_a_set_current_breath_action_and_starts_scrubbing() {
        let state = state_with_rows();
        let mut panel = LayersPanelModule::new("layers_panel", rect(), state.clone());
        let (timeline_start, _) = panel.timeline_bounds();

        panel.on_pointer_event(ModulePointerEvent::Click {
            x: timeline_start + 5,
            y: panel.row_y(ROW_RULER),
            button: ModulePointerButton::Left,
        });

        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(LayersPanelAction::SetCurrentBreath(5))
        );
        assert!(panel.wants_pointer_capture());
    }

    #[test]
    fn dragging_after_a_ruler_click_keeps_updating_the_breath_until_release() {
        let state = state_with_rows();
        let mut panel = LayersPanelModule::new("layers_panel", rect(), state.clone());
        let (timeline_start, _) = panel.timeline_bounds();

        panel.on_pointer_event(ModulePointerEvent::Click {
            x: timeline_start + 3,
            y: panel.row_y(ROW_RULER),
            button: ModulePointerButton::Left,
        });
        state.borrow_mut().take_pending_action();

        panel.on_pointer_event(ModulePointerEvent::Move {
            x: timeline_start + 8,
            y: panel.row_y(ROW_RULER),
        });
        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(LayersPanelAction::SetCurrentBreath(8))
        );

        panel.on_pointer_event(ModulePointerEvent::Up {
            x: timeline_start + 8,
            y: panel.row_y(ROW_RULER),
        });
        assert!(!panel.wants_pointer_capture());
    }

    #[test]
    fn clicking_the_layer_row_timeline_space_selects_the_layer_without_starting_a_drag() {
        let state = state_with_rows();
        let mut panel = LayersPanelModule::new("layers_panel", rect(), state.clone());
        let (timeline_start, _) = panel.timeline_bounds();
        let y = panel.row_y(4);

        panel.on_pointer_event(ModulePointerEvent::Click {
            x: timeline_start + 2,
            y,
            button: ModulePointerButton::Left,
        });
        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(LayersPanelAction::Select("layer-1".to_string()))
        );
        assert!(!panel.wants_pointer_capture());
    }

    #[test]
    fn clicking_the_layer_row_timeline_edge_still_selects_the_layer() {
        let state = state_with_rows();
        let mut panel = LayersPanelModule::new("layers_panel", rect(), state.clone());
        let (timeline_start, _) = panel.timeline_bounds();
        let y = panel.row_y(4);

        panel.on_pointer_event(ModulePointerEvent::Click {
            x: timeline_start + 4,
            y,
            button: ModulePointerButton::Left,
        });

        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(LayersPanelAction::Select("layer-1".to_string()))
        );
    }

    #[test]
    fn clicking_the_layer_row_start_of_timeline_selects_the_layer() {
        let state = state_with_rows();
        let mut panel = LayersPanelModule::new("layers_panel", rect(), state.clone());
        let (timeline_start, _) = panel.timeline_bounds();
        let y = panel.row_y(4);

        panel.on_pointer_event(ModulePointerEvent::Click {
            x: timeline_start,
            y,
            button: ModulePointerButton::Left,
        });

        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(LayersPanelAction::Select("layer-1".to_string()))
        );
    }

    #[test]
    fn clicking_outside_a_single_breath_bar_still_selects_that_layer() {
        let state = state_with_rows();
        let mut panel = LayersPanelModule::new("layers_panel", rect(), state.clone());
        let (timeline_start, _) = panel.timeline_bounds();
        let y = panel.row_y(7);

        panel.on_pointer_event(ModulePointerEvent::Click {
            x: timeline_start,
            y,
            button: ModulePointerButton::Left,
        });

        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(LayersPanelAction::Select("layer-2".to_string()))
        );
        assert!(!panel.wants_pointer_capture());
    }

    #[test]
    fn clicking_outside_any_row_queues_nothing() {
        let state = state_with_rows();
        let mut panel = LayersPanelModule::new("layers_panel", rect(), state.clone());

        panel.on_pointer_event(ModulePointerEvent::Click {
            x: 0,
            y: panel.row_y(7),
            button: ModulePointerButton::Left,
        });

        assert_eq!(state.borrow_mut().take_pending_action(), None);
    }

    #[test]
    fn clicking_the_close_gizmo_hides_the_panel() {
        let state = state_with_rows();
        let mut panel = LayersPanelModule::new("layers_panel", rect(), state);

        let close_y = rect().y0 + (rect().y1 - rect().y0) - 1;
        panel.on_pointer_event(ModulePointerEvent::Click {
            x: rect().x0 + 3,
            y: close_y,
            button: ModulePointerButton::Left,
        });

        assert!(panel.is_hidden());
    }
}
