use thaum_renderer_domain::{CameraViewOrientation, CellGraphic, CellMaterialId, CellPoint};

use crate::{
    brush::{self, Canvas, PaintedCell},
    clipboard::WorldCopyData,
    fill::{self, CanvasBounds, FillConnectivity},
    paint_color::PaintColor,
    painter_tools::shared::{drag_behavior, DragBehavior},
    selection_state::{flood_select_points, PainterSelection, SelectionMode},
    text::{TextLayoutOptions, DEFAULT_TEXT_LAYOUT_OPTIONS},
};
/// Which pointer hand is acting right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaintHand {
    Left,
    Right,
}

impl PaintHand {
    /// The other hand, used by the picker's opposite-hand property.
    pub fn opposite(self) -> PaintHand {
        match self {
            PaintHand::Left => PaintHand::Right,
            PaintHand::Right => PaintHand::Left,
        }
    }
}

/// Which painter-operations primitive one hand is assigned to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaintTool {
    Brush,
    Erase,
    Fill,
    /// Freehand bound: press-drag-release records a lasso path; release
    /// fills the enclosed cells through `apply_lasso_for_hand`, never
    /// through `apply_at_for_hand`.
    Lasso,
    /// Live typing mode: edits flow through the session's
    /// `TextEntryState`, never through `apply_at_for_hand`.
    Text,
    /// Click-only sampler: a press samples the cell under the cursor into
    /// a hand's graphic/color/weight; nothing is ever painted or selected.
    Picker,
    /// Click-only placer: a press places the hand user's copied world cells
    /// at the click cell through the hand's resolved painted cell; drags
    /// never stamp. Application lives in the `stamp_*` seams below and the
    /// canvas-pointer press dispatch, never through `apply_at_for_hand`.
    Stamp,
    /// Selection mover: with an active plane selection, press-drag-release
    /// moves the selected raster content — release clears the old selection
    /// area, pastes the content at the new place, and re-anchors the
    /// selection there, one bounded undoable op. Without a selection it is
    /// a stub for the future layer-offset behavior. Application lives in
    /// the canvas-pointer move-stroke dispatch, never through
    /// `apply_at_for_hand`.
    Move,
}

impl PaintTool {
    /// Every live tool, in toolbox order. Drift-tested against the
    /// painter-tools registry.
    pub fn all() -> [PaintTool; 8] {
        [
            PaintTool::Brush,
            PaintTool::Erase,
            PaintTool::Fill,
            PaintTool::Lasso,
            PaintTool::Text,
            PaintTool::Picker,
            PaintTool::Stamp,
            PaintTool::Move,
        ]
    }

    /// Stable registration id (also the persistence name). One small bridge
    /// between this enum and the painter-tools registry; the parity test
    /// fails if the two drift apart.
    pub fn id(self) -> &'static str {
        match self {
            PaintTool::Brush => "brush",
            PaintTool::Erase => "erase",
            PaintTool::Fill => "fill",
            PaintTool::Lasso => "lasso",
            PaintTool::Text => "text",
            PaintTool::Picker => "picker",
            PaintTool::Stamp => "stamp",
            PaintTool::Move => "move",
        }
    }

    pub fn from_id(id: &str) -> Option<PaintTool> {
        match id {
            "brush" => Some(PaintTool::Brush),
            "erase" => Some(PaintTool::Erase),
            "fill" => Some(PaintTool::Fill),
            "lasso" => Some(PaintTool::Lasso),
            "text" => Some(PaintTool::Text),
            "picker" => Some(PaintTool::Picker),
            "stamp" => Some(PaintTool::Stamp),
            "move" => Some(PaintTool::Move),
            _ => None,
        }
    }

    /// The tool-specific property row ids this tool uses in the properties
    /// panel, delegated to the tool's registered descriptor so the registry
    /// stays the single source of registration truth.
    pub fn property_row_ids(self) -> &'static [&'static str] {
        crate::painter_tools::require_by_id(self.id()).property_row_ids
    }
}

/// Which authored channel a hand may paint, select through, or lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaintChannel {
    Graphic,
    Color,
    Weight,
}

/// Whether a hand is painting the image itself or the live selection surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaintTarget {
    Image,
    Selection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelMask {
    pub graphic: bool,
    pub color: bool,
    pub weight: bool,
}

impl ChannelMask {
    pub fn all() -> Self {
        Self {
            graphic: true,
            color: true,
            weight: true,
        }
    }

    pub fn is_enabled(&self, channel: PaintChannel) -> bool {
        match channel {
            PaintChannel::Graphic => self.graphic,
            PaintChannel::Color => self.color,
            PaintChannel::Weight => self.weight,
        }
    }

    pub fn toggle(&mut self, channel: PaintChannel) {
        match channel {
            PaintChannel::Graphic => self.graphic = !self.graphic,
            PaintChannel::Color => self.color = !self.color,
            PaintChannel::Weight => self.weight = !self.weight,
        }
    }

    pub fn any_enabled(&self) -> bool {
        self.graphic || self.color || self.weight
    }

    pub fn all_enabled(&self) -> bool {
        self.graphic && self.color && self.weight
    }
}

pub type EditChannels = ChannelMask;
pub type SelectChannels = ChannelMask;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandState {
    pub graphic: CellGraphic,
    pub color: PaintColor,
    pub weight_index: i64,
    pub brush_size: i32,
    pub fill_diagonal: bool,
    /// Picker property: whether this hand's picker click hands its sampled
    /// channels to the opposite hand instead of itself. Per-hand toggle,
    /// independent per J.
    pub pick_opposite_hand: bool,
    pub edit_channels: EditChannels,
    pub select_channels: SelectChannels,
    pub target: PaintTarget,
}

impl Default for HandState {
    fn default() -> Self {
        Self {
            graphic: CellGraphic::Glyph('#'),
            color: PaintColor::default(),
            weight_index: 2,
            brush_size: 1,
            fill_diagonal: false,
            pick_opposite_hand: false,
            edit_channels: EditChannels::all(),
            select_channels: SelectChannels::all(),
            target: PaintTarget::Image,
        }
    }
}

/// Live tool settings and active authoring-hand state for the painter session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolState {
    pub left_tool: PaintTool,
    pub right_tool: PaintTool,
    pub active_hand: PaintHand,
    pub left_hand: HandState,
    pub right_hand: HandState,
    /// Text tool layout properties: per-character and per-Enter cursor steps
    /// as view-relative 3D offsets (right, down, depth), live-session state
    /// per the tool-state contract.
    pub text_options: TextLayoutOptions,
    /// Whether Space during typing clears the cell under the cursor.
    pub text_space_replace: bool,
}

impl Default for ToolState {
    fn default() -> Self {
        Self {
            left_tool: PaintTool::Brush,
            right_tool: PaintTool::Erase,
            active_hand: PaintHand::Left,
            left_hand: HandState::default(),
            right_hand: HandState::default(),
            text_options: DEFAULT_TEXT_LAYOUT_OPTIONS,
            text_space_replace: true,
        }
    }
}

impl ToolState {
    pub fn current_tool(&self) -> PaintTool {
        self.tool_for_hand(self.active_hand)
    }

    pub fn tool_for_hand(&self, hand: PaintHand) -> PaintTool {
        match hand {
            PaintHand::Left => self.left_tool,
            PaintHand::Right => self.right_tool,
        }
    }

    pub fn hand_state(&self, hand: PaintHand) -> HandState {
        match hand {
            PaintHand::Left => self.left_hand.clone(),
            PaintHand::Right => self.right_hand.clone(),
        }
    }

    fn hand_state_mut(&mut self, hand: PaintHand) -> &mut HandState {
        match hand {
            PaintHand::Left => &mut self.left_hand,
            PaintHand::Right => &mut self.right_hand,
        }
    }

    pub fn set_tool_for_hand(&mut self, hand: PaintHand, tool: PaintTool) {
        match hand {
            PaintHand::Left => self.left_tool = tool,
            PaintHand::Right => self.right_tool = tool,
        }
        self.active_hand = hand;
    }

    pub fn set_color_for_hand(&mut self, hand: PaintHand, color: PaintColor) {
        self.hand_state_mut(hand).color = color;
        self.active_hand = hand;
    }

    pub fn set_material_for_hand(&mut self, hand: PaintHand, material: CellMaterialId) {
        self.set_color_for_hand(hand, PaintColor::material(material));
    }

    pub fn set_graphic_for_hand(&mut self, hand: PaintHand, graphic: CellGraphic) {
        self.hand_state_mut(hand).graphic = graphic;
        self.active_hand = hand;
    }

    pub fn set_weight_for_hand(&mut self, hand: PaintHand, weight_index: i64) {
        self.hand_state_mut(hand).weight_index = weight_index.clamp(0, 3);
        self.active_hand = hand;
    }

    pub fn toggle_edit_channel_for_hand(&mut self, hand: PaintHand, channel: PaintChannel) {
        self.hand_state_mut(hand).edit_channels.toggle(channel);
        self.active_hand = hand;
    }

    pub fn toggle_select_channel_for_hand(&mut self, hand: PaintHand, channel: PaintChannel) {
        self.hand_state_mut(hand).select_channels.toggle(channel);
        self.active_hand = hand;
    }

    pub fn set_target_for_hand(&mut self, hand: PaintHand, target: PaintTarget) {
        self.hand_state_mut(hand).target = target;
        self.active_hand = hand;
    }

    pub fn set_brush_size_for_hand(&mut self, hand: PaintHand, brush_size: i32) {
        self.hand_state_mut(hand).brush_size = brush_size.clamp(1, 5);
        self.active_hand = hand;
    }

    pub fn set_fill_diagonal_for_hand(&mut self, hand: PaintHand, fill_diagonal: bool) {
        self.hand_state_mut(hand).fill_diagonal = fill_diagonal;
        self.active_hand = hand;
    }

    pub fn set_pick_opposite_hand_for_hand(&mut self, hand: PaintHand, pick_opposite_hand: bool) {
        self.hand_state_mut(hand).pick_opposite_hand = pick_opposite_hand;
        self.active_hand = hand;
    }

    /// Picker behavior: samples the cell under the cursor into the
    /// receiving hand's graphic/color/weight, one channel at a time, gated
    /// by the *picking* hand's edit-channel toggles (source-of-truth from
    /// J: the picker reads the toggles for what the tool is editing).
    /// With `pick_opposite_hand` on, the
    /// sample lands on the opposite hand's state. Never paints and never
    /// touches the selection surface.
    pub fn pick_at_for_hand(&mut self, canvas: &Canvas, position: CellPoint, hand: PaintHand) {
        let picking = self.hand_state(hand);
        let target_hand = if picking.pick_opposite_hand {
            hand.opposite()
        } else {
            hand
        };
        if !picking.edit_channels.any_enabled() {
            return;
        }
        let Some(cell) = canvas.get(&position) else {
            return;
        };
        if picking.edit_channels.graphic {
            self.set_graphic_for_hand(target_hand, cell.graphic.clone());
        }
        if picking.edit_channels.color {
            self.set_color_for_hand(target_hand, cell.color);
        }
        if picking.edit_channels.weight {
            self.set_weight_for_hand(target_hand, cell.weight_index);
        }
    }

    /// Nudges one axis (0 = along right, 1 = down the screen, 2 = into depth)
    /// of the per-character cursor step by `delta` cells, clamped to the
    /// layout options' −16..16 range. Text properties are shared across
    /// hands: the typing session reads one layout regardless of the hand.
    pub fn nudge_text_char_step(&mut self, axis: usize, delta: i32) {
        nudge_step(&mut self.text_options.char_step, axis, delta);
        self.text_options = self.text_options.clamped();
    }

    /// Same as [`ToolState::nudge_text_char_step`] for the per-Enter line
    /// step.
    pub fn nudge_text_enter_step(&mut self, axis: usize, delta: i32) {
        nudge_step(&mut self.text_options.enter_step, axis, delta);
        self.text_options = self.text_options.clamped();
    }

    /// Sets one axis (0 = along right, 1 = down the screen, 2 = into depth)
    /// of the per-character cursor step to an exact value (typed into the
    /// properties panel's number field), clamped like the panel's range.
    pub fn set_text_char_step_axis(&mut self, axis: usize, value: i32) {
        set_step_axis(&mut self.text_options.char_step, axis, value);
        self.text_options = self.text_options.clamped();
    }

    /// Same as [`ToolState::set_text_char_step_axis`] for the per-Enter line
    /// step.
    pub fn set_text_enter_step_axis(&mut self, axis: usize, value: i32) {
        set_step_axis(&mut self.text_options.enter_step, axis, value);
        self.text_options = self.text_options.clamped();
    }

    /// The captured brush cell for a typing session: the hand's current
    /// graphic/color/weight frozen at click time (old
    /// `getBrushForButton(text_mode_button)` capture).
    pub fn text_brush_cell_for_hand(&self, hand: PaintHand) -> PaintedCell {
        let hand_state = self.hand_state(hand);
        PaintedCell {
            graphic: hand_state.graphic,
            color: hand_state.color,
            weight_index: hand_state.weight_index,
        }
    }

    fn resolved_painted_cell(
        &self,
        existing: Option<&PaintedCell>,
        hand: PaintHand,
    ) -> PaintedCell {
        let hand_state = self.hand_state(hand);
        // Unified empty-cell rule: an authored blank (space glyph) is empty —
        // its color/weight never survive as base values.
        let existing = brush::effective_cell(existing);
        // Source-of-truth from J: if any of the three channels is locked, a
        // stroke must not place on empty cells. A locked channel has no
        // existing value to keep on an empty cell, so a partial edit cannot
        // resolve there — the same no-op the gfx-locked case already had.
        if existing.is_none() && !hand_state.edit_channels.all_enabled() {
            return PaintedCell {
                graphic: CellGraphic::Glyph(' '),
                color: hand_state.color,
                weight_index: hand_state.weight_index,
            };
        }
        let base_graphic = existing
            .map(|cell| cell.graphic.clone())
            .unwrap_or(CellGraphic::Glyph(' '));
        let base_color = existing.map(|cell| cell.color).unwrap_or(hand_state.color);
        let base_weight_index = existing
            .map(|cell| cell.weight_index)
            .unwrap_or(hand_state.weight_index);
        PaintedCell {
            graphic: if hand_state.edit_channels.graphic {
                hand_state.graphic.clone()
            } else {
                base_graphic
            },
            color: if hand_state.edit_channels.color {
                hand_state.color
            } else {
                base_color
            },
            weight_index: if hand_state.edit_channels.weight {
                hand_state.weight_index
            } else {
                base_weight_index
            },
        }
    }

    fn fill_connectivity_for_hand(&self, hand: PaintHand) -> FillConnectivity {
        if self.hand_state(hand).fill_diagonal {
            FillConnectivity::CardinalAndDiagonal
        } else {
            FillConnectivity::Cardinal
        }
    }

    fn brush_points_for_hand(
        &self,
        position: CellPoint,
        hand: PaintHand,
        orientation: CameraViewOrientation,
    ) -> Vec<CellPoint> {
        brush::brush_points(position, self.hand_state(hand).brush_size, orientation)
    }

    fn edit_points_for_hand(
        &self,
        canvas: &Canvas,
        position: CellPoint,
        hand: PaintHand,
        bounds: CanvasBounds,
        orientation: CameraViewOrientation,
    ) -> Vec<CellPoint> {
        // Only per-position tools produce edit points here; release-bound
        // tools (lasso fills on release) and typing-session tools (text
        // stages through the session bridge) never do.
        if drag_behavior(self.tool_for_hand(hand).id()) != DragBehavior::PerPosition {
            return Vec::new();
        }
        match self.tool_for_hand(hand) {
            PaintTool::Brush | PaintTool::Erase => {
                self.brush_points_for_hand(position, hand, orientation)
            }
            PaintTool::Fill => {
                // Region sensing follows the hand's Select row: unlocked
                // channels are ignored when matching neighbors (J 2026-09-10).
                let select = self.hand_state(hand).select_channels;
                fill::flood_fill_points_with_connectivity(
                    canvas,
                    position,
                    bounds,
                    self.fill_connectivity_for_hand(hand),
                    fill::FillChannelMask {
                        graphic: select.graphic,
                        color: select.color,
                        weight: select.weight,
                    },
                )
            }
            _ => Vec::new(),
        }
    }

    fn apply_canvas_at_for_hand(
        &mut self,
        canvas: &mut Canvas,
        selection: &PainterSelection,
        position: CellPoint,
        hand: PaintHand,
        bounds: CanvasBounds,
        orientation: CameraViewOrientation,
    ) {
        // Release-bound and typing-session tools never edit per position:
        // lasso paints on release, text flows through the live typing session.
        if drag_behavior(self.tool_for_hand(hand).id()) != DragBehavior::PerPosition {
            return;
        }
        let hand_state = self.hand_state(hand);
        let points = selection.filter_plane_edit_points(self.edit_points_for_hand(
            canvas,
            position,
            hand,
            bounds,
            orientation,
        ));
        match self.tool_for_hand(hand) {
            PaintTool::Brush => {
                if !hand_state.edit_channels.any_enabled() {
                    return;
                }
                for point in points {
                    let painted = self.resolved_painted_cell(canvas.get(&point), hand);
                    brush::write_cell(canvas, point, painted);
                }
            }
            PaintTool::Erase => {
                for point in points {
                    brush::erase(canvas, point);
                }
            }
            PaintTool::Fill => {
                if !hand_state.edit_channels.any_enabled() {
                    return;
                }
                for point in points {
                    let painted = self.resolved_painted_cell(canvas.get(&point), hand);
                    brush::write_cell(canvas, point, painted);
                }
            }
            _ => {}
        }
    }

    pub fn selection_points_for_hand(
        &self,
        canvas: &Canvas,
        position: CellPoint,
        hand: PaintHand,
        bounds: CanvasBounds,
        orientation: CameraViewOrientation,
    ) -> Vec<CellPoint> {
        // Release-bound and typing-session tools never select per position:
        // lasso selects its enclosed region on release through
        // `lasso_selection_points`, text owns the keyboard instead.
        if drag_behavior(self.tool_for_hand(hand).id()) != DragBehavior::PerPosition {
            return Vec::new();
        }
        let hand_state = self.hand_state(hand);
        match self.tool_for_hand(hand) {
            PaintTool::Brush => {
                if !hand_state.select_channels.any_enabled() {
                    return Vec::new();
                }
                self.brush_points_for_hand(position, hand, orientation)
            }
            PaintTool::Erase => self.brush_points_for_hand(position, hand, orientation),
            PaintTool::Fill => {
                // The Select row is the comparison truth for fill, both when
                // sensing paint regions and when flood selecting (J
                // 2026-09-10). With every channel unlocked the hand matches
                // only cells equal on all channels.
                flood_select_points(canvas, position, bounds, hand_state.select_channels)
            }
            _ => Vec::new(),
        }
    }

    fn apply_selection_at_for_hand(
        &mut self,
        canvas: &Canvas,
        selection: &mut PainterSelection,
        position: CellPoint,
        hand: PaintHand,
        bounds: CanvasBounds,
        orientation: CameraViewOrientation,
    ) {
        let points = self.selection_points_for_hand(canvas, position, hand, bounds, orientation);
        let mode = crate::painter_tools::shared::selection_behavior(self.tool_for_hand(hand).id())
            .resolve(selection.mode(), SelectionMode::Subtract);
        selection.apply_plane_points_with_mode(points, mode);
    }

    /// Rasterizes the hand's lasso bound and fills every enclosed cell with
    /// that hand's state, through the same seams as brush/fill: the region
    /// is gated by the current selection, and each filled cell resolves
    /// through the channel mask (under the unified empty-cell rule, a
    /// resolution that comes out an authored blank writes nothing — empty
    /// cells stay empty; a color-only fill recolors glyphs in place only).
    pub fn apply_lasso_for_hand(
        &mut self,
        canvas: &mut Canvas,
        selection: &PainterSelection,
        path: &[CellPoint],
        hand: PaintHand,
        orientation: CameraViewOrientation,
    ) {
        let points = self.lasso_edit_points(selection, path, hand, orientation);
        for point in points {
            let painted = self.resolved_painted_cell(canvas.get(&point), hand);
            brush::write_cell(canvas, point, painted);
        }
    }

    /// The image-edit half of the lasso: exactly the cells `apply_lasso_for_hand`
    /// will paint for the hand's bound — the enclosed region rasterized through
    /// the pure lasso operation, filtered by the current selection, and gated by
    /// the hand's edit channels. The in-progress preview consumes this same seam
    /// so the flashing area is exactly what release will edit.
    pub fn lasso_edit_points(
        &self,
        selection: &PainterSelection,
        path: &[CellPoint],
        hand: PaintHand,
        orientation: CameraViewOrientation,
    ) -> Vec<CellPoint> {
        if !self.hand_state(hand).edit_channels.any_enabled() {
            return Vec::new();
        }
        selection.filter_plane_edit_points(crate::lasso::lasso_points(path, orientation))
    }

    /// Per-cell preview data for the in-progress lasso: the exact points
    /// release will edit, each with the cell as currently drawn and the
    /// painted cell the commit will produce (through the same resolution
    /// `apply_lasso_for_hand` uses). Points that resolve to an authored
    /// blank are skipped: release writes nothing there.
    pub fn lasso_preview_cells(
        &self,
        canvas: &Canvas,
        selection: &PainterSelection,
        path: &[CellPoint],
        hand: PaintHand,
        orientation: CameraViewOrientation,
    ) -> Vec<crate::lasso_stroke::LassoPreviewCell> {
        self.lasso_edit_points(selection, path, hand, orientation)
            .into_iter()
            .filter_map(|point| {
                let current = canvas.get(&point).cloned();
                let upcoming = self.resolved_painted_cell(canvas.get(&point), hand);
                if brush::is_blank_cell(&upcoming) {
                    // Release writes nothing on an authored-blank resolution.
                    return None;
                }
                Some(crate::lasso_stroke::LassoPreviewCell {
                    point,
                    current,
                    upcoming,
                })
            })
            .collect()
    }

    /// The selection-surface half of the lasso: the enclosed cells of the
    /// hand's bound, gated by that hand's select-channel enable state. The
    /// caller applies them through the hand's resolved selection mode.
    pub fn lasso_selection_points(
        &self,
        path: &[CellPoint],
        hand: PaintHand,
        orientation: CameraViewOrientation,
    ) -> Vec<CellPoint> {
        if !self.hand_state(hand).select_channels.any_enabled() {
            return Vec::new();
        }
        crate::lasso::lasso_points(path, orientation)
    }

    /// Selection-target lasso preview: the drawing will not change on
    /// release, so both flash halves carry the cell as currently drawn —
    /// the vivid recolor against the cell's true appearance, matching the
    /// selection display behavior.
    pub fn lasso_select_preview_cells(
        &self,
        canvas: &Canvas,
        path: &[CellPoint],
        hand: PaintHand,
        orientation: CameraViewOrientation,
    ) -> Vec<crate::lasso_stroke::LassoPreviewCell> {
        self.lasso_selection_points(path, hand, orientation)
            .into_iter()
            .map(|point| {
                let current = canvas.get(&point).cloned();
                let upcoming = current.clone().unwrap_or(PaintedCell {
                    graphic: CellGraphic::Glyph(' '),
                    color: PaintColor::flat_rgb(0, 0, 0),
                    weight_index: 3,
                });
                crate::lasso_stroke::LassoPreviewCell {
                    point,
                    current,
                    upcoming,
                }
            })
            .collect()
    }

    /// The stamp tool's edit half: the copied world cells placed at `anchor`,
    /// filtered by the current selection. Enabled edit channels paste the
    /// copied cell's own graphic/color/weight (the copy is the content);
    /// locked channels keep the target cell's existing value so the stamp
    /// obeys the locked graphic/color/weight of the acting hand like
    /// brush/fill do — with the brush blank-glyph rule that a gfx-locked
    /// stamp never places a glyph on an empty target cell. Returns
    /// (point, upcoming) pairs; the canvas-pointer press dispatch stages
    /// them so one stamp is one undo step.
    pub fn stamp_changes_for_hand(
        &self,
        canvas: &Canvas,
        selection: &PainterSelection,
        data: &WorldCopyData,
        anchor: CellPoint,
        hand: PaintHand,
    ) -> Vec<(CellPoint, PaintedCell)> {
        let hand_state = self.hand_state(hand);
        if !hand_state.edit_channels.any_enabled() {
            return Vec::new();
        }
        data.paste_points(anchor)
            .into_iter()
            .filter(|(point, _)| selection.allows_plane_edit(*point))
            .filter_map(|(point, copied)| {
                let existing = brush::effective_cell(canvas.get(&point));
                // Source-of-truth from J: a stamp with any locked channel
                // edits nothing on an empty target — a locked channel has
                // no existing value to keep there, so the point is skipped
                // entirely (the gfx-locked case generalized).
                if !hand_state.edit_channels.all_enabled() && existing.is_none() {
                    return None;
                }
                let upcoming = PaintedCell {
                    graphic: if hand_state.edit_channels.graphic {
                        copied.graphic.clone()
                    } else {
                        // Brush-like blank-glyph rule: a gfx-locked stamp
                        // never places a glyph on an empty target cell.
                        existing
                            .map(|cell| cell.graphic.clone())
                            .unwrap_or(CellGraphic::Glyph(' '))
                    },
                    color: if hand_state.edit_channels.color {
                        copied.color
                    } else {
                        existing.map(|cell| cell.color).unwrap_or(copied.color)
                    },
                    weight_index: if hand_state.edit_channels.weight {
                        copied.weight_index
                    } else {
                        existing
                            .map(|cell| cell.weight_index)
                            .unwrap_or(copied.weight_index)
                    },
                };
                Some((point, upcoming))
            })
            .collect()
    }

    /// Per-cell preview data for the stamp at `anchor`: the exact cells the
    /// press will place (through the same resolution `stamp_changes_for_hand`
    /// uses), each paired with the cell as currently drawn — the two flash
    /// halves of the paste-area preview.
    pub fn stamp_preview_cells(
        &self,
        canvas: &Canvas,
        selection: &PainterSelection,
        data: &WorldCopyData,
        anchor: CellPoint,
        hand: PaintHand,
    ) -> Vec<crate::lasso_stroke::LassoPreviewCell> {
        self.stamp_changes_for_hand(canvas, selection, data, anchor, hand)
            .into_iter()
            .map(|(point, upcoming)| crate::lasso_stroke::LassoPreviewCell {
                current: canvas.get(&point).cloned(),
                point,
                upcoming,
            })
            .collect()
    }

    /// Applies the acting hand's assigned tool at `position`, routing either into image edits or
    /// live selection edits based on that hand's current target.
    pub fn apply_at_for_hand(
        &mut self,
        canvas: &mut Canvas,
        selection: &mut PainterSelection,
        position: CellPoint,
        hand: PaintHand,
        bounds: CanvasBounds,
        orientation: CameraViewOrientation,
    ) {
        self.active_hand = hand;
        match self.hand_state(hand).target {
            PaintTarget::Image => self.apply_canvas_at_for_hand(
                canvas,
                selection,
                position,
                hand,
                bounds,
                orientation,
            ),
            PaintTarget::Selection => self.apply_selection_at_for_hand(
                canvas,
                selection,
                position,
                hand,
                bounds,
                orientation,
            ),
        }
    }
}

fn nudge_step(step: &mut (i32, i32, i32), axis: usize, delta: i32) {
    match axis {
        0 => step.0 += delta,
        1 => step.1 += delta,
        2 => step.2 += delta,
        _ => {}
    }
}

fn set_step_axis(step: &mut (i32, i32, i32), axis: usize, value: i32) {
    match axis {
        0 => step.0 = value,
        1 => step.1 = value,
        2 => step.2 = value,
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use thaum_renderer_domain::{camera_view_orientation_for_camera, CameraRoll, CameraSwing};

    fn flat_view() -> thaum_renderer_domain::CameraViewOrientation {
        camera_view_orientation_for_camera(CameraSwing::PosZ, CameraRoll::Deg0)
    }

    fn point(x: i32, y: i32) -> CellPoint {
        CellPoint { x, y, z: 0 }
    }

    fn bounds() -> CanvasBounds {
        CanvasBounds {
            x0: 0,
            y0: 0,
            x1: 8,
            y1: 8,
            z: 0,
            plane_axis: crate::fill::CanvasPlaneAxis::Z,
        }
    }

    fn selection() -> PainterSelection {
        PainterSelection::new(bounds())
    }

    fn color(red: u8, green: u8, blue: u8) -> PaintColor {
        PaintColor::flat_rgb(red, green, blue)
    }

    #[test]
    fn default_tool_state_assigns_brush_left_and_erase_right() {
        let tool_state = ToolState::default();
        assert_eq!(tool_state.left_tool, PaintTool::Brush);
        assert_eq!(tool_state.right_tool, PaintTool::Erase);
        assert_eq!(tool_state.left_hand.graphic, CellGraphic::Glyph('#'));
        assert_eq!(tool_state.left_hand.brush_size, 1);
        assert!(!tool_state.left_hand.fill_diagonal);
        assert_eq!(tool_state.left_hand.target, PaintTarget::Image);
    }

    #[test]
    fn set_tool_for_hand_changes_only_that_hand_and_marks_it_active() {
        let mut tool_state = ToolState::default();
        tool_state.set_tool_for_hand(PaintHand::Right, PaintTool::Fill);
        assert_eq!(tool_state.left_tool, PaintTool::Brush);
        assert_eq!(tool_state.right_tool, PaintTool::Fill);
        assert_eq!(tool_state.active_hand, PaintHand::Right);
    }

    #[test]
    fn each_tool_declares_its_own_property_rows() {
        assert_eq!(PaintTool::Brush.property_row_ids(), &["brush_size"]);
        assert_eq!(PaintTool::Erase.property_row_ids(), &["brush_size"]);
        assert_eq!(PaintTool::Fill.property_row_ids(), &["fill_diagonal"]);
        assert_eq!(
            PaintTool::Text.property_row_ids(),
            &["text_char_step", "text_enter_step"]
        );
        assert_eq!(
            PaintTool::Picker.property_row_ids(),
            &["picker_opposite_hand"]
        );
    }

    #[test]
    fn every_tool_maps_to_a_registered_descriptor() {
        assert_eq!(PaintTool::all().len(), crate::painter_tools::all().len());
        for tool in PaintTool::all() {
            let descriptor = crate::painter_tools::require_by_id(tool.id());
            assert_eq!(descriptor.property_row_ids, tool.property_row_ids());
            assert!(PaintTool::from_id(descriptor.id) == Some(tool));
        }
        assert!(PaintTool::from_id("nonexistent").is_none());
    }

    #[test]
    fn picker_samples_all_channels_of_the_picking_hand_when_edit_toggles_are_open() {
        let mut tool_state = ToolState::default();
        tool_state.set_tool_for_hand(PaintHand::Left, PaintTool::Picker);
        let mut canvas = Canvas::new();
        brush::apply_brush(
            &mut canvas,
            point(3, 3),
            PaintedCell {
                graphic: CellGraphic::Glyph('&'),
                color: color(9, 8, 7),
                weight_index: 1,
            },
        );

        tool_state.pick_at_for_hand(&mut canvas, point(3, 3), PaintHand::Left);

        let hand = tool_state.left_hand;
        assert_eq!(hand.graphic, CellGraphic::Glyph('&'));
        assert_eq!(hand.color, color(9, 8, 7));
        assert_eq!(hand.weight_index, 1);
    }

    #[test]
    fn picker_only_samples_the_channels_the_picking_hand_has_edit_toggled() {
        let mut tool_state = ToolState::default();
        tool_state.set_tool_for_hand(PaintHand::Left, PaintTool::Picker);
        tool_state.toggle_edit_channel_for_hand(PaintHand::Left, PaintChannel::Graphic);
        let mut canvas = Canvas::new();
        brush::apply_brush(
            &mut canvas,
            point(3, 3),
            PaintedCell {
                graphic: CellGraphic::Glyph('&'),
                color: color(9, 8, 7),
                weight_index: 1,
            },
        );

        tool_state.pick_at_for_hand(&mut canvas, point(3, 3), PaintHand::Left);

        let hand = tool_state.left_hand;
        assert_eq!(hand.graphic, CellGraphic::Glyph('#'));
        assert_eq!(hand.color, color(9, 8, 7));
        assert_eq!(hand.weight_index, 1);
    }

    #[test]
    fn picker_with_opposite_hand_toggled_hands_the_gated_sample_to_the_other_hand() {
        let mut tool_state = ToolState::default();
        tool_state.set_tool_for_hand(PaintHand::Left, PaintTool::Picker);
        tool_state.set_pick_opposite_hand_for_hand(PaintHand::Left, true);
        tool_state.toggle_edit_channel_for_hand(PaintHand::Left, PaintChannel::Graphic);
        let mut canvas = Canvas::new();
        brush::apply_brush(
            &mut canvas,
            point(3, 3),
            PaintedCell {
                graphic: CellGraphic::Glyph('&'),
                color: color(9, 8, 7),
                weight_index: 1,
            },
        );

        tool_state.pick_at_for_hand(&mut canvas, point(3, 3), PaintHand::Left);

        let right = tool_state.right_hand;
        assert_eq!(right.graphic, CellGraphic::Glyph('#'));
        assert_eq!(right.color, color(9, 8, 7));
        assert_eq!(right.weight_index, 1);
        let left = tool_state.left_hand;
        assert_eq!(left.color, PaintColor::default());
    }

    #[test]
    fn apply_at_for_left_brush_paints_a_cell_matching_that_hands_state() {
        let mut tool_state = ToolState::default();
        tool_state.set_graphic_for_hand(PaintHand::Left, CellGraphic::Glyph('@'));
        tool_state.set_color_for_hand(PaintHand::Left, color(10, 20, 30));
        let mut canvas = Canvas::new();
        let mut selection = selection();

        tool_state.apply_at_for_hand(
            &mut canvas,
            &mut selection,
            point(4, 4),
            PaintHand::Left,
            bounds(),
            flat_view(),
        );

        let painted = canvas.get(&point(4, 4)).unwrap();
        assert_eq!(painted.graphic, CellGraphic::Glyph('@'));
        assert_eq!(painted.color, color(10, 20, 30));
    }

    #[test]
    fn apply_at_for_right_erase_removes_an_existing_cell() {
        let mut tool_state = ToolState::default();
        let mut canvas = Canvas::new();
        let mut selection = selection();
        tool_state.apply_at_for_hand(
            &mut canvas,
            &mut selection,
            point(2, 2),
            PaintHand::Left,
            bounds(),
            flat_view(),
        );

        tool_state.apply_at_for_hand(
            &mut canvas,
            &mut selection,
            point(2, 2),
            PaintHand::Right,
            bounds(),
            flat_view(),
        );

        assert!(canvas.get(&point(2, 2)).is_none());
    }

    #[test]
    fn brush_size_expands_brush_and_erase_through_the_shared_brush_footprint() {
        let mut tool_state = ToolState::default();
        tool_state.set_brush_size_for_hand(PaintHand::Left, 2);
        tool_state.set_brush_size_for_hand(PaintHand::Right, 2);
        let mut canvas = Canvas::new();
        let mut selection = selection();

        tool_state.apply_at_for_hand(
            &mut canvas,
            &mut selection,
            point(4, 4),
            PaintHand::Left,
            bounds(),
            flat_view(),
        );
        assert_eq!(canvas.len(), 4);

        tool_state.apply_at_for_hand(
            &mut canvas,
            &mut selection,
            point(4, 4),
            PaintHand::Right,
            bounds(),
            flat_view(),
        );
        assert!(canvas.is_empty());
    }

    #[test]
    fn fill_replaces_the_whole_contiguous_region_with_that_hands_state() {
        let mut tool_state = ToolState::default();
        tool_state.set_tool_for_hand(PaintHand::Left, PaintTool::Fill);
        tool_state.set_graphic_for_hand(PaintHand::Left, CellGraphic::Glyph('.'));
        let mut canvas = Canvas::new();
        let mut selection = selection();
        brush::apply_brush(
            &mut canvas,
            point(2, 2),
            PaintedCell {
                graphic: CellGraphic::Glyph('#'),
                color: color(255, 255, 255),
                weight_index: 1,
            },
        );

        tool_state.apply_at_for_hand(
            &mut canvas,
            &mut selection,
            point(0, 0),
            PaintHand::Left,
            bounds(),
            flat_view(),
        );

        assert_eq!(
            canvas.get(&point(0, 0)).unwrap().graphic,
            CellGraphic::Glyph('.')
        );
        assert_eq!(
            canvas.get(&point(1, 0)).unwrap().graphic,
            CellGraphic::Glyph('.')
        );
        assert_eq!(
            canvas.get(&point(2, 2)).unwrap().graphic,
            CellGraphic::Glyph('#')
        );
    }

    #[test]
    fn masked_brush_preserves_existing_unmasked_channels() {
        let mut tool_state = ToolState::default();
        tool_state.set_graphic_for_hand(PaintHand::Left, CellGraphic::Glyph('@'));
        tool_state.left_hand.edit_channels.color = false;
        tool_state.left_hand.edit_channels.weight = false;
        let mut canvas = Canvas::new();
        let mut selection = selection();
        brush::apply_brush(
            &mut canvas,
            point(1, 1),
            PaintedCell {
                graphic: CellGraphic::Glyph('#'),
                color: color(1, 2, 3),
                weight_index: 0,
            },
        );

        tool_state.apply_at_for_hand(
            &mut canvas,
            &mut selection,
            point(1, 1),
            PaintHand::Left,
            bounds(),
            flat_view(),
        );

        assert_eq!(
            canvas.get(&point(1, 1)),
            Some(&PaintedCell {
                graphic: CellGraphic::Glyph('@'),
                color: color(1, 2, 3),
                weight_index: 0,
            })
        );
    }

    #[test]
    fn masked_brush_on_an_empty_cell_leaves_the_cell_empty() {
        let mut tool_state = ToolState::default();
        tool_state.left_hand.edit_channels.graphic = false;
        tool_state.left_hand.edit_channels.color = true;
        tool_state.left_hand.edit_channels.weight = false;
        tool_state.set_graphic_for_hand(PaintHand::Left, CellGraphic::Glyph('@'));
        tool_state.set_color_for_hand(PaintHand::Left, color(9, 8, 7));
        let mut canvas = Canvas::new();
        let mut selection = selection();

        tool_state.apply_at_for_hand(
            &mut canvas,
            &mut selection,
            point(2, 2),
            PaintHand::Left,
            bounds(),
            flat_view(),
        );

        // Unified empty-cell rule: a masked resolution on an empty cell is
        // an authored blank, and blanks carry nothing — nothing is stored.
        assert_eq!(canvas.get(&point(2, 2)), None);
    }

    #[test]
    fn any_locked_channel_blocks_placing_on_empty_cells() {
        // Source-of-truth from J: with any of the three channels locked, a
        // stroke must not place on empty cells — only an all-unlocked hand
        // authors new cells. A locked channel has no existing value to keep
        // on an empty cell, so a partial edit cannot resolve there.
        let mut tool_state = ToolState::default();
        tool_state.left_hand.edit_channels.color = false;
        tool_state.set_graphic_for_hand(PaintHand::Left, CellGraphic::Glyph('@'));
        tool_state.set_color_for_hand(PaintHand::Left, color(9, 8, 7));
        let mut canvas = Canvas::new();
        let mut selection = selection();

        tool_state.apply_at_for_hand(
            &mut canvas,
            &mut selection,
            point(2, 2),
            PaintHand::Left,
            bounds(),
            flat_view(),
        );

        assert_eq!(canvas.get(&point(2, 2)), None);

        // The same stroke still restyles an existing cell in place: the
        // color-locked resolution keeps the target's color and paints the
        // hand's graphic and weight onto it.
        brush::apply_brush(
            &mut canvas,
            point(3, 3),
            PaintedCell {
                graphic: CellGraphic::Glyph('A'),
                color: color(1, 1, 1),
                weight_index: 0,
            },
        );
        tool_state.apply_at_for_hand(
            &mut canvas,
            &mut selection,
            point(3, 3),
            PaintHand::Left,
            bounds(),
            flat_view(),
        );
        assert_eq!(
            canvas.get(&point(3, 3)).unwrap().graphic,
            CellGraphic::Glyph('@')
        );
        assert_eq!(canvas.get(&point(3, 3)).unwrap().color, color(1, 1, 1));
    }

    #[test]
    fn fully_unlocked_hands_still_place_on_empty_cells() {
        let mut tool_state = ToolState::default();
        tool_state.set_graphic_for_hand(PaintHand::Left, CellGraphic::Glyph('@'));
        let mut canvas = Canvas::new();
        let mut selection = selection();

        tool_state.apply_at_for_hand(
            &mut canvas,
            &mut selection,
            point(2, 2),
            PaintHand::Left,
            bounds(),
            flat_view(),
        );

        assert_eq!(
            canvas.get(&point(2, 2)).unwrap().graphic,
            CellGraphic::Glyph('@')
        );
    }

    #[test]
    fn fill_diagonal_can_cross_corner_neighbors() {
        let mut tool_state = ToolState::default();
        tool_state.set_tool_for_hand(PaintHand::Left, PaintTool::Fill);
        tool_state.set_fill_diagonal_for_hand(PaintHand::Left, true);
        tool_state.set_graphic_for_hand(PaintHand::Left, CellGraphic::Glyph('.'));
        let mut canvas = Canvas::new();
        let mut selection = selection();
        brush::apply_brush(
            &mut canvas,
            point(0, 0),
            PaintedCell {
                graphic: CellGraphic::Glyph('A'),
                color: color(1, 1, 1),
                weight_index: 1,
            },
        );
        brush::apply_brush(
            &mut canvas,
            point(1, 1),
            PaintedCell {
                graphic: CellGraphic::Glyph('A'),
                color: color(1, 1, 1),
                weight_index: 1,
            },
        );

        tool_state.apply_at_for_hand(
            &mut canvas,
            &mut selection,
            point(0, 0),
            PaintHand::Left,
            bounds(),
            flat_view(),
        );

        assert_eq!(
            canvas.get(&point(0, 0)).unwrap().graphic,
            CellGraphic::Glyph('.')
        );
        assert_eq!(
            canvas.get(&point(1, 1)).unwrap().graphic,
            CellGraphic::Glyph('.')
        );
    }

    #[test]
    fn fill_respects_the_channel_mask_per_filled_cell() {
        let mut tool_state = ToolState::default();
        tool_state.set_tool_for_hand(PaintHand::Left, PaintTool::Fill);
        tool_state.left_hand.edit_channels.graphic = false;
        tool_state.left_hand.edit_channels.color = true;
        tool_state.left_hand.edit_channels.weight = false;
        tool_state.set_color_for_hand(PaintHand::Left, color(9, 8, 7));
        let mut canvas = Canvas::new();
        let mut selection = selection();
        brush::apply_brush(
            &mut canvas,
            point(0, 0),
            PaintedCell {
                graphic: CellGraphic::Glyph('A'),
                color: color(1, 1, 1),
                weight_index: 0,
            },
        );
        brush::apply_brush(
            &mut canvas,
            point(1, 0),
            PaintedCell {
                graphic: CellGraphic::Glyph('B'),
                color: color(1, 1, 1),
                weight_index: 3,
            },
        );

        tool_state.apply_at_for_hand(
            &mut canvas,
            &mut selection,
            point(0, 0),
            PaintHand::Left,
            bounds(),
            flat_view(),
        );

        assert_eq!(
            canvas.get(&point(0, 0)).unwrap().graphic,
            CellGraphic::Glyph('A')
        );
        assert_eq!(
            canvas.get(&point(1, 0)).unwrap().graphic,
            CellGraphic::Glyph('B')
        );
        assert_eq!(canvas.get(&point(0, 0)).unwrap().color, color(9, 8, 7));
        assert_eq!(canvas.get(&point(1, 0)).unwrap().weight_index, 3);
    }

    #[test]
    fn material_assignment_paints_material_backed_cells() {
        let mut tool_state = ToolState::default();
        tool_state.set_material_for_hand(PaintHand::Left, CellMaterialId::GrayScale);
        let mut canvas = Canvas::new();
        let mut selection = selection();

        tool_state.apply_at_for_hand(
            &mut canvas,
            &mut selection,
            point(4, 4),
            PaintHand::Left,
            bounds(),
            flat_view(),
        );

        assert_eq!(
            canvas.get(&point(4, 4)).unwrap().color,
            PaintColor::material(CellMaterialId::GrayScale)
        );
    }

    #[test]
    fn selection_target_brush_adds_a_selected_cell_without_painting_canvas() {
        let mut tool_state = ToolState::default();
        tool_state.set_target_for_hand(PaintHand::Left, PaintTarget::Selection);
        let mut canvas = Canvas::new();
        let mut selection = selection();

        tool_state.apply_at_for_hand(
            &mut canvas,
            &mut selection,
            point(3, 3),
            PaintHand::Left,
            bounds(),
            flat_view(),
        );

        assert!(selection.plane().contains(point(3, 3)));
        assert!(canvas.is_empty());
    }

    #[test]
    fn image_target_brush_only_edits_cells_inside_the_current_selection() {
        let mut tool_state = ToolState::default();
        tool_state.set_graphic_for_hand(PaintHand::Left, CellGraphic::Glyph('@'));
        let mut canvas = Canvas::new();
        let mut selection = selection();
        selection.apply_plane_points([point(1, 1)]);
        brush::apply_brush(
            &mut canvas,
            point(0, 0),
            PaintedCell {
                graphic: CellGraphic::Glyph('#'),
                color: color(1, 1, 1),
                weight_index: 1,
            },
        );
        brush::apply_brush(
            &mut canvas,
            point(1, 1),
            PaintedCell {
                graphic: CellGraphic::Glyph('#'),
                color: color(1, 1, 1),
                weight_index: 1,
            },
        );

        tool_state.set_brush_size_for_hand(PaintHand::Left, 2);
        tool_state.apply_at_for_hand(
            &mut canvas,
            &mut selection,
            point(0, 0),
            PaintHand::Left,
            bounds(),
            flat_view(),
        );

        assert_eq!(
            canvas.get(&point(0, 0)).unwrap().graphic,
            CellGraphic::Glyph('#')
        );
        assert_eq!(
            canvas.get(&point(1, 1)).unwrap().graphic,
            CellGraphic::Glyph('@')
        );
    }

    #[test]
    fn image_target_fill_only_edits_cells_inside_the_current_selection() {
        let mut tool_state = ToolState::default();
        tool_state.set_tool_for_hand(PaintHand::Left, PaintTool::Fill);
        tool_state.set_graphic_for_hand(PaintHand::Left, CellGraphic::Glyph('.'));
        let mut canvas = Canvas::new();
        let mut selection = selection();
        selection.set_mode(SelectionMode::Additive);
        selection.apply_plane_points([point(0, 0), point(1, 0)]);
        brush::apply_brush(
            &mut canvas,
            point(0, 0),
            PaintedCell {
                graphic: CellGraphic::Glyph('A'),
                color: color(1, 1, 1),
                weight_index: 1,
            },
        );
        brush::apply_brush(
            &mut canvas,
            point(1, 0),
            PaintedCell {
                graphic: CellGraphic::Glyph('A'),
                color: color(1, 1, 1),
                weight_index: 1,
            },
        );
        brush::apply_brush(
            &mut canvas,
            point(2, 0),
            PaintedCell {
                graphic: CellGraphic::Glyph('A'),
                color: color(1, 1, 1),
                weight_index: 1,
            },
        );

        tool_state.apply_at_for_hand(
            &mut canvas,
            &mut selection,
            point(0, 0),
            PaintHand::Left,
            bounds(),
            flat_view(),
        );

        assert_eq!(
            canvas.get(&point(0, 0)).unwrap().graphic,
            CellGraphic::Glyph('.')
        );
        assert_eq!(
            canvas.get(&point(1, 0)).unwrap().graphic,
            CellGraphic::Glyph('.')
        );
        assert_eq!(
            canvas.get(&point(2, 0)).unwrap().graphic,
            CellGraphic::Glyph('A')
        );
    }

    #[test]
    fn selection_target_fill_senses_regions_through_the_select_channel_mask() {
        let mut tool_state = ToolState::default();
        tool_state.set_target_for_hand(PaintHand::Left, PaintTarget::Selection);
        tool_state.set_tool_for_hand(PaintHand::Left, PaintTool::Fill);
        tool_state.left_hand.select_channels = ChannelMask {
            graphic: true,
            color: false,
            weight: false,
        };
        let mut canvas = Canvas::new();
        let mut selection = selection();
        brush::apply_brush(
            &mut canvas,
            point(0, 0),
            PaintedCell {
                graphic: CellGraphic::Glyph('A'),
                color: color(1, 1, 1),
                weight_index: 0,
            },
        );
        brush::apply_brush(
            &mut canvas,
            point(1, 0),
            PaintedCell {
                graphic: CellGraphic::Glyph('A'),
                color: color(9, 9, 9),
                weight_index: 3,
            },
        );
        brush::apply_brush(
            &mut canvas,
            point(2, 0),
            PaintedCell {
                graphic: CellGraphic::Glyph('B'),
                color: color(1, 1, 1),
                weight_index: 0,
            },
        );

        tool_state.apply_at_for_hand(
            &mut canvas,
            &mut selection,
            point(0, 0),
            PaintHand::Left,
            bounds(),
            flat_view(),
        );

        assert!(selection.plane().contains(point(0, 0)));
        assert!(selection.plane().contains(point(1, 0)));
        assert!(!selection.plane().contains(point(2, 0)));
    }

    #[test]
    fn image_target_fill_ignores_graphic_differences_when_the_graphic_channel_is_unlocked() {
        let mut tool_state = ToolState::default();
        tool_state.set_target_for_hand(PaintHand::Left, PaintTarget::Image);
        tool_state.set_tool_for_hand(PaintHand::Left, PaintTool::Fill);
        // Color and weight locked on, graphic unlocked: two adjacent cells
        // sharing color and weight but differing in graphic flood together.
        tool_state.left_hand.select_channels = ChannelMask {
            graphic: false,
            color: true,
            weight: true,
        };
        let mut canvas = Canvas::new();
        let mut selection = selection();
        // J's scenario: cell a and cell b share color and weight but differ
        // in graphic; cell c differs in color and still bounds the flood.
        brush::apply_brush(
            &mut canvas,
            point(0, 0),
            PaintedCell {
                graphic: CellGraphic::Glyph('A'),
                color: color(1, 1, 1),
                weight_index: 0,
            },
        );
        brush::apply_brush(
            &mut canvas,
            point(1, 0),
            PaintedCell {
                graphic: CellGraphic::Glyph('B'),
                color: color(1, 1, 1),
                weight_index: 0,
            },
        );
        brush::apply_brush(
            &mut canvas,
            point(2, 0),
            PaintedCell {
                graphic: CellGraphic::Glyph('B'),
                color: color(9, 9, 9),
                weight_index: 0,
            },
        );

        let before = canvas.get(&point(1, 0)).cloned();
        tool_state.apply_at_for_hand(
            &mut canvas,
            &mut selection,
            point(0, 0),
            PaintHand::Left,
            bounds(),
            flat_view(),
        );

        // The flood crossed the graphic difference: cell (1,0) changed...
        assert_ne!(canvas.get(&point(1, 0)), before.as_ref());
        // ...while the graphic difference at (2,0) still bounds the fill.
        assert_eq!(
            canvas.get(&point(2, 0)).map(|cell| cell.graphic.clone()),
            Some(CellGraphic::Glyph('B'))
        );
    }

    #[test]
    fn lasso_edit_points_match_what_release_will_paint() {
        // The in-progress preview consumes this seam: the points must be the
        // enclosed region gated by selection and edit channels — exactly what
        // apply_lasso_for_hand paints on release.
        let mut tool_state = ToolState::default();
        tool_state.set_tool_for_hand(PaintHand::Left, PaintTool::Lasso);
        let selection = selection();
        let path = [point(0, 0), point(2, 0), point(2, 2), point(0, 2)];

        let points = tool_state.lasso_edit_points(&selection, &path, PaintHand::Left, flat_view());
        assert_eq!(points.len(), 9);
        assert!(points.contains(&point(1, 1)));

        tool_state.left_hand.edit_channels = ChannelMask {
            graphic: false,
            color: false,
            weight: false,
        };
        assert!(tool_state
            .lasso_edit_points(&selection, &path, PaintHand::Left, flat_view())
            .is_empty());
    }

    #[test]
    fn lasso_fills_its_enclosed_region_with_the_hands_state() {
        let mut tool_state = ToolState::default();
        tool_state.set_tool_for_hand(PaintHand::Left, PaintTool::Lasso);
        tool_state.set_graphic_for_hand(PaintHand::Left, CellGraphic::Glyph('.'));
        let mut canvas = Canvas::new();
        let mut selection = selection();
        brush::apply_brush(
            &mut canvas,
            point(1, 1),
            PaintedCell {
                graphic: CellGraphic::Glyph('#'),
                color: color(255, 255, 255),
                weight_index: 1,
            },
        );

        tool_state.apply_lasso_for_hand(
            &mut canvas,
            &selection,
            &[point(0, 0), point(2, 0), point(2, 2), point(0, 2)],
            PaintHand::Left,
            flat_view(),
        );

        assert_eq!(
            canvas.get(&point(0, 0)).unwrap().graphic,
            CellGraphic::Glyph('.')
        );
        assert_eq!(
            canvas.get(&point(1, 1)).unwrap().graphic,
            CellGraphic::Glyph('.')
        );
        assert!(canvas.get(&point(3, 3)).is_none());
    }

    #[test]
    fn lasso_only_fills_cells_inside_the_current_selection() {
        let mut tool_state = ToolState::default();
        tool_state.set_tool_for_hand(PaintHand::Left, PaintTool::Lasso);
        tool_state.set_graphic_for_hand(PaintHand::Left, CellGraphic::Glyph('.'));
        let mut canvas = Canvas::new();
        let mut selection = selection();
        selection.set_mode(SelectionMode::Additive);
        selection.apply_plane_points([point(0, 0), point(1, 0)]);

        tool_state.apply_lasso_for_hand(
            &mut canvas,
            &selection,
            &[point(0, 0), point(2, 0), point(2, 2), point(0, 2)],
            PaintHand::Left,
            flat_view(),
        );

        assert_eq!(
            canvas.get(&point(0, 0)).unwrap().graphic,
            CellGraphic::Glyph('.')
        );
        assert_eq!(
            canvas.get(&point(1, 0)).unwrap().graphic,
            CellGraphic::Glyph('.')
        );
        assert!(canvas.get(&point(2, 1)).is_none());
    }

    #[test]
    fn lasso_fill_flows_through_the_channel_mask_and_empty_cell_rule() {
        let mut tool_state = ToolState::default();
        tool_state.set_tool_for_hand(PaintHand::Left, PaintTool::Lasso);
        tool_state.left_hand.edit_channels.graphic = false;
        tool_state.left_hand.edit_channels.color = true;
        tool_state.left_hand.edit_channels.weight = false;
        tool_state.set_color_for_hand(PaintHand::Left, color(9, 8, 7));
        let mut canvas = Canvas::new();
        let mut selection = selection();
        brush::apply_brush(
            &mut canvas,
            point(0, 0),
            PaintedCell {
                graphic: CellGraphic::Glyph('A'),
                color: color(1, 1, 1),
                weight_index: 0,
            },
        );

        tool_state.apply_lasso_for_hand(
            &mut canvas,
            &selection,
            &[point(0, 0), point(1, 0), point(1, 1), point(0, 1)],
            PaintHand::Left,
            flat_view(),
        );

        assert_eq!(
            canvas.get(&point(0, 0)),
            Some(&PaintedCell {
                graphic: CellGraphic::Glyph('A'),
                color: color(9, 8, 7),
                weight_index: 0,
            })
        );
        // Empty cells under a gfx-locked fill stay truly empty: a space
        // glyph carries no color or weight to recolor.
        assert_eq!(canvas.get(&point(1, 1)), None);
    }

    #[test]
    fn lasso_selection_points_return_the_enclosed_region_gated_by_select_channels() {
        let mut tool_state = ToolState::default();
        tool_state.set_tool_for_hand(PaintHand::Left, PaintTool::Lasso);
        let path = [point(0, 0), point(2, 0), point(2, 2), point(0, 2)];

        let points = tool_state.lasso_selection_points(&path, PaintHand::Left, flat_view());
        assert!(points.contains(&point(1, 1)));
        assert_eq!(points.len(), 9);

        tool_state.left_hand.select_channels = ChannelMask {
            graphic: false,
            color: false,
            weight: false,
        };
        assert!(tool_state
            .lasso_selection_points(&path, PaintHand::Left, flat_view())
            .is_empty());
    }

    #[test]
    fn nudging_text_steps_stays_shared_across_hands_and_clamped() {
        let mut tool_state = ToolState::default();

        tool_state.nudge_text_char_step(0, 3);
        assert_eq!(tool_state.text_options.char_step, (4, 0, 0));

        tool_state.nudge_text_enter_step(1, -4);
        assert_eq!(tool_state.text_options.enter_step, (0, -3, 0));

        for _ in 0..20 {
            tool_state.nudge_text_char_step(2, 2);
        }
        assert_eq!(tool_state.text_options.char_step.2, 9);

        tool_state.set_text_enter_step_axis(0, -9);
        assert_eq!(tool_state.text_options.enter_step, (-9, -3, 0));
    }

    use crate::clipboard::WorldCopyData;
    use std::collections::BTreeMap;

    fn copy_data(offsets: &[(i32, i32, char)]) -> WorldCopyData {
        let mut cells = BTreeMap::new();
        for &(x, y, glyph) in offsets {
            cells.insert(
                CellPoint { x, y, z: 0 },
                PaintedCell {
                    graphic: CellGraphic::Glyph(glyph),
                    color: color(255, 255, 255),
                    weight_index: 1,
                },
            );
        }
        WorldCopyData {
            anchor: CellPoint { x: 0, y: 0, z: 0 },
            center: CellPoint { x: 0, y: 0, z: 0 },
            cells,
        }
    }

    #[test]
    fn stamp_changes_paste_the_copied_cells_own_appearance() {
        let mut tool_state = ToolState::default();
        tool_state.set_graphic_for_hand(PaintHand::Left, CellGraphic::Glyph('.'));
        tool_state.set_color_for_hand(PaintHand::Left, color(200, 0, 0));
        tool_state.set_weight_for_hand(PaintHand::Left, 3);
        let canvas = Canvas::new();

        let changes = tool_state.stamp_changes_for_hand(
            &canvas,
            &selection(),
            &copy_data(&[(0, 0, 'a'), (1, 0, 'a')]),
            point(5, 5),
            PaintHand::Left,
        );

        // The copy supplies the structure AND the appearance: the hand's
        // brush state never leaks into a stamp's pasted cells.
        assert_eq!(changes.len(), 2);
        for (point, upcoming) in &changes {
            assert_eq!(upcoming.graphic, CellGraphic::Glyph('a'));
            assert_eq!(upcoming.color, color(255, 255, 255));
            assert_eq!(upcoming.weight_index, 1);
            let _ = point;
        }
    }

    #[test]
    fn locked_stamp_channels_keep_the_target_cells() {
        let mut tool_state = ToolState::default();
        tool_state.set_graphic_for_hand(PaintHand::Left, CellGraphic::Glyph('.'));
        tool_state.set_color_for_hand(PaintHand::Left, color(200, 0, 0));
        tool_state.set_weight_for_hand(PaintHand::Left, 3);
        // A color-locked hand restyles glyphs but keeps existing colors.
        tool_state
            .hand_state_mut(PaintHand::Left)
            .edit_channels
            .color = false;
        let mut canvas = Canvas::new();
        brush::apply_brush(
            &mut canvas,
            point(5, 5),
            PaintedCell {
                graphic: CellGraphic::Glyph('a'),
                color: color(10, 10, 10),
                weight_index: 2,
            },
        );

        let changes = tool_state.stamp_changes_for_hand(
            &canvas,
            &selection(),
            &copy_data(&[(0, 0, 'a')]),
            point(5, 5),
            PaintHand::Left,
        );

        let (_, upcoming) = &changes[0];
        // Locked color keeps the target's existing color; the enabled
        // graphic channel pastes the copied glyph ('a'), not the hand's.
        assert_eq!(upcoming.graphic, CellGraphic::Glyph('a'));
        assert_eq!(upcoming.color, color(10, 10, 10));
    }

    #[test]
    fn gfx_locked_stamp_skips_empty_target_cells() {
        let mut tool_state = ToolState::default();
        tool_state
            .hand_state_mut(PaintHand::Left)
            .edit_channels
            .graphic = false;
        let canvas = Canvas::new();

        let changes = tool_state.stamp_changes_for_hand(
            &canvas,
            &selection(),
            &copy_data(&[(0, 0, 'a')]),
            point(5, 5),
            PaintHand::Left,
        );

        // Unified empty-cell rule: a space carries no color or weight, so a
        // gfx-locked stamp has nothing to edit on an empty target — the
        // point is skipped entirely instead of staging an authored blank.
        assert!(changes.is_empty());
    }

    #[test]
    fn any_locked_channel_stamp_skips_empty_target_cells() {
        // Source-of-truth from J: one locked channel of any kind is enough
        // to block stamping onto empty cells.
        let mut tool_state = ToolState::default();
        tool_state
            .hand_state_mut(PaintHand::Left)
            .edit_channels
            .weight = false;
        let canvas = Canvas::new();

        let changes = tool_state.stamp_changes_for_hand(
            &canvas,
            &selection(),
            &copy_data(&[(0, 0, 'a')]),
            point(5, 5),
            PaintHand::Left,
        );

        assert!(changes.is_empty());
    }

    #[test]
    fn stamp_changes_stay_inside_the_plane_selection() {
        let tool_state = ToolState::default();
        let mut plane = selection();
        plane.apply_plane_points_with_mode([point(5, 5)], SelectionMode::Replace);

        let changes = tool_state.stamp_changes_for_hand(
            &Canvas::new(),
            &plane,
            &copy_data(&[(0, 0, 'a'), (1, 0, 'b')]),
            point(5, 5),
            PaintHand::Left,
        );

        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].0, point(5, 5));
    }

    #[test]
    fn stamp_preview_pairs_current_cells_with_the_upcoming_stamp() {
        let tool_state = ToolState::default();
        let mut canvas = Canvas::new();
        brush::apply_brush(
            &mut canvas,
            point(5, 5),
            PaintedCell {
                graphic: CellGraphic::Glyph('a'),
                color: color(10, 10, 10),
                weight_index: 2,
            },
        );

        let previews = tool_state.stamp_preview_cells(
            &canvas,
            &selection(),
            &copy_data(&[(0, 0, 'a')]),
            point(5, 5),
            PaintHand::Left,
        );

        assert_eq!(previews.len(), 1);
        assert_eq!(previews[0].point, point(5, 5));
        assert_eq!(
            previews[0].current.as_ref().unwrap().graphic,
            CellGraphic::Glyph('a')
        );
        assert_eq!(previews[0].upcoming.graphic, CellGraphic::Glyph('a'));
    }
}
