//! The tool-presets module: a toolbox sibling that lists the user's saved
//! two-hand tool presets and applies them per hand.
//!
//! Rows, top-down: presets on top (above SAVE), SAVE next, then a
//! collapsible `save options` line (J 2026-09-24):
//! - preset rows: one per saved preset, preset 0 highest so the 1-9 slot
//!   digits read top-down. Left click applies the left half to the left
//!   hand, right click the right half, middle click both. A
//!   double-left-click starts an inline rename; a double-right-click
//!   deletes the preset. Number keys apply both halves and are
//!   orchestration-owned, not module clicks.
//! - `SAVE`: captures both live hands as a new preset, filtered by the
//!   capture toggles.
//! - `save options`: one collapsed line by default; click to expand it
//!   into one `[x] label` toggle row per `HAND_FIELDS` registry entry,
//!   generated from the registry so new hand properties appear for free.
//!
//! Preset lifetime is user-session state (the shared `Vec<ToolPreset>`
//! handle), never this module's own storage — the module only edits it.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;
use std::time::Instant;

use thaum_renderer_domain::{
    push_text_cells, text_entry::TextEntryField, title_hotspot, Cell, CellGraphic, CellGroup,
    CellGroupIntakeBehavior, CellPoint, CellWeight, DoubleClick, GizmoBar, GizmoClickOutcome,
    GizmoKind, GizmoState, Hotspot, Module, ModulePointerButton, ModulePointerEvent, ModuleRect,
    PanelChrome, PersistedModuleUiState, UiColorRole, UiPalette, WorldPoint,
};

use crate::tool_presets::{ToolPreset, HAND_FIELDS};
use crate::tool_state::{PaintHand, ToolState};

const RENAME_MAX_CHARS: usize = 24;

/// Which module row sits under a pointer position.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Row {
    Save,
    SaveOptions,
    FieldToggle(usize),
    Preset(usize),
}

struct RenameSession {
    index: usize,
    field: TextEntryField,
}

pub struct ToolPresetsModule {
    id: String,
    rect: ModuleRect,
    tool_state: Rc<RefCell<ToolState>>,
    presets: Rc<RefCell<Vec<ToolPreset>>>,
    palette: UiPalette,
    gizmos: GizmoBar,
    gizmo_state: GizmoState,
    hidden: bool,
    hovered: Option<Row>,
    /// Which registry field keys the SAVE row captures. Starts at every
    /// registered field; the user toggles rows down.
    capture_keys: BTreeSet<String>,
    /// Whether the save-options capture-toggle list is expanded. Collapsed
    /// by default: the list lives behind one `save options` line.
    save_options_open: bool,
    rename_double_click: DoubleClick<usize>,
    delete_double_click: DoubleClick<usize>,
    rename: Option<RenameSession>,
}

impl ToolPresetsModule {
    pub fn new(
        id: impl Into<String>,
        rect: ModuleRect,
        tool_state: Rc<RefCell<ToolState>>,
        presets: Rc<RefCell<Vec<ToolPreset>>>,
    ) -> Self {
        Self {
            id: id.into(),
            rect,
            tool_state,
            presets,
            palette: UiPalette::default(),
            gizmos: GizmoBar::standard(),
            gizmo_state: GizmoState::new(),
            hidden: false,
            hovered: None,
            capture_keys: HAND_FIELDS.iter().map(|field| field.key.to_string()).collect(),
            save_options_open: false,
            rename_double_click: DoubleClick::new(),
            delete_double_click: DoubleClick::new(),
            rename: None,
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

    fn row_rect(&self, index: usize) -> ModuleRect {
        let (content_x, _) = PanelChrome::content_origin();
        let (content_width, _) = PanelChrome::content_size(self.rect);
        ModuleRect {
            x0: self.rect.x0 + content_x,
            y0: self.rect.y0 + self.row_y(index),
            x1: self.rect.x0 + content_x + content_width - 1,
            y1: self.rect.y0 + self.row_y(index),
        }
    }

    fn row_at(&self, x: i32, y: i32) -> Option<Row> {
        let local_x = x - self.rect.x0;
        let local_y = y - self.rect.y0;
        let (content_x, _) = PanelChrome::content_origin();
        let (content_width, _) = PanelChrome::content_size(self.rect);
        if local_x < content_x || local_x >= content_x + content_width {
            return None;
        }
        let row = self
            .row_heights()
            .find_map(|(index, row)| (local_y == self.row_y(index)).then_some((index, row)))
            .map(|(_, row)| row);
        row
    }

    /// Every drawable row as (stack index, row), topmost first. Preset
    /// rows fill the top (preset 0 highest so the digit slots read
    /// top-down), SAVE sits under them, and the collapsible save-options
    /// line comes last — its registry toggle rows only exist while
    /// expanded.
    fn row_heights(&self) -> impl Iterator<Item = (usize, Row)> {
        let preset_count = self.presets.borrow().len();
        let toggle_count = if self.save_options_open {
            HAND_FIELDS.len()
        } else {
            0
        };
        (0..preset_count + 2 + toggle_count).map(move |index| {
            let row = if index < preset_count {
                Row::Preset(index)
            } else if index == preset_count {
                Row::Save
            } else if index == preset_count + 1 {
                Row::SaveOptions
            } else {
                Row::FieldToggle(index - preset_count - 2)
            };
            (index, row)
        })
    }
}

impl Module for ToolPresetsModule {
    fn id(&self) -> &str {
        &self.id
    }

    fn rect(&self) -> ModuleRect {
        self.rect
    }

    fn hotspots(&self) -> Vec<Hotspot> {
        let mut hotspots = self.gizmos.hotspots_with(
            self.rect,
            vec![title_hotspot(
                self.rect,
                self.gizmos.title_start_x(),
                "tool presets module",
                "saved tool + property setups. left click sets the left hand, right click the right hand, middle click both.",
            )],
        );
        hotspots.extend(self.row_heights().map(|(index, row)| {
            let (title, description) = match row {
                Row::Save => (
                    "SAVE".to_string(),
                    "save both hands' tools and the toggled properties as a new preset"
                        .to_string(),
                ),
                Row::SaveOptions => (
                    "save options".to_string(),
                    "expand or contract the property capture toggles".to_string(),
                ),
                Row::FieldToggle(field_index) => {
                    let field = &HAND_FIELDS[field_index];
                    (
                        format!("[ ] {}", field.label),
                        "toggle whether SAVE captures this property".to_string(),
                    )
                }
                Row::Preset(preset_index) => (
                    self.presets
                        .borrow()
                        .get(preset_index)
                        .map(|preset| preset.name.clone())
                        .unwrap_or_default(),
                    "left click left hand, right click right hand, middle click both; double left click to rename, double right click to delete".to_string(),
                ),
            };
            Hotspot::new(self.row_rect(index), title, description)
        }));
        hotspots
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
                        .with_title("PRESETS")
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
        let presets = self.presets.borrow();
        let (content_x, _) = PanelChrome::content_origin();
        let (content_width, _) = PanelChrome::content_size(self.rect);
        // Draw cells are module-local (the group origin is rect.x0/y0), so
        // the clamp is local too. Mixing in rect.x0 here (absolute) made
        // every clamped text run empty on a negative-x panel — the rects
        // live left of origin, so max_x went negative (J 2026-09-24).
        let max_x = content_x + content_width - 1;

        for (index, row) in self.row_heights().collect::<Vec<_>>() {
            let y = self.row_y(index);
            let is_hovered = self.hovered.as_ref() == Some(&row);
            let base_color = match &row {
                Row::Save => self.palette.get(UiColorRole::Bright),
                Row::FieldToggle(_) => self.palette.get(UiColorRole::Medium),
                Row::Preset(_) => self.palette.get(UiColorRole::Medium),
                Row::SaveOptions => self.palette.get(UiColorRole::Medium),
            };
            let color = if is_hovered {
                self.palette.get(UiColorRole::Bright)
            } else {
                base_color
            };
            match row {
                Row::Save => {
                    push_text_cells(&mut cells, content_x, y, "SAVE", color, CellWeight::Three, max_x);
                }
                Row::SaveOptions => {
                    let mark = if self.save_options_open { '-' } else { '+' };
                    push_text_cells(
                        &mut cells,
                        content_x,
                        y,
                        &format!("[{mark}] save options"),
                        color,
                        CellWeight::Two,
                        max_x,
                    );
                }
                Row::FieldToggle(field_index) => {
                    let field = &HAND_FIELDS[field_index];
                    let on = self.capture_keys.contains(field.key);
                    let mark = if on { 'x' } else { ' ' };
                    let color = if on { color } else { self.palette.get(UiColorRole::Medium) };
                    push_text_cells(
                        &mut cells,
                        content_x,
                        y,
                        &format!("[{mark}] {}", field.label),
                        color,
                        CellWeight::Two,
                        max_x,
                    );
                }
                Row::Preset(preset_index) => {
                    let Some(preset) = presets.get(preset_index) else {
                        continue;
                    };
                    // Slot digit for the number keys (1-9), dot beyond.
                    let slot = if preset_index < 9 {
                        format!("{}", preset_index + 1)
                    } else {
                        "·".to_string()
                    };
                    // Which live hands carry this preset's tools right now.
                    let tool_state = self.tool_state.borrow();
                    let left_match = tool_state.left_tool.id() == preset.left.tool;
                    let right_match = tool_state.right_tool.id() == preset.right.tool;
                    drop(tool_state);
                    let (indicator, row_color) = if left_match && right_match {
                        ('◆', self.palette.get(UiColorRole::Vivid))
                    } else if left_match {
                        ('L', self.palette.get(UiColorRole::LeftHand))
                    } else if right_match {
                        ('R', self.palette.get(UiColorRole::RightHand))
                    } else {
                        (' ', color)
                    };
                    let weight = if left_match || right_match {
                        CellWeight::Three
                    } else {
                        CellWeight::Two
                    };
                    cells.push(Cell {
                        position: CellPoint { x: content_x, y, z: 0 },
                        graphic: CellGraphic::Glyph(indicator),
                        color: row_color,
                        weight,
                        ..Cell::default()
                    });
                    cells.push(Cell {
                        position: CellPoint { x: content_x + 1, y, z: 0 },
                        graphic: CellGraphic::Glyph(slot.chars().next().unwrap_or(' ')),
                        color,
                        weight,
                        ..Cell::default()
                    });
                    let display = match &self.rename {
                        Some(session) if session.index == preset_index => {
                            format!("{}: {}", preset.name, session.field.display())
                        }
                        _ => preset.name.clone(),
                    };
                    push_text_cells(
                        &mut cells,
                        content_x + 3,
                        y,
                        &display,
                        color,
                        weight,
                        max_x,
                    );
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
                let Some(row) = self.row_at(x, y) else {
                    return;
                };
                match row {
                    Row::Save => {
                        if button == ModulePointerButton::Left {
                            let count = self.presets.borrow().len();
                            let captured = ToolPreset::capture(
                                format!("preset {}", count + 1),
                                &self.tool_state.borrow(),
                                &self.capture_keys,
                            );
                            self.presets.borrow_mut().push(captured);
                        }
                    }
                    Row::SaveOptions => {
                        if button == ModulePointerButton::Left {
                            self.save_options_open = !self.save_options_open;
                        }
                    }
                    Row::FieldToggle(field_index) => {
                        if button == ModulePointerButton::Left {
                            let key = HAND_FIELDS[field_index].key.to_string();
                            if !self.capture_keys.remove(&key) {
                                self.capture_keys.insert(key);
                            }
                        }
                    }
                    Row::Preset(preset_index) => {
                        let now = Instant::now();
                        if button == ModulePointerButton::Left
                            && self
                                .rename_double_click
                                .note_click(preset_index, now)
                        {
                            // Double left click: inline rename (J
                            // 2026-09-24). The first click already applied
                            // the preset, the second starts the rename.
                            let mut field =
                                TextEntryField::new(RENAME_MAX_CHARS, "name + enter");
                            field.focus();
                            self.rename = Some(RenameSession {
                                index: preset_index,
                                field,
                            });
                            return;
                        }
                        if button == ModulePointerButton::Right
                            && self
                                .delete_double_click
                                .note_click(preset_index, now)
                        {
                            // Double right click: delete (J 2026-09-24).
                            if self.presets.borrow().len() > preset_index {
                                self.presets.borrow_mut().remove(preset_index);
                            }
                            return;
                        }
                        let preset = match self.presets.borrow().get(preset_index) {
                            Some(preset) => preset.clone(),
                            None => return,
                        };
                        let mut tool_state = self.tool_state.borrow_mut();
                        match button {
                            ModulePointerButton::Left => {
                                preset.left.apply_to_runtime(&mut tool_state, PaintHand::Left)
                            }
                            ModulePointerButton::Right => {
                                preset.right.apply_to_runtime(&mut tool_state, PaintHand::Right)
                            }
                            ModulePointerButton::Middle => preset.apply_to_runtime(&mut tool_state),
                        }
                    }
                }
            }
            ModulePointerEvent::Move { x, y } => {
                self.gizmo_state.note_pointer(&self.gizmos, self.rect, x, y);
                if self.gizmo_state.drag_rect(x, y).is_some() {
                    self.hovered = None;
                } else {
                    self.hovered = self.row_at(x, y);
                }
                if let Some(next_rect) = self.gizmo_state.drag_rect(x, y) {
                    self.rect = next_rect;
                }
            }
            ModulePointerEvent::Up { .. } => self.gizmo_state.end_drag(),
            ModulePointerEvent::Enter => self.gizmo_state.set_hovered(true),
            ModulePointerEvent::Leave => {
                self.gizmo_state.set_hovered(false);
                self.hovered = None;
            }
            ModulePointerEvent::Down { .. } => {}
        }
    }

    /// Inline-rename key entry: keys are consumed only while a rename is
    /// active; otherwise every key falls through to normal dispatch.
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
                let mut presets = self.presets.borrow_mut();
                if let Some(preset) = presets.get_mut(session.index) {
                    preset.name = name;
                }
            }
        } else if label == "ESCAPE" {
            self.rename = None;
        }
        consumed
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
    use crate::tool_state::PaintTool;
    use thaum_renderer_domain::CellGraphic;

    fn rect() -> ModuleRect {
        ModuleRect {
            x0: 0,
            y0: 0,
            x1: 18,
            y1: 20,
        }
    }

    fn tool_state() -> Rc<RefCell<ToolState>> {
        Rc::new(RefCell::new(ToolState::default()))
    }

    fn presets() -> Rc<RefCell<Vec<ToolPreset>>> {
        Rc::new(RefCell::new(Vec::new()))
    }

    fn module() -> ToolPresetsModule {
        ToolPresetsModule::new("tool_presets", rect(), tool_state(), presets())
    }

    /// Toggle the save-options line open (its row sits directly below SAVE).
    fn open_save_options(module: &mut ToolPresetsModule) {
        let y = row_y(module, module.presets.borrow().len() + 1);
        click(module, 3, y, ModulePointerButton::Left);
    }

    fn module_with(presets: Vec<ToolPreset>) -> (Rc<RefCell<ToolState>>, Rc<RefCell<Vec<ToolPreset>>>, ToolPresetsModule) {
        let tool_state = tool_state();
        let store = Rc::new(RefCell::new(presets));
        let module =
            ToolPresetsModule::new("tool_presets", rect(), tool_state.clone(), store.clone());
        (tool_state, store, module)
    }

    fn click(module: &mut ToolPresetsModule, x: i32, y: i32, button: ModulePointerButton) {
        module.on_pointer_event(ModulePointerEvent::Click { x, y, button });
    }

    fn row_y(module: &ToolPresetsModule, index: usize) -> i32 {
        module.rect.y0 + module.row_y(index)
    }

    /// The absolute y of one preset's row (preset 0 is the topmost row of
    /// the module now, matching the digit-slot read order).
    fn preset_row_y(module: &ToolPresetsModule, preset_index: usize) -> i32 {
        row_y(module, preset_index)
    }

    /// The absolute y of the SAVE row (directly below the preset rows).
    fn save_row_y(module: &ToolPresetsModule) -> i32 {
        row_y(module, module.presets.borrow().len())
    }

    /// The absolute y of one save-options toggle row (directly below the
    /// `save options` line; only present while expanded).
    fn toggle_row_y(module: &ToolPresetsModule, field_index: usize) -> i32 {
        row_y(module, module.presets.borrow().len() + 2 + field_index)
    }

    #[test]
    fn save_row_click_captures_both_hands_with_the_toggled_fields() {
        let (tool_state, store, mut presets) = module_with(Vec::new());
        // Two capture toggles off: only the rest ride along.
        {
            let mut tool_state = tool_state.borrow_mut();
            tool_state.set_tool_for_hand(PaintHand::Left, PaintTool::Brush);
            tool_state.left_hand.brush_size = 4;
        }
        let save_y = save_row_y(&presets);
        click(&mut presets, 3, save_y, ModulePointerButton::Left);

        let stored = store.borrow();
        assert_eq!(stored.len(), 1);
        let preset = &stored[0];
        assert_eq!(preset.name, "preset 1");
        assert_eq!(preset.left.tool, "brush");
        assert_eq!(preset.left.fields["brush_size"], serde_json::json!(4));
        assert!(preset.left.fields.contains_key("weight"));
    }

    #[test]
    fn toggle_row_clicks_flip_one_registry_field() {
        let mut presets = module();
        // Save options starts collapsed; opening it puts the first registry
        // field (graphic) directly below the `save options` row.
        assert_eq!(presets.capture_keys.len(), HAND_FIELDS.len());
        open_save_options(&mut presets);
        let toggle_y = toggle_row_y(&presets, 0);
        click(&mut presets, 3, toggle_y, ModulePointerButton::Left);
        assert_eq!(presets.capture_keys.len(), HAND_FIELDS.len() - 1);
        assert!(!presets.capture_keys.contains("graphic"));
        click(&mut presets, 3, toggle_y, ModulePointerButton::Left);
        assert!(presets.capture_keys.contains("graphic"));
    }

    #[test]
    fn preset_rows_apply_per_hand() {
        let (tool_state, _, mut presets) = module_with(crate::tool_presets::starter_presets());
        let preset_y = preset_row_y(&presets, 0);
        click(&mut presets, 3, preset_y, ModulePointerButton::Left);
        assert_eq!(tool_state.borrow().left_tool, PaintTool::Lasso);
        assert_eq!(tool_state.borrow().right_tool, PaintTool::Brush);
        click(&mut presets, 3, preset_y, ModulePointerButton::Right);
        assert_eq!(tool_state.borrow().right_tool, PaintTool::Move);
        click(&mut presets, 3, preset_y, ModulePointerButton::Middle);
        let state = tool_state.borrow();
        assert_eq!(state.left_tool, PaintTool::Lasso);
        assert_eq!(state.right_tool, PaintTool::Move);
    }

    #[test]
    fn double_left_click_starts_an_inline_rename_and_commit_renames() {
        let (_, _, mut presets) = module_with(crate::tool_presets::starter_presets());
        let preset_y = preset_row_y(&presets, 0);
        click(&mut presets, 3, preset_y, ModulePointerButton::Left);
        click(&mut presets, 3, preset_y, ModulePointerButton::Left);
        assert!(presets.rename.is_some());
        // Typed keys route through key capture while the rename is active.
        assert!(presets.on_key_capture("J"));
        assert!(presets.on_key_capture("ENTER"));
        assert!(presets.rename.is_none());
        assert_eq!(presets.presets.borrow()[0].name, "J");
    }

    #[test]
    fn escape_cancels_a_rename_without_changing_the_name() {
        let (_, _, mut presets) = module_with(crate::tool_presets::starter_presets());
        let preset_y = preset_row_y(&presets, 0);
        click(&mut presets, 3, preset_y, ModulePointerButton::Left);
        click(&mut presets, 3, preset_y, ModulePointerButton::Left);
        assert!(presets.on_key_capture("Q"));
        assert!(presets.on_key_capture("ESCAPE"));
        assert!(presets.rename.is_none());
        assert_eq!(
            presets.presets.borrow()[0].name,
            crate::tool_presets::starter_presets()[0].name
        );
    }

    #[test]
    fn double_right_click_deletes_the_preset() {
        let (_, store, mut presets) = module_with(crate::tool_presets::starter_presets());
        let preset_y = preset_row_y(&presets, 0);
        click(&mut presets, 3, preset_y, ModulePointerButton::Right);
        click(&mut presets, 3, preset_y, ModulePointerButton::Right);
        assert_eq!(presets.presets.borrow().len(), 1);
        assert_eq!(store.borrow().len(), 1);
        assert_eq!(store.borrow()[0].left.tool, "brush");
    }

    #[test]
    fn single_clicks_do_not_trigger_rename_or_delete() {
        let (_, _, mut presets) = module_with(crate::tool_presets::starter_presets());
        let preset_y = preset_row_y(&presets, 0);
        click(&mut presets, 3, preset_y, ModulePointerButton::Right);
        assert_eq!(presets.presets.borrow().len(), 2);
        assert!(presets.rename.is_none());
    }

    #[test]
    fn clicking_outside_rows_is_ignored() {
        let (_, _, mut presets) = module_with(crate::tool_presets::starter_presets());
        let outside_y = presets.rect.y1 + 5;
        click(&mut presets, 3, outside_y, ModulePointerButton::Left);
        assert_eq!(presets.presets.borrow().len(), 2);
    }

    #[test]
    fn draw_shows_save_toggles_and_preset_slot_digits() {
        let presets = module_with(crate::tool_presets::starter_presets()).2;
        let group = presets.draw();
        let glyphs: String = group
            .cells
            .values()
            .filter_map(|cell| match &cell.graphic {
                CellGraphic::Glyph(glyph) => Some(*glyph),
                _ => None,
            })
            .collect();
        assert!(glyphs.contains('S'));
        assert!(glyphs.contains('['));
        assert!(glyphs.contains('1'));
        assert!(glyphs.contains('2'));
    }

    /// The panel's real placement is left of the origin (x0 = -41); a
    /// draw there must still lay out full text runs, not just unclamped
    /// cells (regression: the max_x clamp mixed absolute rect.x0 into the
    /// local draw space and clipped every clamped run to empty).
    #[test]
    fn save_options_line_expands_and_contracts_the_toggle_list() {
        let (_, _, mut presets) = module_with(crate::tool_presets::starter_presets());
        assert!(!presets.save_options_open);
        // Collapsed: preset rows, SAVE, and the save-options line.
        assert_eq!(presets.row_heights().count(), 2 + 2);
        open_save_options(&mut presets);
        assert!(presets.save_options_open);
        // Expanded: every registry toggle row joins the stack.
        assert_eq!(
            presets.row_heights().count(),
            2 + 2 + HAND_FIELDS.len()
        );
        open_save_options(&mut presets);
        assert!(!presets.save_options_open);
        assert_eq!(presets.row_heights().count(), 2 + 2);
    }

    #[test]
    fn save_options_collapsed_by_default_and_clicks_land_on_the_right_rows() {
        let (_, _, mut presets) = module_with(crate::tool_presets::starter_presets());
        // Top of the panel is preset 0; below the two presets sit SAVE then
        // the save-options line. A click on SAVE must not flip the toggle.
        let save_y = save_row_y(&presets);
        click(&mut presets, 3, save_y, ModulePointerButton::Left);
        assert_eq!(presets.presets.borrow().len(), 3);
        // And the line below SAVE is the save-options line, not a toggle
        // (the save click grew the list, so the line moved down one).
        let options_y = row_y(&presets, presets.presets.borrow().len() + 1);
        click(&mut presets, 3, options_y, ModulePointerButton::Left);
        assert!(presets.save_options_open);
    }

    #[test]
    fn draw_at_a_negative_x_panel_still_lays_out_full_text_runs() {
        let (tool_state, store, mut presets) = module_with(crate::tool_presets::starter_presets());
        presets.rect = ModuleRect { x0: -41, y0: 36, x1: -26, y1: 56 };
        // Make the live tools match starter preset 0 so the row paints its
        // indicator, digit, and name like the real first-boot view.
        tool_state.borrow_mut().set_tool_for_hand(PaintHand::Left, PaintTool::Lasso);
        tool_state.borrow_mut().set_tool_for_hand(PaintHand::Right, PaintTool::Move);
        let group = presets.draw();
        let glyphs: String = group
            .cells
            .values()
            .filter_map(|cell| match &cell.graphic {
                CellGraphic::Glyph(glyph) => Some(*glyph),
                _ => None,
            })
            .collect();
        assert!(glyphs.contains('S'));
        assert!(glyphs.contains('A'));
        assert!(glyphs.contains('V'));
        assert!(glyphs.contains('E'));
        assert!(glyphs.contains('o')); // "save options" collapsed line
        assert!(glyphs.contains('◆'));
        assert_eq!(store.borrow().len(), 2);
    }

    #[test]
    fn hotspots_cover_the_save_row_and_each_preset_row() {
        let presets = module_with(crate::tool_presets::starter_presets()).2;
        let hotspots = presets.hotspots();
        assert!(hotspots
            .iter()
            .any(|hotspot| hotspot.title == "SAVE"));
        assert!(hotspots
            .iter()
            .any(|hotspot| hotspot.title == "select + move"));
    }
}
