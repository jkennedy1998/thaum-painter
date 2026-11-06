use std::{cell::RefCell, rc::Rc};

use thaum_renderer_domain::{
    title_hotspot, CellGroup, CellGroupIntakeBehavior, GizmoBar, GizmoClickOutcome, GizmoKind,
    GizmoState, Hotspot, Module, ModulePointerButton, ModulePointerEvent, ModuleRect,
    NumberFieldEdit, PanelChrome, PersistedModuleUiState, PropertyHit, PropertyRow, PropertyRows,
    UiPalette, WorldPoint,
};

use crate::display_profile_presets::{
    built_in_display_profile_presets, matching_preset_index, DisplayProfilePreset,
};
use crate::storage::SharedDocumentDisplayProfile;

/// The reserved value `spriteset` cycles to for "no spriteset override" —
/// matches `thaum_renderer_boot::NO_SPRITESET_OVERRIDE_ID`. Kept as a local
/// literal rather than a cross-crate dependency since the painter's module
/// layer intentionally does not depend on the boot crate.
const NO_SPRITESET_OVERRIDE_ID: &str = "none";

/// The `cell_size` row's editable clamp range. A floor above zero keeps a
/// nudge from ever producing an unrenderable zero-size cell; the ceiling
/// keeps the field's 3-char render budget (see `PropertyRows`) honest.
pub const MIN_CELL_DIMENSION: i32 = 4;
pub const MAX_CELL_DIMENSION: i32 = 99;

pub struct DisplayProfileModule {
    id: String,
    rect: ModuleRect,
    profile: Rc<RefCell<SharedDocumentDisplayProfile>>,
    typeface_options: Vec<String>,
    /// Discovered spriteset ids with `NO_SPRITESET_OVERRIDE_ID` prepended —
    /// the discovery helpers never emit that sentinel themselves.
    spriteset_options: Vec<String>,
    presets: &'static [DisplayProfilePreset],
    number_edit: Rc<RefCell<Option<NumberFieldEdit>>>,
    palette: UiPalette,
    gizmos: GizmoBar,
    gizmo_state: GizmoState,
    hidden: bool,
}

impl DisplayProfileModule {
    pub fn new(
        id: impl Into<String>,
        rect: ModuleRect,
        profile: Rc<RefCell<SharedDocumentDisplayProfile>>,
        typeface_options: Vec<String>,
        spriteset_options: Vec<String>,
        number_edit: Rc<RefCell<Option<NumberFieldEdit>>>,
    ) -> Self {
        let mut full_spriteset_options = vec![NO_SPRITESET_OVERRIDE_ID.to_owned()];
        full_spriteset_options.extend(spriteset_options);
        Self {
            id: id.into(),
            rect,
            profile,
            typeface_options,
            spriteset_options: full_spriteset_options,
            presets: built_in_display_profile_presets(),
            number_edit,
            palette: UiPalette::default(),
            gizmos: GizmoBar::standard(),
            gizmo_state: GizmoState::new(),
            hidden: false,
        }
    }

    pub fn with_palette(mut self, palette: UiPalette) -> Self {
        self.palette = palette;
        self
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

    fn build_rows(&self) -> Vec<PropertyRow> {
        let profile = self.profile.borrow();
        let preset_value = matching_preset_index(self.presets, &profile)
            .map(|index| self.presets[index].label.to_owned())
            .unwrap_or_else(|| "custom".to_owned());
        vec![
            PropertyRow::ListSwitchRow {
                id: "preset".into(),
                label: "preset".into(),
                value: preset_value,
                option_count: self.presets.len(),
            },
            PropertyRow::NumberRow {
                id: "cell_size".into(),
                label: "size".into(),
                values: vec![profile.cell_width as i32, profile.cell_height as i32],
                min: MIN_CELL_DIMENSION,
                max: MAX_CELL_DIMENSION,
                editing: self.editing_for("cell_size"),
            },
            PropertyRow::ListSwitchRow {
                id: "typeface".into(),
                label: "font".into(),
                value: profile.typeface_id.clone(),
                option_count: self.typeface_options.len(),
            },
            PropertyRow::ListSwitchRow {
                id: "spriteset".into(),
                label: "sprites".into(),
                value: profile.spriteset_id.clone(),
                option_count: self.spriteset_options.len(),
            },
        ]
    }

    /// Steps `row_id`'s selection forward or backward through its option
    /// list, wrapping at either end. A current value absent from the list
    /// (an older/custom id the discovery pass didn't find) falls back to
    /// index 0 so the switcher still lands on a valid choice.
    fn step_list_switch(&mut self, row_id: &str, forward: bool) {
        let options: &[String] = match row_id {
            "typeface" => &self.typeface_options,
            "spriteset" => &self.spriteset_options,
            _ => return,
        };
        if options.is_empty() {
            return;
        }
        let mut profile = self.profile.borrow_mut();
        let current = match row_id {
            "typeface" => profile.typeface_id.clone(),
            "spriteset" => profile.spriteset_id.clone(),
            _ => return,
        };
        let current_index = options.iter().position(|option| *option == current).unwrap_or(0);
        let next_index = if forward {
            (current_index + 1) % options.len()
        } else {
            (current_index + options.len() - 1) % options.len()
        };
        let next_value = options[next_index].clone();
        match row_id {
            "typeface" => profile.typeface_id = next_value,
            "spriteset" => profile.spriteset_id = next_value,
            _ => {}
        }
    }

    /// Steps the preset switcher forward or backward, wrapping at either
    /// end, and applies the landed preset's four fields atomically. A
    /// hand-edited ("custom") combination that matches no preset falls back
    /// to index 0, same as the typeface/spriteset switchers.
    fn step_preset(&mut self, forward: bool) {
        if self.presets.is_empty() {
            return;
        }
        let mut profile = self.profile.borrow_mut();
        let current_index = matching_preset_index(self.presets, &profile).unwrap_or(0);
        let next_index = if forward {
            (current_index + 1) % self.presets.len()
        } else {
            (current_index + self.presets.len() - 1) % self.presets.len()
        };
        self.presets[next_index].apply(&mut profile);
    }

    fn apply_property_hit(&mut self, hit: PropertyHit) {
        match hit {
            PropertyHit::Number { row_id, field } if row_id == "cell_size" => {
                let current = {
                    let profile = self.profile.borrow();
                    if field == 0 {
                        profile.cell_width as i32
                    } else {
                        profile.cell_height as i32
                    }
                };
                *self.number_edit.borrow_mut() =
                    Some(NumberFieldEdit::begin(row_id, field, current));
            }
            PropertyHit::ListSwitch { row_id, forward } if row_id == "preset" => {
                self.step_preset(forward);
            }
            PropertyHit::ListSwitch { row_id, forward } => {
                self.step_list_switch(&row_id, forward);
            }
            _ => {}
        }
    }
}

impl Module for DisplayProfileModule {
    fn id(&self) -> &str {
        &self.id
    }

    fn rect(&self) -> ModuleRect {
        self.rect
    }

    fn hotspots(&self) -> Vec<Hotspot> {
        let rows = self.build_rows();
        let mut custom = vec![title_hotspot(
            self.rect,
            self.gizmos.title_start_x(),
            "display profile module",
            "this file's own cell width/height, typeface, and spriteset. click a size field to type a value, scroll to nudge it; click a switcher's arrow to cycle its choices.",
        )];
        custom.extend(PropertyRows::hotspots(self.rect, &rows, 0));
        self.gizmos.hotspots_with(self.rect, custom)
    }

    fn draw(&self) -> CellGroup {
        let origin = WorldPoint {
            x: self.rect.x0,
            y: self.rect.y0,
            z: 0,
        };
        let mut cells = if self.gizmo_state.is_seamless() {
            Vec::new()
        } else {
            self.gizmo_state
                .decorate_panel_chrome(
                    PanelChrome::new(self.rect, &self.palette)
                        .with_title("DISPLAY")
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
            &self.build_rows(),
            &self.palette,
            0,
        ));

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
                let rows = self.build_rows();
                if let Some(hit) = PropertyRows::hit_test(self.rect, &rows, x, y, button, 0) {
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

    /// Scroll on the size row nudges that field by one, clamped to its
    /// range. Anywhere else this module has nothing to scroll.
    fn on_wheel(&mut self, x: i32, y: i32, _delta_x: f32, delta_y: f32) -> bool {
        if self.number_edit.borrow().is_some() {
            return true;
        }
        let rows = self.build_rows();
        let Some((row_id, field)) = PropertyRows::number_field_at(self.rect, &rows, x, y, 0)
        else {
            return false;
        };
        if row_id != "cell_size" {
            return false;
        }
        let delta = if delta_y > 0.0 {
            1
        } else if delta_y < 0.0 {
            -1
        } else {
            return true;
        };
        let mut profile = self.profile.borrow_mut();
        if field == 0 {
            profile.cell_width = (profile.cell_width as i32 + delta)
                .clamp(MIN_CELL_DIMENSION, MAX_CELL_DIMENSION) as u32;
        } else {
            profile.cell_height = (profile.cell_height as i32 + delta)
                .clamp(MIN_CELL_DIMENSION, MAX_CELL_DIMENSION) as u32;
        }
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

    fn rect() -> ModuleRect {
        ModuleRect {
            x0: 0,
            y0: 0,
            x1: 24,
            y1: 6,
        }
    }

    fn module_with(
        profile: SharedDocumentDisplayProfile,
        typefaces: Vec<&str>,
        spritesets: Vec<&str>,
    ) -> DisplayProfileModule {
        DisplayProfileModule::new(
            "display_profile",
            rect(),
            Rc::new(RefCell::new(profile)),
            typefaces.into_iter().map(str::to_owned).collect(),
            spritesets.into_iter().map(str::to_owned).collect(),
            Rc::new(RefCell::new(None)),
        )
    }

    #[test]
    fn rows_reflect_the_live_profile_and_option_counts() {
        let module = module_with(
            SharedDocumentDisplayProfile {
                cell_width: 12,
                cell_height: 16,
                typeface_id: "thaum-mono".into(),
                spriteset_id: "monothaum-atlas-v3".into(),
            },
            vec!["thaum-mono", "jetbrains-mono"],
            vec!["monothaum-atlas-v3"],
        );
        let rows = module.build_rows();
        // The live profile exactly matches the built-in "thaum-mono" preset.
        assert_eq!(
            rows[0],
            PropertyRow::ListSwitchRow {
                id: "preset".into(),
                label: "preset".into(),
                value: "thaum-mono".into(),
                option_count: built_in_display_profile_presets().len(),
            }
        );
        assert_eq!(
            rows[1],
            PropertyRow::NumberRow {
                id: "cell_size".into(),
                label: "size".into(),
                values: vec![12, 16],
                min: MIN_CELL_DIMENSION,
                max: MAX_CELL_DIMENSION,
                editing: None,
            }
        );
        assert_eq!(
            rows[2],
            PropertyRow::ListSwitchRow {
                id: "typeface".into(),
                label: "font".into(),
                value: "thaum-mono".into(),
                option_count: 2,
            }
        );
        // "none" is prepended ahead of the one discovered spriteset.
        assert_eq!(
            rows[3],
            PropertyRow::ListSwitchRow {
                id: "spriteset".into(),
                label: "sprites".into(),
                value: "monothaum-atlas-v3".into(),
                option_count: 2,
            }
        );
    }

    #[test]
    fn the_preset_row_reports_custom_for_a_hand_edited_combination() {
        let module = module_with(
            SharedDocumentDisplayProfile {
                cell_width: 13,
                cell_height: 16,
                typeface_id: "thaum-mono".into(),
                spriteset_id: "monothaum-atlas-v3".into(),
            },
            vec!["thaum-mono"],
            vec!["monothaum-atlas-v3"],
        );
        let rows = module.build_rows();
        assert_eq!(
            rows[0],
            PropertyRow::ListSwitchRow {
                id: "preset".into(),
                label: "preset".into(),
                value: "custom".into(),
                option_count: built_in_display_profile_presets().len(),
            }
        );
    }

    #[test]
    fn switching_the_preset_applies_all_four_fields_atomically() {
        let mut module = module_with(
            SharedDocumentDisplayProfile {
                cell_width: 12,
                cell_height: 16,
                typeface_id: "thaum-mono".into(),
                spriteset_id: "monothaum-atlas-v3".into(),
            },
            vec!["thaum-mono", "jetbrains-mono"],
            vec!["monothaum-atlas-v3"],
        );

        module.step_preset(true);

        let profile = module.profile.borrow();
        assert_eq!(profile.cell_width, 10);
        assert_eq!(profile.cell_height, 22);
        assert_eq!(profile.typeface_id, "jetbrains-mono");
        assert_eq!(profile.spriteset_id, "none");
    }

    #[test]
    fn switching_the_typeface_forward_wraps_to_the_first_option() {
        let mut module = module_with(
            SharedDocumentDisplayProfile {
                cell_width: 12,
                cell_height: 16,
                typeface_id: "jetbrains-mono".into(),
                spriteset_id: "monothaum-atlas-v3".into(),
            },
            vec!["thaum-mono", "jetbrains-mono"],
            vec!["monothaum-atlas-v3"],
        );

        module.step_list_switch("typeface", true);
        assert_eq!(module.profile.borrow().typeface_id, "thaum-mono");
    }

    #[test]
    fn switching_the_spriteset_backward_lands_on_the_none_sentinel() {
        let mut module = module_with(
            SharedDocumentDisplayProfile {
                cell_width: 12,
                cell_height: 16,
                typeface_id: "thaum-mono".into(),
                spriteset_id: "monothaum-atlas-v3".into(),
            },
            vec!["thaum-mono"],
            vec!["monothaum-atlas-v3"],
        );

        module.step_list_switch("spriteset", false);
        assert_eq!(module.profile.borrow().spriteset_id, "none");
    }

    #[test]
    fn a_current_value_missing_from_the_options_falls_back_to_the_first_choice() {
        let mut module = module_with(
            SharedDocumentDisplayProfile {
                cell_width: 12,
                cell_height: 16,
                typeface_id: "some-removed-typeface".into(),
                spriteset_id: "monothaum-atlas-v3".into(),
            },
            vec!["thaum-mono", "jetbrains-mono"],
            vec!["monothaum-atlas-v3"],
        );

        module.step_list_switch("typeface", true);
        assert_eq!(module.profile.borrow().typeface_id, "jetbrains-mono");
    }

    #[test]
    fn clicking_a_size_field_opens_a_number_edit_seeded_with_the_live_value() {
        let mut module = module_with(
            SharedDocumentDisplayProfile {
                cell_width: 12,
                cell_height: 16,
                typeface_id: "thaum-mono".into(),
                spriteset_id: "monothaum-atlas-v3".into(),
            },
            vec!["thaum-mono"],
            vec!["monothaum-atlas-v3"],
        );
        let (_, value_x, _) = PropertyRows::content_columns(rect());
        // Row 0 is the preset switcher; "cell_size" is row 1.
        let field_y = rect().y0 + PropertyRows::top_row_y(rect()) - 1;

        module.on_pointer_event(ModulePointerEvent::Click {
            x: rect().x0 + value_x,
            y: field_y,
            button: ModulePointerButton::Left,
        });

        let edit = module.number_edit.borrow();
        let edit = edit.as_ref().unwrap();
        assert_eq!(edit.row_id, "cell_size");
        assert_eq!(edit.field, 0);
        assert_eq!(edit.buffer, "12");
    }

    #[test]
    fn clicking_a_switch_arrow_steps_the_selection() {
        let mut module = module_with(
            SharedDocumentDisplayProfile {
                cell_width: 12,
                cell_height: 16,
                typeface_id: "thaum-mono".into(),
                spriteset_id: "monothaum-atlas-v3".into(),
            },
            vec!["thaum-mono", "jetbrains-mono"],
            vec!["monothaum-atlas-v3"],
        );
        let (_, value_x, value_width) = PropertyRows::content_columns(rect());
        let right_arrow_x = rect().x0 + value_x + value_width - 1;
        // Rows: 0 preset, 1 cell_size, 2 typeface.
        let typeface_row_y = rect().y0 + PropertyRows::top_row_y(rect()) - 2;

        module.on_pointer_event(ModulePointerEvent::Click {
            x: right_arrow_x,
            y: typeface_row_y,
            button: ModulePointerButton::Left,
        });

        assert_eq!(module.profile.borrow().typeface_id, "jetbrains-mono");
    }
}
