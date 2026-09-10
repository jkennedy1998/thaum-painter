use std::{cell::RefCell, rc::Rc};

use thaum_renderer_domain::{
    Cell, CellGraphic, CellGroup, CellGroupIntakeBehavior, CellMaterialId, CellPoint, CellWeight,
    ColorBand, GizmoBar, GizmoClickOutcome, GizmoKind, GizmoState, Hotspot, Module,
    ModulePointerButton, ModulePointerEvent, ModuleRect, PanelChrome, PersistedModuleUiState,
    UiColorRole, UiPalette, WorldPoint, title_hotspot,
};

use crate::{
    paint_color::PaintColor,
    tool_state::{PaintHand, ToolState},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MaterialHit {
    y: i32,
    material: CellMaterialId,
}

fn supported_materials() -> &'static [CellMaterialId] {
    CellMaterialId::all()
}

fn band_preview_cell(
    material: CellMaterialId,
    band: ColorBand,
) -> thaum_renderer_domain::CellColor {
    let rgba = material.resolve_band(band);
    thaum_renderer_domain::CellColor::Flat(rgba)
}

fn assignment_role(tool_state: &ToolState, material: CellMaterialId) -> UiColorRole {
    let is_left = tool_state.left_hand.color == PaintColor::material(material);
    let is_right = tool_state.right_hand.color == PaintColor::material(material);
    if is_left && is_right {
        UiColorRole::Vivid
    } else if is_left {
        UiColorRole::LeftHand
    } else if is_right {
        UiColorRole::RightHand
    } else {
        UiColorRole::Medium
    }
}

fn assignment_indicator(tool_state: &ToolState, material: CellMaterialId) -> char {
    let is_left = tool_state.left_hand.color == PaintColor::material(material);
    let is_right = tool_state.right_hand.color == PaintColor::material(material);
    if is_left && is_right {
        '◆'
    } else if is_left {
        'L'
    } else if is_right {
        'R'
    } else {
        ' '
    }
}

pub struct MaterialPickerModule {
    id: String,
    rect: ModuleRect,
    tool_state: Rc<RefCell<ToolState>>,
    palette: UiPalette,
    gizmos: GizmoBar,
    gizmo_state: GizmoState,
    hidden: bool,
}

impl MaterialPickerModule {
    pub fn new(
        id: impl Into<String>,
        rect: ModuleRect,
        tool_state: Rc<RefCell<ToolState>>,
    ) -> Self {
        Self {
            id: id.into(),
            rect,
            tool_state,
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

    fn build_layout(&self) -> (Vec<Cell>, Vec<MaterialHit>) {
        let (content_x, content_y) = PanelChrome::content_origin();
        let (_, content_height) = PanelChrome::content_size(self.rect);
        let mut cells: Vec<Cell> = if self.gizmo_state.is_seamless() {
            Vec::new()
        } else {
            self.gizmo_state
                .decorate_panel_chrome(
                    PanelChrome::new(self.rect, &self.palette)
                        .with_title("MATERIALS")
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

        let state = self.tool_state.borrow();
        let mut hits = Vec::new();
        let mut y = content_y + content_height - 1;
        for material in supported_materials() {
            let role = assignment_role(&state, *material);
            cells.push(Cell {
                position: CellPoint {
                    x: content_x,
                    y,
                    z: 0,
                },
                graphic: CellGraphic::Glyph(assignment_indicator(&state, *material)),
                color: self.palette.get(role),
                weight: CellWeight::from_index_clamped(2),
                ..Cell::default()
            });
            for (offset, band) in [
                ColorBand::Darkest,
                ColorBand::MediumDark,
                ColorBand::MediumLight,
                ColorBand::Lightest,
            ]
            .into_iter()
            .enumerate()
            {
                cells.push(Cell {
                    position: CellPoint {
                        x: content_x + 2 + offset as i32,
                        y,
                        z: 0,
                    },
                    graphic: CellGraphic::Glyph('█'),
                    color: band_preview_cell(*material, band),
                    weight: CellWeight::from_index_clamped(3),
                    ..Cell::default()
                });
            }
            for (offset, glyph) in material.label().to_ascii_uppercase().chars().enumerate() {
                cells.push(Cell {
                    position: CellPoint {
                        x: content_x + 7 + offset as i32,
                        y,
                        z: 0,
                    },
                    graphic: CellGraphic::Glyph(glyph),
                    color: self.palette.get(role),
                    weight: CellWeight::from_index_clamped(2),
                    ..Cell::default()
                });
            }
            hits.push(MaterialHit {
                y,
                material: *material,
            });
            y -= 1;
        }

        (cells, hits)
    }

    fn hit_material_at(&self, x: i32, y: i32) -> Option<CellMaterialId> {
        let (_, hits) = self.build_layout();
        let local_x = x - self.rect.x0;
        let local_y = y - self.rect.y0;
        (local_x >= PanelChrome::content_origin().0)
            .then_some(())
            .and_then(|_| {
                hits.into_iter()
                    .find(|hit| hit.y == local_y)
                    .map(|hit| hit.material)
            })
    }
}

impl Module for MaterialPickerModule {
    fn id(&self) -> &str {
        &self.id
    }

    fn rect(&self) -> ModuleRect {
        self.rect
    }

    /// Tooltip hotspots: the module's gizmo bar plus one row hotspot per
    /// material, so hovering a material explains assignment.
    fn hotspots(&self) -> Vec<Hotspot> {
        let (_, hits) = self.build_layout();
        let (content_x, content_width) = {
            let (x, _) = PanelChrome::content_origin();
            let (w, _) = PanelChrome::content_size(self.rect);
            (x, w - 1)
        };
        let mut custom = vec![title_hotspot(
            self.rect,
            self.gizmos.title_start_x(),
            "materials module",
            "this is largely a stub development place for lighting systems for thaumworld :3",
        )];
        custom.extend(
            hits
            .into_iter()
            .map(|hit| {
                let name = hit.material.label().to_ascii_uppercase();
                Hotspot::new(
                    ModuleRect {
                        x0: self.rect.x0 + content_x,
                        y0: self.rect.y0 + hit.y,
                        x1: self.rect.x0 + content_x + content_width,
                        y1: self.rect.y0 + hit.y,
                    },
                    name,
                    "click to assign the material to a hand: left-click left, right-click right",
                )
            }),
        );
        self.gizmos.hotspots_with(self.rect, custom)
    }

    fn draw(&self) -> CellGroup {
        let origin = WorldPoint {
            x: self.rect.x0,
            y: self.rect.y0,
            z: 0,
        };
        let (cells, _) = self.build_layout();
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
                let hand = match button {
                    ModulePointerButton::Left => PaintHand::Left,
                    ModulePointerButton::Right => PaintHand::Right,
                    ModulePointerButton::Middle => return,
                };
                let Some(material) = self.hit_material_at(x, y) else {
                    return;
                };
                self.tool_state
                    .borrow_mut()
                    .set_color_for_hand(hand, PaintColor::material(material));
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
            y1: 8,
        }
    }

    #[test]
    fn left_click_assigns_the_material_to_the_left_hand() {
        let state = Rc::new(RefCell::new(ToolState::default()));
        let mut module = MaterialPickerModule::new("materials", rect(), state.clone());

        module.on_pointer_event(ModulePointerEvent::Click {
            x: 3,
            y: 5,
            button: ModulePointerButton::Left,
        });

        assert_eq!(
            state.borrow().left_hand.color,
            PaintColor::material(CellMaterialId::GrayScale)
        );
    }
}
