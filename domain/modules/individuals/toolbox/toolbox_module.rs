use std::{cell::RefCell, rc::Rc};

use thaum_renderer_domain::{
    Cell, CellGraphic, CellGroup, CellGroupIntakeBehavior, CellPoint, GizmoBar, GizmoClickOutcome,
    GizmoKind, GizmoState, Module, ModulePointerButton, ModulePointerEvent, ModuleRect,
    PanelChrome, PersistedModuleUiState, UiColorRole, UiPalette, WorldPoint,
};

use crate::tool_state::{PaintHand, PaintTool, ToolState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolDef {
    pub tool: PaintTool,
    pub icon: char,
    pub label: &'static str,
}

fn standard_gizmo_bar() -> GizmoBar {
    GizmoBar::new(vec![
        GizmoKind::Move,
        GizmoKind::Close,
        GizmoKind::Resize,
        GizmoKind::Seamless,
    ])
}

pub struct ToolboxModule {
    id: String,
    rect: ModuleRect,
    tool_state: Rc<RefCell<ToolState>>,
    tool_defs: Vec<ToolDef>,
    palette: UiPalette,
    gizmos: GizmoBar,
    gizmo_state: GizmoState,
    hidden: bool,
}

impl ToolboxModule {
    pub fn new(
        id: impl Into<String>,
        rect: ModuleRect,
        tool_state: Rc<RefCell<ToolState>>,
        tool_defs: Vec<ToolDef>,
    ) -> Self {
        Self {
            id: id.into(),
            rect,
            tool_state,
            tool_defs,
            palette: UiPalette::default(),
            gizmos: standard_gizmo_bar(),
            gizmo_state: GizmoState::new(),
            hidden: false,
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

    fn tool_at(&self, x: i32, y: i32) -> Option<PaintTool> {
        let local_x = x - self.rect.x0;
        let local_y = y - self.rect.y0;
        let (content_x, _) = PanelChrome::content_origin();
        let (content_width, _) = PanelChrome::content_size(self.rect);
        if local_x < content_x || local_x >= content_x + content_width {
            return None;
        }
        self.tool_defs
            .iter()
            .enumerate()
            .find(|(index, _)| local_y == self.row_y(*index))
            .map(|(_, def)| def.tool)
    }
}

impl Module for ToolboxModule {
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
                        .with_title("TOOLS")
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
        let current = state.current_tool();

        for (index, def) in self.tool_defs.iter().enumerate() {
            let y = self.row_y(index);
            let is_left = state.left_tool == def.tool;
            let is_right = state.right_tool == def.tool;
            let is_current = current == def.tool;
            let indicator = if is_left && is_right {
                ('◆', self.palette.get(UiColorRole::Vivid))
            } else if is_left {
                ('L', self.palette.get(UiColorRole::LeftHand))
            } else if is_right {
                ('R', self.palette.get(UiColorRole::RightHand))
            } else {
                (' ', self.palette.get(UiColorRole::Medium))
            };
            let text_color = if is_current {
                self.palette.get(UiColorRole::Bright)
            } else {
                self.palette.get(UiColorRole::Medium)
            };
            let icon_color = if is_left && is_right {
                self.palette.get(UiColorRole::Vivid)
            } else if is_left {
                self.palette.get(UiColorRole::LeftHand)
            } else if is_right {
                self.palette.get(UiColorRole::RightHand)
            } else {
                text_color
            };

            cells.push(Cell {
                position: CellPoint { x: 1, y, z: 0 },
                graphic: CellGraphic::Glyph(indicator.0),
                color: indicator.1,
                ..Cell::default()
            });
            cells.push(Cell {
                position: CellPoint { x: 3, y, z: 0 },
                graphic: CellGraphic::Glyph(def.icon),
                color: icon_color,
                ..Cell::default()
            });
            for (column, glyph) in def.label.chars().enumerate() {
                let x = 5 + column as i32;
                if x > self.rect.x1 - self.rect.x0 - 1 {
                    break;
                }
                cells.push(Cell {
                    position: CellPoint { x, y, z: 0 },
                    graphic: CellGraphic::Glyph(glyph),
                    color: text_color,
                    ..Cell::default()
                });
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
                let Some(tool) = self.tool_at(x, y) else {
                    return;
                };
                let mut state = self.tool_state.borrow_mut();
                match button {
                    ModulePointerButton::Left => state.set_tool_for_hand(PaintHand::Left, tool),
                    ModulePointerButton::Right => state.set_tool_for_hand(PaintHand::Right, tool),
                    ModulePointerButton::Middle => {}
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
            x1: 18,
            y1: 6,
        }
    }

    fn tool_state() -> Rc<RefCell<ToolState>> {
        Rc::new(RefCell::new(ToolState::default()))
    }

    fn tool_defs() -> Vec<ToolDef> {
        vec![
            ToolDef {
                tool: PaintTool::Brush,
                icon: 'B',
                label: "Brush",
            },
            ToolDef {
                tool: PaintTool::Erase,
                icon: 'E',
                label: "Erase",
            },
            ToolDef {
                tool: PaintTool::Fill,
                icon: 'F',
                label: "Fill",
            },
        ]
    }

    #[test]
    fn left_click_assigns_the_left_hand_tool() {
        let state = tool_state();
        let mut toolbox = ToolboxModule::new("toolbox", rect(), state.clone(), tool_defs());

        toolbox.on_pointer_event(ModulePointerEvent::Click {
            x: 3,
            y: 2,
            button: ModulePointerButton::Left,
        });

        let state = state.borrow();
        assert_eq!(state.left_tool, PaintTool::Erase);
        assert_eq!(state.right_tool, PaintTool::Erase);
    }

    #[test]
    fn right_click_assigns_only_the_right_hand_tool() {
        let state = tool_state();
        let mut toolbox = ToolboxModule::new("toolbox", rect(), state.clone(), tool_defs());

        toolbox.on_pointer_event(ModulePointerEvent::Click {
            x: 3,
            y: 1,
            button: ModulePointerButton::Right,
        });

        let state = state.borrow();
        assert_eq!(state.left_tool, PaintTool::Brush);
        assert_eq!(state.right_tool, PaintTool::Fill);
    }

    #[test]
    fn draw_marks_left_and_right_assignments() {
        let state = tool_state();
        {
            let mut state = state.borrow_mut();
            state.set_tool_for_hand(PaintHand::Left, PaintTool::Brush);
            state.set_tool_for_hand(PaintHand::Right, PaintTool::Fill);
        }
        let toolbox = ToolboxModule::new("toolbox", rect(), state, tool_defs());

        let group = toolbox.draw();

        assert_eq!(
            group.cells[&CellPoint { x: 1, y: 3, z: 0 }].graphic,
            CellGraphic::Glyph('L')
        );
        assert_eq!(
            group.cells[&CellPoint { x: 1, y: 1, z: 0 }].graphic,
            CellGraphic::Glyph('R')
        );
    }

    #[test]
    fn click_outside_rows_is_ignored() {
        let state = tool_state();
        let mut toolbox = ToolboxModule::new("toolbox", rect(), state.clone(), tool_defs());

        toolbox.on_pointer_event(ModulePointerEvent::Click {
            x: 3,
            y: 4,
            button: ModulePointerButton::Left,
        });

        let state = state.borrow();
        assert_eq!(state.left_tool, PaintTool::Brush);
        assert_eq!(state.right_tool, PaintTool::Erase);
    }

    #[test]
    fn draw_marks_shared_assignment_with_both_indicator() {
        let state = tool_state();
        {
            let mut state = state.borrow_mut();
            state.set_tool_for_hand(PaintHand::Left, PaintTool::Fill);
            state.set_tool_for_hand(PaintHand::Right, PaintTool::Fill);
        }
        let toolbox = ToolboxModule::new("toolbox", rect(), state, tool_defs());

        let group = toolbox.draw();

        assert_eq!(
            group.cells[&CellPoint { x: 1, y: 1, z: 0 }].graphic,
            CellGraphic::Glyph('◆')
        );
    }

    #[test]
    fn clicking_the_move_gizmo_starts_requesting_pointer_capture() {
        let state = tool_state();
        let mut toolbox = ToolboxModule::new("toolbox", rect(), state, tool_defs());

        toolbox.on_pointer_event(ModulePointerEvent::Click {
            x: 1,
            y: 5,
            button: ModulePointerButton::Left,
        });

        assert!(toolbox.wants_pointer_capture());
    }
}
