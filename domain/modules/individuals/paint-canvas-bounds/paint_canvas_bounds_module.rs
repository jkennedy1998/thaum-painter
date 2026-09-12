use std::{cell::RefCell, rc::Rc};

use thaum_renderer_domain::{
    title_hotspot, Cell, CellGraphic, CellGroup, CellGroupIntakeBehavior, CellPoint, CellWeight,
    GizmoBar, GizmoClickOutcome, GizmoKind, GizmoState, Hotspot, Module, ModulePointerEvent,
    ModuleRect, PanelChrome, PersistedModuleUiState, UiColorRole, UiPalette, WorldPoint,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrawingSpaceWheelMode {
    Pan,
    Depth,
    /// Wheel steps the playhead through time, wrapping at the document
    /// loop-window edges (scroll up = forward, down = backward).
    Time,
}

impl DrawingSpaceWheelMode {
    fn glyph(self) -> char {
        match self {
            DrawingSpaceWheelMode::Pan => 'P',
            DrawingSpaceWheelMode::Depth => 'D',
            DrawingSpaceWheelMode::Time => 'T',
        }
    }

    fn cycle(self) -> Self {
        match self {
            DrawingSpaceWheelMode::Pan => DrawingSpaceWheelMode::Depth,
            DrawingSpaceWheelMode::Depth => DrawingSpaceWheelMode::Time,
            DrawingSpaceWheelMode::Time => DrawingSpaceWheelMode::Pan,
        }
    }
}

pub struct PaintCanvasBoundsModule {
    id: String,
    viewport: Rc<RefCell<ModuleRect>>,
    wheel_mode: Rc<RefCell<DrawingSpaceWheelMode>>,
    palette: UiPalette,
    gizmos: GizmoBar,
    gizmo_state: GizmoState,
    wheel_mode_hovered: bool,
    hidden: bool,
}

impl PaintCanvasBoundsModule {
    pub fn new(
        id: impl Into<String>,
        viewport: Rc<RefCell<ModuleRect>>,
        wheel_mode: Rc<RefCell<DrawingSpaceWheelMode>>,
        palette: UiPalette,
    ) -> Self {
        Self {
            id: id.into(),
            viewport,
            wheel_mode,
            palette,
            gizmos: GizmoBar::standard(),
            gizmo_state: GizmoState::new(),
            wheel_mode_hovered: false,
            hidden: false,
        }
    }

    pub fn content_rect(viewport: ModuleRect) -> ModuleRect {
        PanelChrome::content_rect(viewport)
    }

    pub fn is_gizmo_hit(viewport: ModuleRect, x: i32, y: i32) -> bool {
        GizmoBar::standard().hit_test(viewport, x, y).is_some()
            || Self::is_wheel_mode_hit(viewport, x, y)
    }

    fn wheel_mode_local_x(&self) -> i32 {
        self.gizmos.title_start_x()
    }

    fn title_start_x(&self) -> i32 {
        self.wheel_mode_local_x() + 2
    }

    fn is_wheel_mode_hit(viewport: ModuleRect, x: i32, y: i32) -> bool {
        let local_x = x - viewport.x0;
        let local_y = y - viewport.y0;
        let height = viewport.y1 - viewport.y0;
        local_y == height - 1 && local_x == GizmoBar::standard().title_start_x()
    }

    fn set_viewport(&mut self, rect: ModuleRect) {
        *self.viewport.borrow_mut() = rect;
    }

    fn cycle_wheel_mode(&mut self) {
        let next = self.wheel_mode.borrow().cycle();
        *self.wheel_mode.borrow_mut() = next;
    }

    fn wheel_mode_cell(&self, rect: ModuleRect) -> Cell {
        let height = rect.y1 - rect.y0;
        Cell {
            position: CellPoint {
                x: self.wheel_mode_local_x(),
                y: height - 1,
                z: 0,
            },
            graphic: CellGraphic::Glyph(self.wheel_mode.borrow().glyph()),
            color: if self.wheel_mode_hovered {
                self.palette.get(UiColorRole::Vivid)
            } else {
                self.palette.get(UiColorRole::Medium)
            },
            weight: CellWeight::from_index_clamped(if self.wheel_mode_hovered { 3 } else { 2 }),
            ..Cell::default()
        }
    }
}

impl Module for PaintCanvasBoundsModule {
    fn id(&self) -> &str {
        &self.id
    }

    fn rect(&self) -> ModuleRect {
        *self.viewport.borrow()
    }

    /// Tooltip hotspots: the module's gizmo bar plus the wheel-mode toggle
    /// on the border row.
    fn hotspots(&self) -> Vec<Hotspot> {
        let rect = self.rect();
        let mode = self.wheel_mode.borrow();
        let mode_description = match *mode {
            DrawingSpaceWheelMode::Pan => "the wheel scrolls the drawing space around",
            DrawingSpaceWheelMode::Depth => "the wheel steps through visible depth layers",
            DrawingSpaceWheelMode::Time => {
                "the wheel steps the playhead through time, wrapping at the loop window"
            }
        };
        drop(mode);
        let x = rect.x0 + self.wheel_mode_local_x();
        let y = rect.y0 + (rect.y1 - rect.y0 - 1);
        let custom = vec![
            title_hotspot(
                rect,
                self.title_start_x(),
                "drawing space module",
                "the interactable surface for drawing. hosts the toggle for scroll interactions just to the left of here so users can pan, traverse time, or 3d depth.",
            ),
            Hotspot::new(
            ModuleRect { x0: x, y0: y, x1: x, y1: y },
            "wheel mode",
            format!("{mode_description}; click to cycle pan/depth/time"),
        )];
        self.gizmos.hotspots_with(rect, custom)
    }

    fn draw(&self) -> CellGroup {
        let rect = self.rect();
        let origin = WorldPoint {
            x: rect.x0,
            y: rect.y0,
            z: 0,
        };
        let mut cells: Vec<Cell> = if self.gizmo_state.is_seamless() {
            Vec::new()
        } else {
            self.gizmo_state
                .decorate_panel_chrome(
                    PanelChrome::new(rect, &self.palette)
                        .with_title("DRAWING SPACE")
                        .with_title_start_x(self.title_start_x())
                        .without_background(),
                    &self.palette,
                )
                .cells()
        };
        if self.gizmo_state.should_draw_gizmo_bar() {
            cells.extend(self.gizmos.cells(rect, &self.gizmo_state, &self.palette));
            cells.push(self.wheel_mode_cell(rect));
        }

        CellGroup::from_cells(origin, cells).with_intake_behavior(CellGroupIntakeBehavior::Flat2d)
    }

    fn on_pointer_event(&mut self, event: ModulePointerEvent) {
        match event {
            ModulePointerEvent::Click { x, y, .. } => {
                let rect = self.rect();
                if Self::is_wheel_mode_hit(rect, x, y) {
                    self.cycle_wheel_mode();
                    return;
                }
                if let Some(outcome) = self.gizmo_state.handle_click(&self.gizmos, rect, x, y) {
                    if outcome == GizmoClickOutcome::Gizmo(GizmoKind::Close) {
                        self.hidden = true;
                    }
                }
            }
            ModulePointerEvent::Move { x, y } => {
                let rect = self.rect();
                self.gizmo_state.note_pointer(&self.gizmos, rect, x, y);
                self.wheel_mode_hovered = Self::is_wheel_mode_hit(rect, x, y);
                if let Some(next_rect) = self.gizmo_state.drag_rect(x, y) {
                    self.set_viewport(next_rect);
                }
            }
            ModulePointerEvent::Up { .. } => {
                self.gizmo_state.end_drag();
            }
            ModulePointerEvent::Enter => {
                self.gizmo_state.set_hovered(true);
            }
            ModulePointerEvent::Leave => {
                self.gizmo_state.set_hovered(false);
                self.wheel_mode_hovered = false;
            }
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
            self.rect(),
            self.gizmo_state.is_seamless(),
            self.hidden,
        ))
    }

    fn apply_persisted_ui_state(&mut self, state: &PersistedModuleUiState) {
        self.set_viewport(state.rect.to_runtime());
        self.gizmo_state.set_seamless(state.is_seamless);
        self.hidden = state.is_hidden;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use thaum_renderer_domain::{CellGraphic, ModulePointerButton, ModulePointerEvent};

    fn viewport() -> ModuleRect {
        ModuleRect {
            x0: 1,
            y0: 2,
            x1: 10,
            y1: 12,
        }
    }

    #[test]
    fn clicking_move_and_dragging_updates_the_shared_viewport() {
        let viewport = Rc::new(RefCell::new(viewport()));
        let wheel_mode = Rc::new(RefCell::new(DrawingSpaceWheelMode::Pan));
        let mut module = PaintCanvasBoundsModule::new(
            "canvas_bounds",
            viewport.clone(),
            wheel_mode,
            UiPalette::default(),
        );

        module.on_pointer_event(ModulePointerEvent::Click {
            x: 2,
            y: 11,
            button: ModulePointerButton::Left,
        });
        module.on_pointer_event(ModulePointerEvent::Move { x: 5, y: 16 });

        assert_eq!(
            *viewport.borrow(),
            ModuleRect {
                x0: 4,
                y0: 7,
                x1: 13,
                y1: 17,
            }
        );
    }

    #[test]
    fn resizing_updates_the_shared_viewport() {
        let viewport = Rc::new(RefCell::new(viewport()));
        let wheel_mode = Rc::new(RefCell::new(DrawingSpaceWheelMode::Pan));
        let mut module = PaintCanvasBoundsModule::new(
            "canvas_bounds",
            viewport.clone(),
            wheel_mode,
            UiPalette::default(),
        );

        module.on_pointer_event(ModulePointerEvent::Click {
            x: 6,
            y: 11,
            button: ModulePointerButton::Left,
        });
        module.on_pointer_event(ModulePointerEvent::Click {
            x: 10,
            y: 7,
            button: ModulePointerButton::Left,
        });
        module.on_pointer_event(ModulePointerEvent::Move { x: 6, y: 7 });

        assert_eq!(viewport.borrow().x1, 7);
    }

    #[test]
    fn clicking_the_close_gizmo_hides_the_module_and_set_hidden_reopens_it() {
        let viewport = Rc::new(RefCell::new(viewport()));
        let wheel_mode = Rc::new(RefCell::new(DrawingSpaceWheelMode::Pan));
        let mut module = PaintCanvasBoundsModule::new(
            "canvas_bounds",
            viewport,
            wheel_mode,
            UiPalette::default(),
        );
        // local_x == 3 is the standard bar's Close glyph.
        module.on_pointer_event(ModulePointerEvent::Click {
            x: 4,
            y: 11,
            button: ModulePointerButton::Left,
        });

        assert!(module.is_hidden());

        module.set_hidden(false);
        assert!(!module.is_hidden());
    }

    #[test]
    fn content_rect_reserves_the_border_and_header_rows() {
        assert_eq!(
            PaintCanvasBoundsModule::content_rect(viewport()),
            ModuleRect {
                x0: 2,
                y0: 3,
                x1: 8,
                y1: 9,
            }
        );
    }

    #[test]
    fn draws_the_standard_title_for_the_drawing_space() {
        let viewport = Rc::new(RefCell::new(ModuleRect {
            x0: 1,
            y0: 2,
            x1: 20,
            y1: 12,
        }));
        let wheel_mode = Rc::new(RefCell::new(DrawingSpaceWheelMode::Pan));
        let module = PaintCanvasBoundsModule::new(
            "canvas_bounds",
            viewport,
            wheel_mode,
            UiPalette::default(),
        );
        let drawn = module.draw();

        // The wheel-mode toggle sits right after the standard four gizmos,
        // and the title starts after it.
        assert!(drawn.iter_cells().any(|cell| cell.position.x == 9
            && cell.position.y == 9
            && cell.graphic == CellGraphic::Glyph('P')));
        assert!(drawn.iter_cells().any(|cell| cell.position.x == 11
            && cell.position.y == 9
            && cell.graphic == CellGraphic::Glyph('D')));
    }

    #[test]
    fn clicking_the_wheel_mode_gizmo_cycles_pan_and_depth() {
        let viewport = Rc::new(RefCell::new(viewport()));
        let wheel_mode = Rc::new(RefCell::new(DrawingSpaceWheelMode::Pan));
        let mut module = PaintCanvasBoundsModule::new(
            "canvas_bounds",
            viewport,
            wheel_mode.clone(),
            UiPalette::default(),
        );

        module.on_pointer_event(ModulePointerEvent::Click {
            x: 10,
            y: 11,
            button: ModulePointerButton::Left,
        });
        assert_eq!(*wheel_mode.borrow(), DrawingSpaceWheelMode::Depth);

        module.on_pointer_event(ModulePointerEvent::Click {
            x: 10,
            y: 11,
            button: ModulePointerButton::Left,
        });
        assert_eq!(*wheel_mode.borrow(), DrawingSpaceWheelMode::Time);

        module.on_pointer_event(ModulePointerEvent::Click {
            x: 10,
            y: 11,
            button: ModulePointerButton::Left,
        });
        assert_eq!(*wheel_mode.borrow(), DrawingSpaceWheelMode::Pan);
    }
}
