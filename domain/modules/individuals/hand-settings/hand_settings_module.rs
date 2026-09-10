use std::{cell::RefCell, rc::Rc};

use thaum_renderer_domain::{
    Cell, CellGraphic, CellGroup, CellGroupIntakeBehavior, CellPoint, CellWeight, GizmoBar,
    GizmoClickOutcome, GizmoKind, GizmoState, Hotspot, Module, ModulePointerEvent, ModuleRect,
    NumberFieldEdit, PanelChrome, PersistedModuleUiState, PropertyHit, PropertyMatrixColumn,
    PropertyMatrixSide, PropertyRow, PropertyRows, ScrollState, UiPalette, WorldPoint,
    title_hotspot,
};

use crate::{
    selection_state::{PainterSelection, SelectionMode},
    tool_state::{PaintChannel, PaintHand, PaintTarget, PaintTool, ToolState},
};

/// Rows reserved at the top of the content area for the two 3x3 hand
/// preview blocks (one per hand).
const HAND_BLOCK_ROWS: i32 = 3;

fn format_graphic(graphic: &CellGraphic) -> String {
    match graphic {
        CellGraphic::None => "none".into(),
        CellGraphic::Glyph(glyph) => glyph.to_string(),
        CellGraphic::Sprite(sprite) => format!("spr:{}", sprite.atlas_relative_path().display()),
    }
}
pub struct HandSettingsModule {
    id: String,
    rect: ModuleRect,
    tool_state: Rc<RefCell<ToolState>>,
    selection: Rc<RefCell<PainterSelection>>,
    /// In-place number-field edit driven through the host's typing seam
    /// (shared with the entrypoint, which routes the keys and applies the
    /// committed value).
    number_edit: Rc<RefCell<Option<NumberFieldEdit>>>,
    palette: UiPalette,
    gizmos: GizmoBar,
    gizmo_state: GizmoState,
    /// Row scroll over the property rows; the panel crops rows that do not
    /// fit, so scrolling is how tucked-away tool rows become reachable.
    scroll: ScrollState,
    hidden: bool,
}

impl HandSettingsModule {
    pub fn new(
        id: impl Into<String>,
        rect: ModuleRect,
        tool_state: Rc<RefCell<ToolState>>,
        selection: Rc<RefCell<PainterSelection>>,
        number_edit: Rc<RefCell<Option<NumberFieldEdit>>>,
    ) -> Self {
        Self {
            id: id.into(),
            rect,
            tool_state,
            selection,
            number_edit,
            palette: UiPalette::default(),
            gizmos: GizmoBar::standard(),
            gizmo_state: GizmoState::new(),
            scroll: ScrollState::new(),
            hidden: false,
        }
    }

    pub fn with_palette(mut self, palette: UiPalette) -> Self {
        self.palette = palette;
        self
    }

    fn tool_label(tool: PaintTool) -> &'static str {
        crate::painter_tools::require_by_id(tool.id()).label
    }

    fn build_rows(&self) -> Vec<PropertyRow> {
        let state = self.tool_state.borrow();
        let left = state.left_hand.clone();
        let right = state.right_hand.clone();

        let mut weight_columns = Vec::new();
        for weight in 0..=3 {
            let mut column = PropertyMatrixColumn::new(weight.to_string(), weight.to_string());
            column.left_value = left.weight_index == weight;
            column.right_value = right.weight_index == weight;
            weight_columns.push(column);
        }

        let mut select_columns = Vec::new();
        for (id, label, channel) in [
            ("graphic", "GFX", PaintChannel::Graphic),
            ("color", "COL", PaintChannel::Color),
            ("weight", "WGT", PaintChannel::Weight),
        ] {
            let mut column = PropertyMatrixColumn::new(id, label);
            column.left_value = left.select_channels.is_enabled(channel);
            column.right_value = right.select_channels.is_enabled(channel);
            select_columns.push(column);
        }

        let mut edit_columns = Vec::new();
        for (id, label, channel) in [
            ("graphic", "GFX", PaintChannel::Graphic),
            ("color", "COL", PaintChannel::Color),
            ("weight", "WGT", PaintChannel::Weight),
        ] {
            let mut column = PropertyMatrixColumn::new(id, label);
            column.left_value = left.edit_channels.is_enabled(channel);
            column.right_value = right.edit_channels.is_enabled(channel);
            edit_columns.push(column);
        }

        let mut target_columns = Vec::new();
        let mut selection = PropertyMatrixColumn::new("selection", "select");
        selection.left_value = left.target == PaintTarget::Selection;
        selection.right_value = right.target == PaintTarget::Selection;
        target_columns.push(selection);
        let mut image = PropertyMatrixColumn::new("image", "image");
        image.left_value = left.target == PaintTarget::Image;
        image.right_value = right.target == PaintTarget::Image;
        target_columns.push(image);

        let selection_mode = self.selection.borrow().mode();

        let mut mode_columns = Vec::new();
        for (id, label, mode) in [
            ("replace", "rep", SelectionMode::Replace),
            ("add", "add", SelectionMode::Additive),
            ("sub", "sub", SelectionMode::Subtract),
            ("mul", "mul", SelectionMode::Intersect),
        ] {
            let mut column = PropertyMatrixColumn::new(id, label);
            column.left_value = selection_mode == mode;
            column.right_value = selection_mode == mode;
            mode_columns.push(column);
        }

        let mut brush_size_columns = Vec::new();
        for size in 1..=5 {
            let mut column = PropertyMatrixColumn::new(size.to_string(), size.to_string());
            column.left_value = left.brush_size == size;
            column.right_value = right.brush_size == size;
            column.left_enabled = matches!(state.left_tool, PaintTool::Brush | PaintTool::Erase);
            column.right_enabled = matches!(state.right_tool, PaintTool::Brush | PaintTool::Erase);
            brush_size_columns.push(column);
        }

        let mut fill_diag_columns = Vec::new();
        for (id, label, value) in [("off", "orth", false), ("on", "diag", true)] {
            let mut column = PropertyMatrixColumn::new(id, label);
            column.left_value = left.fill_diagonal == value;
            column.right_value = right.fill_diagonal == value;
            column.left_enabled = state.left_tool == PaintTool::Fill;
            column.right_enabled = state.right_tool == PaintTool::Fill;
            fill_diag_columns.push(column);
        }

        let mut picker_opp_columns = Vec::new();
        for (id, label, value) in [("self", "self", false), ("opp", "opp", true)] {
            let mut column = PropertyMatrixColumn::new(id, label);
            column.left_value = left.pick_opposite_hand == value;
            column.right_value = right.pick_opposite_hand == value;
            column.left_enabled = state.left_tool == PaintTool::Picker;
            column.right_enabled = state.right_tool == PaintTool::Picker;
            picker_opp_columns.push(column);
        }

        let char_step = state.text_options.char_step;
        let enter_step = state.text_options.enter_step;

        // Tools, graphics and colors are not rows: the hand preview blocks
        // drawn at the top of the panel carry that state visually.
        let mut rows = vec![
            PropertyRow::Matrix {
                id: "weight".into(),
                label: "weight".into(),
                columns: weight_columns,
                token_width: 2,
            },
            PropertyRow::Matrix {
                id: "select".into(),
                label: "Select".into(),
                columns: select_columns,
                token_width: 4,
            },
            PropertyRow::Matrix {
                id: "edit".into(),
                label: "Edit".into(),
                columns: edit_columns,
                token_width: 4,
            },
            PropertyRow::Matrix {
                id: "target".into(),
                label: "target".into(),
                columns: target_columns,
                token_width: 7,
            },
            PropertyRow::Matrix {
                id: "selection_mode".into(),
                label: "mode".into(),
                columns: mode_columns,
                token_width: 4,
            },
        ];

        // Tool rows only appear when either equipped hand's tool declares
        // them, so the panel stays thin as tools gain properties.
        if Self::tool_row_used(&state, "brush_size") {
            rows.push(PropertyRow::Matrix {
                id: "brush_size".into(),
                label: "size".into(),
                columns: brush_size_columns,
                token_width: 2,
            });
        }
        if Self::tool_row_used(&state, "fill_diagonal") {
            rows.push(PropertyRow::Matrix {
                id: "fill_diagonal".into(),
                label: "fill".into(),
                columns: fill_diag_columns,
                token_width: 4,
            });
        }
        // Picker target hand: each hand's picker toggles independently
        // whether its samples land on itself or the opposite hand.
        if Self::tool_row_used(&state, "picker_opposite_hand") {
            rows.push(PropertyRow::Matrix {
                id: "picker_opposite_hand".into(),
                label: "pick".into(),
                columns: picker_opp_columns,
                token_width: 4,
            });
        }
        // Text typing geometry: per-character and per-Enter 3D cursor steps,
        // shared across hands. Left-click a number to type a value in; scroll
        // a number to nudge it by one.
        if Self::tool_row_used(&state, "text_char_step") {
            rows.push(PropertyRow::NumberRow {
                id: "text_char_step".into(),
                label: "char".into(),
                values: vec![char_step.0, char_step.1, char_step.2],
                min: -9,
                max: 9,
                editing: self.editing_for("text_char_step"),
            });
        }
        if Self::tool_row_used(&state, "text_enter_step") {
            rows.push(PropertyRow::NumberRow {
                id: "text_enter_step".into(),
                label: "enter".into(),
                values: vec![enter_step.0, enter_step.1, enter_step.2],
                min: -9,
                max: 9,
                editing: self.editing_for("text_enter_step"),
            });
        }

        rows
    }

    /// Rows that fit below the reserved hand-preview block rows; every
    /// property row is one line tall, so that is the visible row count.
    fn visible_row_count(&self) -> usize {
        let (_, content_height) = PanelChrome::content_size(self.rect);
        (content_height - HAND_BLOCK_ROWS).max(0) as usize
    }

    /// The scrolled window of rows actually drawn and hit-tested: the full
    /// row list offset by the scroll state, cropped to what fits.
    fn visible_rows(&self) -> Vec<PropertyRow> {
        let rows = self.build_rows();
        let scroll = self.scroll.offset().min(ScrollState::max_offset(
            rows.len(),
            self.visible_row_count(),
        ));
        rows.into_iter()
            .skip(scroll)
            .take(self.visible_row_count())
            .collect()
    }

    fn tool_row_used(state: &ToolState, row_id: &str) -> bool {
        state.left_tool.property_row_ids().contains(&row_id)
            || state.right_tool.property_row_ids().contains(&row_id)
    }

    fn editing_for(&self, row_id: &str) -> Option<(usize, String)> {
        self.number_edit
            .borrow()
            .as_ref()
            .filter(|edit| edit.row_id == row_id)
            .map(|edit| (edit.field, edit.buffer.clone()))
    }

    /// Whether a number-field typing session is currently open. The
    /// entrypoint reads this to route keys through the typing seam.
    pub fn number_edit_active(&self) -> bool {
        self.number_edit.borrow().is_some()
    }

    /// The active edit, for the entrypoint's key routing.
    pub fn number_edit(&self) -> Rc<RefCell<Option<NumberFieldEdit>>> {
        self.number_edit.clone()
    }

    fn apply_property_hit(&mut self, hit: PropertyHit) {
        // A number-field click begins an in-place edit; the entrypoint routes
        // the typing and applies the committed value through the tool state.
        if let PropertyHit::Number { row_id, field } = hit {
            let current = match row_id.as_str() {
                "text_char_step" => {
                    let step = self.tool_state.borrow().text_options.char_step;
                    [step.0, step.1, step.2][field.min(2)]
                }
                "text_enter_step" => {
                    let step = self.tool_state.borrow().text_options.enter_step;
                    [step.0, step.1, step.2][field.min(2)]
                }
                _ => return,
            };
            *self.number_edit.borrow_mut() = Some(NumberFieldEdit::begin(row_id, field, current));
            return;
        }
        let PropertyHit::Matrix {
            row_id,
            side,
            column_id,
        } = hit
        else {
            return;
        };
        let hand = match side {
            PropertyMatrixSide::Left => PaintHand::Left,
            PropertyMatrixSide::Right => PaintHand::Right,
        };
        let mut state = self.tool_state.borrow_mut();
        match row_id.as_str() {
            "weight" => {
                if let Ok(weight) = column_id.parse::<i64>() {
                    state.set_weight_for_hand(hand, weight);
                }
            }
            "select" => {
                if let Some(channel) = Self::channel_from_id(&column_id) {
                    state.toggle_select_channel_for_hand(hand, channel);
                }
            }
            "edit" => {
                if let Some(channel) = Self::channel_from_id(&column_id) {
                    state.toggle_edit_channel_for_hand(hand, channel);
                }
            }
            "target" => {
                let next = match column_id.as_str() {
                    "image" => PaintTarget::Image,
                    "selection" => PaintTarget::Selection,
                    _ => return,
                };
                state.set_target_for_hand(hand, next);
            }
            "selection_mode" => {
                let mode = match column_id.as_str() {
                    "replace" => SelectionMode::Replace,
                    "add" => SelectionMode::Additive,
                    "sub" => SelectionMode::Subtract,
                    "mul" => SelectionMode::Intersect,
                    _ => return,
                };
                self.selection.borrow_mut().set_mode(mode);
            }
            "brush_size" => {
                if let Ok(size) = column_id.parse::<i32>() {
                    state.set_brush_size_for_hand(hand, size);
                }
            }
            "fill_diagonal" => {
                let fill_diagonal = match column_id.as_str() {
                    "off" => false,
                    "on" => true,
                    _ => return,
                };
                state.set_fill_diagonal_for_hand(hand, fill_diagonal);
            }
            "picker_opposite_hand" => {
                let pick_opposite_hand = match column_id.as_str() {
                    "self" => false,
                    "opp" => true,
                    _ => return,
                };
                state.set_pick_opposite_hand_for_hand(hand, pick_opposite_hand);
            }
            _ => {}
        }
    }

    fn channel_from_id(id: &str) -> Option<PaintChannel> {
        match id {
            "graphic" => Some(PaintChannel::Graphic),
            "color" => Some(PaintChannel::Color),
            "weight" => Some(PaintChannel::Weight),
            _ => None,
        }
    }
}

impl Module for HandSettingsModule {
    fn id(&self) -> &str {
        &self.id
    }

    fn rect(&self) -> ModuleRect {
        self.rect
    }

    /// Tooltip hotspots: the module's gizmo bar, one hotspot per hand
    /// preview block, and one hotspot per visible property row.
    fn hotspots(&self) -> Vec<Hotspot> {
        let rows = self.visible_rows();
        let mut custom = vec![title_hotspot(
            self.rect,
            self.gizmos.title_start_x(),
            "properties module",
            "adjust the properties of a selected tool. properties are adjusted per hand for your left and right clicks. left click for adjusting the left tool, right click for adjusting the right tool.",
        )];
        let state = self.tool_state.borrow();
        let (content_x, _) = PanelChrome::content_origin();
        let block_top_y = PropertyRows::top_row_y(self.rect) - 2;
        for (label, tool, hand, block_x) in [
            ("left", state.left_tool, &state.left_hand, content_x),
            ("right", state.right_tool, &state.right_hand, content_x + 4),
        ] {
            custom.push(Hotspot::new(
                ModuleRect {
                    x0: self.rect.x0 + block_x,
                    y0: self.rect.y0 + block_top_y,
                    x1: self.rect.x0 + block_x + 2,
                    y1: self.rect.y0 + block_top_y + 2,
                },
                format!("{label} hand"),
                format!(
                    "tool {} · graphic {} · weight {} · color {}",
                    Self::tool_label(tool),
                    format_graphic(&hand.graphic),
                    hand.weight_index,
                    hand.color.label()
                ),
            ));
        }
        custom.extend(PropertyRows::hotspots(self.rect, &rows, HAND_BLOCK_ROWS));
        // J 2026-09-10: the Select and Edit rows speak as whole rows, not
        // per-token — their copy carries the per-hand channel truth.
        for hotspot in custom.iter_mut() {
            match hotspot.title.as_str() {
                "Select" => {
                    hotspot.description = "lock a graphic, color, or weight channel per hand for selection oriented tool use. like fill sensing adjacent tiles".into();
                }
                "Edit" => {
                    hotspot.description = "lock or unlock a graphic, color, or weight channel per hand for placement oriented tool use. like fill placing down the actual content".into();
                }
                _ => {}
            }
        }
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
                        .with_title("PROPS")
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
        cells.extend(PropertyRows::draw(
            self.rect,
            &self.visible_rows(),
            &self.palette,
            HAND_BLOCK_ROWS,
        ));

        let state = self.tool_state.borrow();
        let (content_x, _) = PanelChrome::content_origin();
        let block_top_y = PropertyRows::top_row_y(self.rect) - 2;

        // Two 3x3 hand preview blocks: the ring is the hand's graphic in the
        // hand's weight and color; the center is the hand letter in weight
        // two and the hand's color. This carries the tools/graphic/color
        // state visually instead of text rows.
        for (label, hand, block_x) in [
            ('L', &state.left_hand, content_x),
            ('R', &state.right_hand, content_x + 4),
        ] {
            let color = hand.color.preview_rgb();
            let color = thaum_renderer_domain::CellColor::Flat([
                color.0 as f32 / 255.0,
                color.1 as f32 / 255.0,
                color.2 as f32 / 255.0,
                1.0,
            ]);
            let ring_glyph = match &hand.graphic {
                CellGraphic::Glyph(glyph) => *glyph,
                // Sprites cannot render in one cell; a solid block signals
                // "a sprite is equipped".
                CellGraphic::Sprite(_) => '█',
                CellGraphic::None => '·',
            };
            let weight = CellWeight::from_index_clamped(hand.weight_index as i32);
            for dy in 0..3 {
                for dx in 0..3 {
                    let center = dx == 1 && dy == 1;
                    cells.push(Cell {
                        position: CellPoint {
                            x: block_x + dx,
                            y: block_top_y + dy,
                            z: 0,
                        },
                        graphic: CellGraphic::Glyph(if center { label } else { ring_glyph }),
                        color,
                        weight: if center {
                            CellWeight::from_index_clamped(2)
                        } else {
                            weight
                        },
                        ..Cell::default()
                    });
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
                let rows = self.visible_rows();
                if let Some(hit) =
                    PropertyRows::hit_test(self.rect, &rows, x, y, button, HAND_BLOCK_ROWS)
                {
                    self.apply_property_hit(hit);
                }
            }
            ModulePointerEvent::Move { x, y } => {
                self.gizmo_state.note_pointer(&self.gizmos, self.rect, x, y);
                if let Some(next_rect) = self.gizmo_state.drag_rect(x, y) {
                    self.rect = next_rect;
                }
            }
            ModulePointerEvent::Up { .. } => self.gizmo_state.end_drag(),
            ModulePointerEvent::Enter => self.gizmo_state.set_hovered(true),
            ModulePointerEvent::Leave => self.gizmo_state.set_hovered(false),
            ModulePointerEvent::Down { .. } => {}
        }
    }

    /// Scroll on a number field nudges that value by one (up = +1, down =
    /// −1), clamped to the row's range. Anywhere else, the wheel scrolls the
    /// panel's row list so cropped tool rows stay reachable.
    fn on_wheel(&mut self, x: i32, y: i32, _delta_x: f32, delta_y: f32) -> bool {
        if self.number_edit.borrow().is_some() {
            // A typing session owns the keyboard; ignore wheel while editing.
            return true;
        }
        let rows = self.visible_rows();
        if let Some((row_id, field)) =
            PropertyRows::number_field_at(self.rect, &rows, x, y, HAND_BLOCK_ROWS)
        {
            let delta = if delta_y > 0.0 {
                1
            } else if delta_y < 0.0 {
                -1
            } else {
                return true;
            };
            let axis = field.min(2);
            let mut state = self.tool_state.borrow_mut();
            return match row_id.as_str() {
                "text_char_step" => {
                    state.nudge_text_char_step(axis, delta);
                    true
                }
                "text_enter_step" => {
                    state.nudge_text_enter_step(axis, delta);
                    true
                }
                _ => false,
            };
        }
        // Not over a number field: the wheel scrolls the row list.
        let max = ScrollState::max_offset(self.build_rows().len(), self.visible_row_count());
        self.scroll.wheel(delta_y, max);
        true
    }

    fn wants_pointer_capture(&self) -> bool {
        self.gizmo_state.wants_pointer_capture()
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
    use thaum_renderer_domain::ModulePointerButton;

    fn rect() -> ModuleRect {
        ModuleRect {
            x0: 10,
            y0: 10,
            x1: 40,
            // 15 tall: content 12 = 9 visible property rows + the 3 reserved
            // hand-preview block rows, matching the production boot size.
            y1: 25,
        }
    }

    fn tool_state() -> Rc<RefCell<ToolState>> {
        Rc::new(RefCell::new(ToolState::default()))
    }

    fn number_edit() -> Rc<RefCell<Option<NumberFieldEdit>>> {
        Rc::new(RefCell::new(None))
    }

    fn selection() -> Rc<RefCell<PainterSelection>> {
        Rc::new(RefCell::new(PainterSelection::new(
            crate::fill::CanvasBounds {
                x0: 0,
                y0: 0,
                x1: 8,
                y1: 8,
                z: 0,
                plane_axis: crate::fill::CanvasPlaneAxis::Z,
            },
        )))
    }

    #[test]
    fn clicking_a_weight_token_assigns_that_weight_to_the_matching_hand() {
        let state = tool_state();
        let mut module =
            HandSettingsModule::new("hands", rect(), state.clone(), selection(), number_edit());

        module.apply_property_hit(PropertyHit::Matrix {
            row_id: "weight".into(),
            side: PropertyMatrixSide::Left,
            column_id: "3".into(),
        });

        assert_eq!(state.borrow().left_hand.weight_index, 3);
    }

    #[test]
    fn clicking_select_tokens_toggles_that_select_channel_for_the_clicked_side() {
        let state = tool_state();
        let mut module =
            HandSettingsModule::new("hands", rect(), state.clone(), selection(), number_edit());

        module.apply_property_hit(PropertyHit::Matrix {
            row_id: "select".into(),
            side: PropertyMatrixSide::Right,
            column_id: "color".into(),
        });

        assert!(!state.borrow().right_hand.select_channels.color);
    }

    #[test]
    fn clicking_edit_tokens_toggles_that_edit_channel_for_the_clicked_side() {
        let state = tool_state();
        let mut module =
            HandSettingsModule::new("hands", rect(), state.clone(), selection(), number_edit());

        module.apply_property_hit(PropertyHit::Matrix {
            row_id: "edit".into(),
            side: PropertyMatrixSide::Right,
            column_id: "color".into(),
        });

        assert!(!state.borrow().right_hand.edit_channels.color);
    }

    #[test]
    fn clicking_target_tokens_sets_that_exact_target_for_the_matching_hand() {
        let state = tool_state();
        let mut module =
            HandSettingsModule::new("hands", rect(), state.clone(), selection(), number_edit());

        module.apply_property_hit(PropertyHit::Matrix {
            row_id: "target".into(),
            side: PropertyMatrixSide::Left,
            column_id: "selection".into(),
        });

        assert_eq!(state.borrow().left_hand.target, PaintTarget::Selection);
    }

    #[test]
    fn clicking_selection_mode_tokens_sets_the_shared_selection_mode() {
        let state = tool_state();
        let selection = selection();
        let mut module =
            HandSettingsModule::new("hands", rect(), state, selection.clone(), number_edit());

        module.apply_property_hit(PropertyHit::Matrix {
            row_id: "selection_mode".into(),
            side: PropertyMatrixSide::Left,
            column_id: "sub".into(),
        });

        assert_eq!(selection.borrow().mode(), SelectionMode::Subtract);
    }

    #[test]
    fn clicking_brush_size_tokens_updates_only_the_matching_hand() {
        let state = tool_state();
        state
            .borrow_mut()
            .set_tool_for_hand(PaintHand::Left, PaintTool::Brush);
        let mut module =
            HandSettingsModule::new("hands", rect(), state.clone(), selection(), number_edit());

        module.apply_property_hit(PropertyHit::Matrix {
            row_id: "brush_size".into(),
            side: PropertyMatrixSide::Left,
            column_id: "4".into(),
        });

        assert_eq!(state.borrow().left_hand.brush_size, 4);
        assert_eq!(state.borrow().right_hand.brush_size, 1);
    }

    #[test]
    fn clicking_fill_diag_tokens_updates_only_the_matching_hand() {
        let state = tool_state();
        state
            .borrow_mut()
            .set_tool_for_hand(PaintHand::Right, PaintTool::Fill);
        let mut module =
            HandSettingsModule::new("hands", rect(), state.clone(), selection(), number_edit());

        module.apply_property_hit(PropertyHit::Matrix {
            row_id: "fill_diagonal".into(),
            side: PropertyMatrixSide::Right,
            column_id: "on".into(),
        });

        assert!(state.borrow().right_hand.fill_diagonal);
        assert!(!state.borrow().left_hand.fill_diagonal);
    }

    #[test]
    fn rows_only_include_tool_properties_the_equipped_tools_use() {
        let state = tool_state();
        let mut module =
            HandSettingsModule::new("hands", rect(), state.clone(), selection(), number_edit());

        let row_ids = |module: &HandSettingsModule| -> Vec<String> {
            module
                .build_rows()
                .into_iter()
                .filter_map(|row| match row {
                    PropertyRow::Matrix { id, .. } => Some(id),
                    _ => None,
                })
                .collect()
        };

        // Default hands: brush left, erase right — brush size shows, fill hides.
        let default_ids = row_ids(&module);
        assert!(default_ids.contains(&"brush_size".to_string()));
        assert!(!default_ids.contains(&"fill_diagonal".to_string()));

        state
            .borrow_mut()
            .set_tool_for_hand(PaintHand::Right, PaintTool::Fill);
        let mixed_ids = row_ids(&module);
        assert!(mixed_ids.contains(&"brush_size".to_string()));
        assert!(mixed_ids.contains(&"fill_diagonal".to_string()));
        assert!(!mixed_ids.contains(&"fill_match".to_string()));

        // Standard rows stay put regardless of the equipped tools.
        assert!(mixed_ids.contains(&"weight".to_string()));
    }

    #[test]
    fn text_step_rows_only_appear_when_text_is_equipped() {
        let state = tool_state();
        let mut module =
            HandSettingsModule::new("hands", rect(), state.clone(), selection(), number_edit());

        let row_ids = |module: &HandSettingsModule| -> Vec<String> {
            module
                .build_rows()
                .into_iter()
                .filter_map(|row| match row {
                    PropertyRow::NumberRow { id, .. } => Some(id),
                    _ => None,
                })
                .collect()
        };

        // Brush/erase hands: no text rows.
        let default_ids = row_ids(&module);
        assert!(!default_ids.contains(&"text_char_step".to_string()));
        assert!(!default_ids.contains(&"text_enter_step".to_string()));

        state
            .borrow_mut()
            .set_tool_for_hand(PaintHand::Left, PaintTool::Text);
        let text_ids = row_ids(&module);
        assert!(text_ids.contains(&"text_char_step".to_string()));
        assert!(text_ids.contains(&"text_enter_step".to_string()));
    }

    #[test]
    fn clicking_a_text_number_field_begins_an_in_place_edit_with_the_current_value() {
        let state = tool_state();
        state
            .borrow_mut()
            .set_tool_for_hand(PaintHand::Left, PaintTool::Text);
        let number_edit = number_edit();
        let mut module =
            HandSettingsModule::new("hands", rect(), state, selection(), number_edit.clone());

        module.apply_property_hit(PropertyHit::Number {
            row_id: "text_char_step".into(),
            field: 1,
        });

        let edit = number_edit.borrow().clone().expect("edit began");
        assert_eq!(edit.row_id, "text_char_step");
        assert_eq!(edit.field, 1);
        // char_step starts (1, 0, 0), so field 1 edits "0".
        assert_eq!(edit.buffer, "0");
        assert!(module.number_edit_active());
    }

    // Content 9 tall -> 6 visible property rows below the 3 hand-preview
    // block rows, so the 7 text-tool rows overflow and scrolling is
    // exercised.
    fn scroll_rect() -> ModuleRect {
        ModuleRect {
            x0: 10,
            y0: 10,
            x1: 40,
            y1: 22,
        }
    }

    #[test]
    fn scrolling_a_text_number_field_nudges_that_axis_by_one() {
        let state = tool_state();
        state
            .borrow_mut()
            .set_tool_for_hand(PaintHand::Left, PaintTool::Text);
        let mut module = HandSettingsModule::new(
            "hands",
            scroll_rect(),
            state.clone(),
            selection(),
            number_edit(),
        );

        // Locate the char row's first field through the seam, then wheel on
        // it. Rows draw one line each from the top row downward.
        let rows = module.build_rows();
        let row_index = rows
            .iter()
            .position(
                |row| matches!(row, PropertyRow::NumberRow { id, .. } if id == "text_char_step"),
            )
            .expect("text char row present");
        let (_, value_x, _) = PropertyRows::content_columns(scroll_rect());
        let field_x = scroll_rect().x0 + value_x;

        // The text rows sit below the visible window: scroll the panel down
        // until the char row is on screen (wheel off the fields scrolls).
        let visible = module.visible_row_count();
        let scroll_needed = row_index.saturating_sub(visible - 1);
        for _ in 0..scroll_needed {
            assert!(module.on_wheel(field_x, scroll_rect().y0, 0.0, -1.0));
        }
        // Rows stack downward from the first row below the hand-preview
        // blocks, one row_height each.
        let row_y = rows[scroll_needed..row_index]
            .iter()
            .map(|row| PropertyRows::row_height(row))
            .sum::<i32>();
        let field_y = scroll_rect().y0 + PropertyRows::top_row_y(scroll_rect())
            - HAND_BLOCK_ROWS
            - row_y;
        assert_eq!(
            PropertyRows::number_field_at(
                scroll_rect(),
                &module.visible_rows(),
                field_x,
                field_y,
                HAND_BLOCK_ROWS,
            ),
            Some(("text_char_step".into(), 0))
        );

        module.on_wheel(field_x, field_y, 0.0, 1.0);
        assert_eq!(state.borrow().text_options.char_step, (2, 0, 0));
        module.on_wheel(field_x, field_y, 0.0, -2.0);
        assert_eq!(state.borrow().text_options.char_step, (1, 0, 0));
    }

    #[test]
    fn wheeling_off_the_fields_scrolls_cropped_tool_rows_into_view() {
        let state = tool_state();
        state
            .borrow_mut()
            .set_tool_for_hand(PaintHand::Left, PaintTool::Text);
        let mut module = HandSettingsModule::new(
            "hands",
            scroll_rect(),
            state.clone(),
            selection(),
            number_edit(),
        );

        // Before scrolling, the cropped text rows are outside the visible
        // slice even though the full row list declares them.
        let total = module.build_rows().len();
        let visible = module.visible_row_count();
        assert!(total > visible);
        assert!(module.visible_rows().len() == visible);
        assert!(module
            .visible_rows()
            .iter()
            .all(|row| !matches!(row, PropertyRow::NumberRow { .. })));

        // Wheel down off the fields: consumed, and the char row surfaces.
        let (_, value_x, _) = PropertyRows::content_columns(scroll_rect());
        let x = scroll_rect().x0 + value_x;
        for _ in 0..(total - visible) {
            assert!(module.on_wheel(x, scroll_rect().y0, 0.0, -1.0));
        }
        assert!(module
            .visible_rows()
            .iter()
            .any(|row| matches!(row, PropertyRow::NumberRow { id, .. } if id == "text_char_step")));

        // Wheeling back up returns to the top and clamps there.
        for _ in 0..total {
            module.on_wheel(x, rect().y0, 0.0, 1.0);
        }
        assert_eq!(module.visible_rows().len(), visible);
        let first = module.build_rows().into_iter().next().expect("row");
        assert_eq!(module.visible_rows()[0], first);
    }

    #[test]
    fn clicking_the_move_gizmo_starts_requesting_pointer_capture() {
        let state = tool_state();
        let mut module =
            HandSettingsModule::new("hands", rect(), state, selection(), number_edit());

        module.on_pointer_event(ModulePointerEvent::Click {
            x: 11,
            // The move gizmo lives on the top border row.
            y: rect().y1 - 1,
            button: ModulePointerButton::Left,
        });

        assert!(module.wants_pointer_capture());
    }
}
