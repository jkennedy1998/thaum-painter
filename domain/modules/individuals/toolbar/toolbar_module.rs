use thaum_renderer_domain::{
    Cell, CellColor, CellGraphic, CellGroup, CellGroupIntakeBehavior, CellPoint, Module,
    ModulePointerEvent, ModuleRect, WorldPoint,
};

/// One tool button in the toolbar: the glyph drawn and the tool it selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolbarButton {
    pub glyph: char,
    pub tool: &'static str,
}

/// Painter's own toolbar panel: a row of tool buttons drawn through the
/// renderer's `Module` trait, owned here rather than in
/// `thaum-renderer/domain/modules/` because no other consumer needs it.
pub struct ToolbarModule {
    id: String,
    rect: ModuleRect,
    buttons: Vec<ToolbarButton>,
    active_index: usize,
}

impl ToolbarModule {
    pub fn new(id: impl Into<String>, rect: ModuleRect, buttons: Vec<ToolbarButton>) -> Self {
        Self {
            id: id.into(),
            rect,
            buttons,
            active_index: 0,
        }
    }

    pub fn active_tool(&self) -> Option<&'static str> {
        self.buttons
            .get(self.active_index)
            .map(|button| button.tool)
    }
}

impl Module for ToolbarModule {
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

        let cells = self.buttons.iter().enumerate().map(|(index, button)| {
            let color = if index == self.active_index {
                CellColor::Flat([1.0, 0.85, 0.4, 1.0])
            } else {
                CellColor::Flat([0.7, 0.7, 0.7, 1.0])
            };

            Cell {
                position: CellPoint {
                    x: index as i32 * 2,
                    y: 0,
                    z: 0,
                },
                graphic: CellGraphic::Glyph(button.glyph),
                color,
                ..Cell::default()
            }
        });

        CellGroup::from_cells(origin, cells).with_intake_behavior(CellGroupIntakeBehavior::Flat2d)
    }

    fn on_pointer_event(&mut self, event: ModulePointerEvent) {
        let ModulePointerEvent::Click { x, y: _, .. } = event else {
            return;
        };

        let clicked_index = (x - self.rect.x0) / 2;
        if clicked_index >= 0 && (clicked_index as usize) < self.buttons.len() {
            self.active_index = clicked_index as usize;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use thaum_renderer_domain::ModulePointerButton;

    fn rect(x0: i32, y0: i32, x1: i32, y1: i32) -> ModuleRect {
        ModuleRect { x0, y0, x1, y1 }
    }

    fn sample_buttons() -> Vec<ToolbarButton> {
        vec![
            ToolbarButton {
                glyph: 'B',
                tool: "brush",
            },
            ToolbarButton {
                glyph: 'E',
                tool: "eraser",
            },
            ToolbarButton {
                glyph: 'F',
                tool: "fill",
            },
        ]
    }

    #[test]
    fn draws_one_cell_per_button_spaced_two_apart() {
        let toolbar = ToolbarModule::new("toolbar", rect(0, 0, 10, 0), sample_buttons());
        let group = toolbar.draw();

        assert_eq!(group.cells.len(), 3);
        let second = &group.cells[&CellPoint { x: 2, y: 0, z: 0 }];
        assert_eq!(second.graphic, CellGraphic::Glyph('E'));
    }

    #[test]
    fn first_button_is_active_by_default() {
        let toolbar = ToolbarModule::new("toolbar", rect(0, 0, 10, 0), sample_buttons());
        assert_eq!(toolbar.active_tool(), Some("brush"));
    }

    #[test]
    fn clicking_a_button_makes_it_active() {
        let mut toolbar = ToolbarModule::new("toolbar", rect(0, 0, 10, 0), sample_buttons());
        toolbar.on_pointer_event(ModulePointerEvent::Click {
            x: 2,
            y: 0,
            button: ModulePointerButton::Left,
        });
        assert_eq!(toolbar.active_tool(), Some("eraser"));
    }

    #[test]
    fn click_outside_buttons_is_ignored() {
        let mut toolbar = ToolbarModule::new("toolbar", rect(0, 0, 10, 0), sample_buttons());
        toolbar.on_pointer_event(ModulePointerEvent::Click {
            x: 9,
            y: 0,
            button: ModulePointerButton::Left,
        });
        assert_eq!(toolbar.active_tool(), Some("brush"));
    }
}
