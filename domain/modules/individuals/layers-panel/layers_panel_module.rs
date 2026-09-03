use std::{cell::RefCell, rc::Rc, time::{Duration, Instant}};


use thaum_renderer_domain::{
    Cell, CellGraphic, CellGroup, CellGroupIntakeBehavior, CellPoint, GizmoBar, GizmoClickOutcome,
    GizmoKind, GizmoState, Module, ModulePointerButton, ModulePointerEvent, ModuleRect,
    PanelChrome, PersistedModuleUiState, UiColorRole, UiPalette, WorldPoint,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerPropertyKind {
    Raster,
    Move,
}

/// Which content neighbor a blank property block merges into, mirroring
/// `thaum_painter_domain::PropertyBlockMergeDirection` without coupling this module to
/// canonical storage (see this encapsulation's "does not own"). The orchestration layer maps
/// this onto the storage-owned enum when applying the action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeDirection {
    Left,
    Right,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyTrackBlock {
    pub id: String,
    pub start_breath: u32,
    pub length_breaths: u32,
    pub is_blank: bool,
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
    ToggleAutoKey,
    SetCurrentBreath(u32),
    SetLayerTiming(String, u32, u32),
    SetPropertyBlockTiming(String, String, String, u32, u32),
    SetPropertyBlockTimingPushed(String, String, String, u32, u32),
    SetPropertyBlockTimingDestructive(String, String, String, u32, u32),
    SplitPropertyBlock(String, String, String, u32),
    BlankPropertyBlock(String, String, String),
    MergeBlankPropertyBlock(String, String, String, MergeDirection),
    SwapPropertyBlocks(String, String, String, String),
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
}

impl LayersPanelState {
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
const COL_MARKER: i32 = 5;
const COL_NAME: i32 = 7;
const NAME_WIDTH: i32 = 10;
const TIMELINE_START: i32 = COL_NAME + NAME_WIDTH + 1;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PropertyBlockHitMode {
    EdgeStart,
    EdgeEnd,
    BodyMove,
    BodySingle,
    BlankStart,
    BlankEnd,
    BlankCenter,
    BlankSingle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PropertyBlockHit {
    layer_id: String,
    property_id: String,
    block_id: String,
    breath: u32,
    mode: PropertyBlockHitMode,
    is_blank: bool,
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
}

/// Resolves what a press-drag on `hit` should do, mirroring the old system's
/// `resolve_groups_raster_drag_mode`: blanks only drag via a right-click swap on their
/// center; edge drags are time-preserving (push) on left-click and destructive
/// (overwrite) on right-click; a multi-breath body swaps on left-click and destructively
/// slides over neighbors on right-click; a single-breath block destructively repositions
/// on left-click and destructively resizes from whichever side the pointer moves toward
/// on right-click.
fn resolve_property_drag_mode(
    hit: &PropertyBlockHit,
    button: ModulePointerButton,
) -> Option<(PropertyDragMode, PropertyTimingResolution)> {
    if hit.is_blank {
        return if button == ModulePointerButton::Right && hit.mode == PropertyBlockHitMode::BlankCenter
        {
            Some((PropertyDragMode::Swap, PropertyTimingResolution::Destructive))
        } else {
            None
        };
    }
    let is_right = button == ModulePointerButton::Right;
    let edge_resolution = if is_right {
        PropertyTimingResolution::Destructive
    } else {
        PropertyTimingResolution::Pushed
    };
    match hit.mode {
        PropertyBlockHitMode::BodySingle => Some(if is_right {
            (PropertyDragMode::DynamicResize, PropertyTimingResolution::Destructive)
        } else {
            (PropertyDragMode::Move, PropertyTimingResolution::Destructive)
        }),
        PropertyBlockHitMode::EdgeStart => Some((PropertyDragMode::TrimStart, edge_resolution)),
        PropertyBlockHitMode::EdgeEnd => Some((PropertyDragMode::TrimEnd, edge_resolution)),
        PropertyBlockHitMode::BodyMove => Some(if is_right {
            (PropertyDragMode::Move, PropertyTimingResolution::Destructive)
        } else {
            (PropertyDragMode::Swap, PropertyTimingResolution::Destructive)
        }),
        PropertyBlockHitMode::BlankStart
        | PropertyBlockHitMode::BlankEnd
        | PropertyBlockHitMode::BlankCenter
        | PropertyBlockHitMode::BlankSingle => None,
    }
}

/// Which side a blank block should merge into on a right double-click, mirroring the old
/// system's `getBlankCompactDirection`: the start/end caps always compact toward that side,
/// a single-breath blank compacts left, and a blank's interior compacts toward whichever
/// half of its span was clicked.
fn blank_merge_direction(hit: &PropertyBlockHit, block: &PropertyTrackBlock) -> MergeDirection {
    match hit.mode {
        PropertyBlockHitMode::BlankStart => MergeDirection::Left,
        PropertyBlockHitMode::BlankEnd => MergeDirection::Right,
        PropertyBlockHitMode::BlankSingle => MergeDirection::Left,
        _ => {
            let end = block.start_breath + block.length_breaths.max(1) - 1;
            let midpoint = (block.start_breath + end) / 2;
            if hit.breath <= midpoint {
                MergeDirection::Left
            } else {
                MergeDirection::Right
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

#[derive(Debug, Clone)]
struct RecentRasterClick {
    hit: PropertyBlockHit,
    button: ModulePointerButton,
    at: Instant,
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
    recent_raster_click: Option<RecentRasterClick>,
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
            recent_raster_click: None,
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
        (state.loop_window_start, state.loop_window_end.max(state.loop_window_start))
    }

    /// Playhead styling: rests bright at weight 2, hovers vivid at weight 3,
    /// and while its scrub drag is live goes vivid at weight 4.
    fn playhead_style(&self) -> (thaum_renderer_domain::CellColor, thaum_renderer_domain::CellWeight) {
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

    fn delete_column(&self) -> i32 {
        self.content_right()
    }

    fn property_bar_bounds(&self, property: &PropertyTrackRow) -> Option<(u32, u32)> {
        let start = property.blocks.iter().map(|block| block.start_breath).min()?;
        let end = property
            .blocks
            .iter()
            .map(|block| block.start_breath + block.length_breaths.saturating_sub(1))
            .max()?;
        Some((start, end))
    }

    fn layer_visible_for_property(&self, state: &LayersPanelState, property: &PropertyTrackRow) -> bool {
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
        })?;
        let mode = property_block_hit_mode(block, breath);
        Some(PropertyBlockHit {
            layer_id: property.layer_id.clone(),
            property_id: property.property_id.clone(),
            block_id: block.id.clone(),
            breath,
            mode,
            is_blank: block.is_blank,
        })
    }

    fn property_block_interaction_style(
        &self,
        property: &PropertyTrackRow,
        block: &PropertyTrackBlock,
        breath: u32,
    ) -> (thaum_renderer_domain::CellColor, thaum_renderer_domain::CellWeight) {
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
        // Bars rest at weight one; any highlight (hover or active drag) steps the
        // whole bar up to weight two.
        if drag_matches_block {
            return (
                self.palette.get(UiColorRole::Vivid),
                thaum_renderer_domain::CellWeight::from_index_clamped(2),
            );
        }
        if hover_matches_breath {
            return (
                self.palette.get(UiColorRole::Vivid),
                thaum_renderer_domain::CellWeight::from_index_clamped(2),
            );
        }
        if hover_matches_block {
            return (
                self.palette.get(UiColorRole::Vivid),
                thaum_renderer_domain::CellWeight::from_index_clamped(2),
            );
        }
        (
            base_color,
            thaum_renderer_domain::CellWeight::from_index_clamped(1),
        )
    }

    fn find_property_block(&self, hit: &PropertyBlockHit) -> Option<PropertyTrackBlock> {
        self.state
            .borrow()
            .property_rows
            .iter()
            .find(|property| {
                property.layer_id == hit.layer_id && property.property_id == hit.property_id
            })
            .and_then(|property| property.blocks.iter().find(|block| block.id == hit.block_id))
            .cloned()
    }

    fn handle_property_block_click(
        &mut self,
        hit: PropertyBlockHit,
        button: ModulePointerButton,
    ) {
        const DOUBLE_CLICK_WINDOW: Duration = Duration::from_millis(350);
        let now = Instant::now();
        let is_double_click = self
            .recent_raster_click
            .as_ref()
            .map(|recent| {
                recent.button == button
                    && recent.at.elapsed() <= DOUBLE_CLICK_WINDOW
                    && recent.hit.layer_id == hit.layer_id
                    && recent.hit.property_id == hit.property_id
                    && recent.hit.block_id == hit.block_id
                    && recent.hit.mode == hit.mode
            })
            .unwrap_or(false);
        self.recent_raster_click = Some(RecentRasterClick {
            hit: hit.clone(),
            button,
            at: now,
        });

        if is_double_click {
            self.property_block_drag = None;
            if hit.is_blank {
                if button == ModulePointerButton::Right {
                    if let Some(block) = self.find_property_block(&hit) {
                        let direction = blank_merge_direction(&hit, &block);
                        self.state.borrow_mut().queue_action(LayersPanelAction::MergeBlankPropertyBlock(
                            hit.layer_id,
                            hit.property_id,
                            hit.block_id,
                            direction,
                        ));
                    }
                }
                return;
            }
            if button == ModulePointerButton::Left && hit.mode == PropertyBlockHitMode::BodyMove {
                self.state.borrow_mut().queue_action(LayersPanelAction::SplitPropertyBlock(
                    hit.layer_id,
                    hit.property_id,
                    hit.block_id,
                    hit.breath,
                ));
                return;
            }
            if button == ModulePointerButton::Right {
                self.state.borrow_mut().queue_action(LayersPanelAction::BlankPropertyBlock(
                    hit.layer_id,
                    hit.property_id,
                    hit.block_id,
                ));
            }
            return;
        }

        self.state.borrow_mut().queue_action(LayersPanelAction::SelectProperty(
            hit.layer_id.clone(),
            hit.property_id.clone(),
        ));

        if hit.is_blank
            && button == ModulePointerButton::Left
            && matches!(hit.mode, PropertyBlockHitMode::BlankCenter | PropertyBlockHitMode::BlankSingle)
        {
            self.state
                .borrow_mut()
                .queue_action(LayersPanelAction::SetCurrentBreath(hit.breath));
            self.scrubbing_ruler = true;
            return;
        }

        let Some((mode, resolution)) = resolve_property_drag_mode(&hit, button) else {
            return;
        };
        let Some(block) = self.find_property_block(&hit) else {
            return;
        };
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
        });
    }
}

fn property_block_hit_mode(block: &PropertyTrackBlock, breath: u32) -> PropertyBlockHitMode {
    let start = block.start_breath;
    let end = block.start_breath + block.length_breaths.max(1).saturating_sub(1);
    if start == end {
        return if block.is_blank {
            PropertyBlockHitMode::BlankSingle
        } else {
            PropertyBlockHitMode::BodySingle
        };
    }
    if breath == start {
        return if block.is_blank {
            PropertyBlockHitMode::BlankStart
        } else {
            PropertyBlockHitMode::EdgeStart
        };
    }
    if breath == end {
        return if block.is_blank {
            PropertyBlockHitMode::BlankEnd
        } else {
            PropertyBlockHitMode::EdgeEnd
        };
    }
    if block.is_blank {
        PropertyBlockHitMode::BlankCenter
    } else {
        PropertyBlockHitMode::BodyMove
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
            return '▢';
        }
        if is_first {
            return '<';
        }
        if is_last {
            return '>';
        }
        return '▢';
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

impl Module for LayersPanelModule {
    fn id(&self) -> &str {
        &self.id
    }

    fn rect(&self) -> ModuleRect {
        self.rect
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

        let push_text = |cells: &mut Vec<Cell>, start_x: i32, y: i32, text: &str, color, max_x: i32| {
            for (column, glyph) in text.chars().enumerate() {
                let x = start_x + column as i32;
                if x > max_x {
                    break;
                }
                cells.push(Cell {
                    position: CellPoint { x, y, z: 0 },
                    graphic: CellGraphic::Glyph(glyph),
                    color,
                    ..Cell::default()
                });
            }
        };

        let (timeline_start, timeline_end) = self.timeline_bounds();
        let playhead_x = self.x_for_breath(state.current_breath);

        for (index, row_kind) in visible_rows.into_iter().enumerate() {
            let y = self.row_y(index);
            match row_kind {
                PanelRow::AutoKeyToggle => {
                    let auto_key_glyph = if state.auto_key_enabled { 'x' } else { ' ' };
                    push_text(
                        &mut cells,
                        1,
                        y,
                        &format!("[{auto_key_glyph}] AUTO KEY"),
                        self.palette.get(UiColorRole::Medium),
                        timeline_start - 2,
                    );

                    let end_breath = self.visible_timeline_end_breath();
                    let start_label = "0";
                    let current_label = state.current_breath.to_string();
                    let end_label = end_breath.to_string();
                    push_text(
                        &mut cells,
                        timeline_start,
                        y,
                        start_label,
                        self.palette.get(UiColorRole::Dimmest),
                        timeline_end,
                    );
                    let end_start = (timeline_end - end_label.len() as i32 + 1).max(timeline_start);
                    push_text(
                        &mut cells,
                        end_start,
                        y,
                        &end_label,
                        self.palette.get(UiColorRole::Dimmest),
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
                            .clamp(timeline_start, timeline_end - loop_start_label.len() as i32 + 1);
                        push_text(
                            &mut cells,
                            loop_start_x,
                            y,
                            &loop_start_label,
                            self.palette.get(UiColorRole::Medium),
                            timeline_end,
                        );
                        let loop_end_x = (self.x_for_breath(loop_end)
                            - (loop_end_label.len() as i32 / 2))
                            .clamp(timeline_start, timeline_end - loop_end_label.len() as i32 + 1);
                        push_text(
                            &mut cells,
                            loop_end_x,
                            y,
                            &loop_end_label,
                            self.palette.get(UiColorRole::Medium),
                            timeline_end,
                        );
                    }
                    let current_start = (playhead_x - (current_label.len() as i32 / 2))
                        .clamp(timeline_start, timeline_end - current_label.len() as i32 + 1);
                    let (playhead_color, playhead_weight) = self.playhead_style();
                    let mut current_cell = Cell {
                        position: CellPoint { x: current_start, y, z: 0 },
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
                    let play_label = if state.playing { "[||] STOP" } else { "[>] PLAY" };
                    push_text(
                        &mut cells,
                        PLAY_BUTTON_START,
                        y,
                        play_label,
                        if state.playing {
                            self.palette.get(UiColorRole::Vivid)
                        } else {
                            self.palette.get(UiColorRole::Medium)
                        },
                        PLAY_BUTTON_END,
                    );
                    let loop_label = if state.loop_enabled { "[x] LOOP" } else { "[ ] LOOP" };
                    push_text(
                        &mut cells,
                        LOOP_BUTTON_START,
                        y,
                        loop_label,
                        if state.loop_enabled {
                            self.palette.get(UiColorRole::Medium)
                        } else {
                            self.palette.get(UiColorRole::Dimmest)
                        },
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
                    let highlighted =
                        self.loop_window_drag.is_some() || self.hovered_loop_window;
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
                    push_text(
                        &mut cells,
                        1,
                        y,
                        "+ NEW LAYER",
                        self.palette.get(UiColorRole::Medium),
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
                    let visible_glyph = if row.visible { 'o' } else { '.' };
                    let lock_glyph = if row.locked { 'L' } else { '.' };
                    let marker = if is_selected { '*' } else { '-' };

                    cells.push(Cell {
                        position: CellPoint {
                            x: COL_VISIBLE,
                            y,
                            z: 0,
                        },
                        graphic: CellGraphic::Glyph(visible_glyph),
                        color: text_color,
                        ..Cell::default()
                    });
                    cells.push(Cell {
                        position: CellPoint { x: COL_LOCK, y, z: 0 },
                        graphic: CellGraphic::Glyph(lock_glyph),
                        color: text_color,
                        ..Cell::default()
                    });
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
                    push_text(&mut cells, COL_NAME, y, &row.name, text_color, timeline_start - 2);

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

                    cells.push(Cell {
                        position: CellPoint {
                            x: self.delete_column(),
                            y,
                            z: 0,
                        },
                        graphic: CellGraphic::Glyph('x'),
                        color: self.palette.get(UiColorRole::Medium),
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
                    push_text(
                        &mut cells,
                        COL_NAME,
                        y,
                        &property.label,
                        text_color,
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
                        .map(|drag| (drag.block_id.as_str(), drag.last_requested));
                    for block in &property.blocks {
                        // While a timing drag is in flight the document is untouched,
                        // so the dragged bar previews at its requested span instead of
                        // its stored one; victims stay intact until the release commit.
                        let (draw_start, draw_length) = match drag_preview {
                            Some((drag_block_id, Some((start, length))))
                                if drag_block_id == block.id.as_str() =>
                            {
                                (start, length)
                            }
                            _ => (block.start_breath, block.length_breaths.max(1)),
                        };
                        let block_start_x = self.x_for_breath(draw_start);
                        for local_index in 0..draw_length {
                            let breath = draw_start + local_index;
                            let x = block_start_x + local_index as i32;
                            if x > timeline_end {
                                break;
                            }
                            let (color, weight) =
                                self.property_block_interaction_style(property, block, breath);
                            cells.push(Cell {
                                position: CellPoint { x, y, z: 0 },
                                graphic: CellGraphic::Glyph(property_track_cell_graphic(
                                    block.is_blank,
                                    is_visible,
                                    draw_length,
                                    local_index,
                                )),
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
                match row_kind {
                    PanelRow::AutoKeyToggle => {
                        if button == ModulePointerButton::Left {
                            self.state.borrow_mut().queue_action(LayersPanelAction::ToggleAutoKey);
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
                                } else if (LOOP_BUTTON_START..=LOOP_BUTTON_END)
                                    .contains(&local_x)
                                {
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
                            self.state.borrow_mut().queue_action(LayersPanelAction::AddRequested);
                        }
                    }
                    PanelRow::Layer(row_index) => {
                        if button != ModulePointerButton::Left {
                            return;
                        }
                        let local_x = x - self.rect.x0;
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
                        if local_x == self.delete_column() {
                            self.state
                                .borrow_mut()
                                .queue_action(LayersPanelAction::Delete(row.id));
                            return;
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
                                self.state.borrow_mut().queue_action(LayersPanelAction::SelectProperty(
                                    property.layer_id,
                                    property.property_id,
                                ));
                            }
                        }
                    }
                }
            }
            ModulePointerEvent::Move { x, y } => {
                self.gizmo_state.note_pointer(&self.gizmos, self.rect, x, y);
                self.hovered_property_block = self.property_block_hit_at(x, y);
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
                                .clamp(0, drag.orig_end as i64) as u32;
                            (new_start, drag.orig_end)
                        }
                        LoopWindowDragMode::TrimEnd => {
                            let new_end = (drag.orig_end as i64 + delta)
                                .max(drag.orig_start as i64) as u32;
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
                    self.state.borrow_mut().queue_action(LayersPanelAction::SetLayerTiming(
                        drag.layer_id.clone(),
                        new_start,
                        new_length,
                    ));
                }
                // Compute the breath up front: `breath_at_x` borrows all of
                // `self`, which conflicts with the mutable drag borrow below.
                let current_breath = self
                    .property_block_drag
                    .as_ref()
                    .map(|_| {
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
                                    .clamp(0, max_start as i64) as u32;
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
                                        .max(drag.orig_start as i64) as u32;
                                    (drag.orig_start, new_end - drag.orig_start + 1)
                                }
                            }
                            PropertyDragMode::Swap => unreachable!(),
                        };
                        drag.last_requested = Some((requested_start, requested_length));
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
                            self.state.borrow_mut().queue_action(LayersPanelAction::SwapPropertyBlocks(
                                drag.layer_id,
                                drag.property_id,
                                drag.block_id,
                                target_block_id,
                            ));
                        }
                    } else if let Some((start, length)) = drag.last_requested {
                        // One commit per drag: the previewed span is applied exactly
                        // here, so destructive drags only yield the bars under the
                        // release position instead of everything crossed mid-flight.
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
            }
            ModulePointerEvent::Down { .. } => {}
        }
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

    fn property(layer_id: &str, property_id: &str, label: &str, kind: LayerPropertyKind, blocks: Vec<PropertyTrackBlock>) -> PropertyTrackRow {
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
                    }],
                ),
                property(
                    "layer-1",
                    "move",
                    "MOVE",
                    LayerPropertyKind::Move,
                    vec![],
                ),
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
            group.cells.get(&CellPoint {
                x: COL_NAME,
                y: panel.row_y(5),
                z: 0,
            }).map(|cell| cell.graphic.clone()),
            Some(CellGraphic::Glyph('R'))
        );
        assert_eq!(
            group.cells.get(&CellPoint {
                x: timeline_start,
                y: panel.row_y(5),
                z: 0,
            }).map(|cell| cell.graphic.clone()),
            Some(CellGraphic::Glyph('║'))
        );
    }

    #[test]
    fn raster_property_blocks_use_old_system_endcaps_midsections_and_singles() {
        assert_eq!(property_track_cell_graphic(false, true, 1, 0), '█');
        assert_eq!(property_track_cell_graphic(false, true, 2, 0), '█');
        assert_eq!(property_track_cell_graphic(false, true, 2, 1), '▦');
        assert_eq!(property_track_cell_graphic(false, true, 4, 1), '▥');
        assert_eq!(property_track_cell_graphic(true, true, 1, 0), '▢');
        assert_eq!(property_track_cell_graphic(true, true, 2, 0), '<');
        assert_eq!(property_track_cell_graphic(true, true, 2, 1), '>');
        assert_eq!(property_track_cell_graphic(true, true, 4, 1), '▢');
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
        assert_eq!(cell.weight, thaum_renderer_domain::CellWeight::from_index_clamped(2));
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
                    PropertyTrackBlock { id: "block-1".to_string(), start_breath: 0, length_breaths: 5, is_blank: false },
                    PropertyTrackBlock { id: "block-2".to_string(), start_breath: 10, length_breaths: 5, is_blank: false },
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
    fn double_clicking_the_body_of_a_raster_block_splits_it() {
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
    fn right_double_clicking_a_content_block_blanks_it() {
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
                    PropertyTrackBlock { id: "block-1".to_string(), start_breath: 0, length_breaths: 3, is_blank: false },
                    PropertyTrackBlock { id: "block-2".to_string(), start_breath: 3, length_breaths: 5, is_blank: true },
                    PropertyTrackBlock { id: "block-3".to_string(), start_breath: 8, length_breaths: 3, is_blank: false },
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
    fn right_double_clicking_the_left_half_of_a_blank_merges_it_left() {
        let state = state_with_a_blank_between_two_content_blocks();
        let mut panel = LayersPanelModule::new("layers_panel", rect(), state.clone());
        let (timeline_start, _) = panel.timeline_bounds();
        let y = panel.row_y(5);

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
            Some(LayersPanelAction::MergeBlankPropertyBlock(
                "layer-1".to_string(),
                "raster".to_string(),
                "block-2".to_string(),
                MergeDirection::Left,
            ))
        );
    }

    #[test]
    fn right_double_clicking_the_right_half_of_a_blank_merges_it_right() {
        let state = state_with_a_blank_between_two_content_blocks();
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
            Some(LayersPanelAction::MergeBlankPropertyBlock(
                "layer-1".to_string(),
                "raster".to_string(),
                "block-2".to_string(),
                MergeDirection::Right,
            ))
        );
    }

    #[test]
    fn left_click_dragging_the_center_of_a_blank_scrubs_the_timeline_instead_of_dragging() {
        let state = state_with_a_blank_between_two_content_blocks();
        let mut panel = LayersPanelModule::new("layers_panel", rect(), state.clone());
        let (timeline_start, _) = panel.timeline_bounds();
        let y = panel.row_y(5);

        panel.on_pointer_event(ModulePointerEvent::Click {
            x: timeline_start + 5,
            y,
            button: ModulePointerButton::Left,
        });

        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(LayersPanelAction::SetCurrentBreath(5))
        );
        assert!(panel.wants_pointer_capture());

        panel.on_pointer_event(ModulePointerEvent::Move {
            x: timeline_start + 6,
            y,
        });
        assert_eq!(
            state.borrow_mut().take_pending_action(),
            Some(LayersPanelAction::SetCurrentBreath(6))
        );
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
                    PropertyTrackBlock { id: "block-1".to_string(), start_breath: 0, length_breaths: 5, is_blank: false },
                    PropertyTrackBlock { id: "block-2".to_string(), start_breath: 5, length_breaths: 5, is_blank: false },
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
                    PropertyTrackBlock { id: "block-1".to_string(), start_breath: 3, length_breaths: 1, is_blank: false },
                    PropertyTrackBlock { id: "block-2".to_string(), start_breath: 6, length_breaths: 1, is_blank: false },
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
            x: panel.delete_column(),
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
