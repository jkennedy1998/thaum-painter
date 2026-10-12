use std::{cell::RefCell, rc::Rc};

use thaum_renderer_domain::{
    title_hotspot, Cell, CellColor, CellGraphic, CellGroup, CellGroupIntakeBehavior, CellPoint,
    CellWeight, GizmoBar, GizmoClickOutcome, GizmoKind, GizmoState, Hotspot, Module,
    ModulePointerButton, ModulePointerEvent, ModuleRect, PanelChrome, PersistedModuleUiState,
    UiColorRole, UiPalette, WorldPoint,
};

use crate::tool_state::{PaintHand, ToolState};
use crate::PaintColor;

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
    /// The drawing-space background (J): `Some(rgb)` fills the drawing space
    /// with that color, `None` is transparent — the app's own background
    /// shows through. Edited by the background widget, saved with the file.
    background: Option<Rc<RefCell<Option<[u8; 3]>>>>,
    /// Live hand state, read by the background widget: the pressing hand's
    /// color fills the background, the clear hand (blank) sets transparent.
    tool_state: Option<Rc<RefCell<ToolState>>>,
    background_hovered: bool,
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
            background: None,
            tool_state: None,
            background_hovered: false,
            hidden: false,
        }
    }

    /// Binds the drawing-space background handle (file truth) and the live
    /// tool state, enabling the background widget on the border row.
    pub fn with_background(
        mut self,
        background: Rc<RefCell<Option<[u8; 3]>>>,
        tool_state: Rc<RefCell<ToolState>>,
    ) -> Self {
        self.background = Some(background);
        self.tool_state = Some(tool_state);
        self
    }

    /// Where the background widget lives on the border row: the slot right
    /// of the wheel-mode toggle, with one border gap on each side before
    /// the DRAWING SPACE title (J) — displayed alongside the gizmos as a
    /// custom gizmo.
    fn background_local_x(&self) -> i32 {
        Self::background_local_x_for(self.gizmos.title_start_x())
    }

    fn background_local_x_for(gizmo_title_start_x: i32) -> i32 {
        // The wheel toggle sits at the gizmo bar's title slot; the widget
        // takes the next slot, and the title starts two past the widget.
        gizmo_title_start_x + 2
    }

    fn is_background_hit(rect: ModuleRect, x: i32, y: i32) -> bool {
        let local_x = x - rect.x0;
        let local_y = y - rect.y0;
        let height = rect.y1 - rect.y0;
        local_y == height - 1
            && local_x == Self::background_local_x_for(GizmoBar::standard().title_start_x())
    }

    /// Applies one hand to the drawing-space background (J): the hand's
    /// color fills it; the clear hand — the blank-graphic hand, painter's
    /// own clear semantics — sets the background transparent.
    fn apply_hand_to_background(&self, hand: PaintHand) {
        let (Some(background), Some(tool_state)) = (&self.background, &self.tool_state)
        else {
            return;
        };
        let state = tool_state.borrow();
        let hand_state = state.hand_state(hand);
        let next = match hand_state.graphic {
            CellGraphic::Glyph(' ') => None,
            _ => {
                let (r, g, b) = hand_state.color.preview_rgb();
                Some([r, g, b])
            }
        };
        if *background.borrow() != next {
            *background.borrow_mut() = next;
        }
    }

    /// The widget cell: the 'B' glyph in the gizmos' own color scheme —
    /// medium at rest, vivid under the pointer (J) — regardless of what the
    /// background itself is set to.
    fn background_widget_cell(&self, rect: ModuleRect) -> Cell {
        let hovered = self.background_hovered;
        Cell {
            position: CellPoint {
                x: self.background_local_x(),
                y: rect.y1 - rect.y0 - 1,
                z: 0,
            },
            graphic: CellGraphic::Glyph('B'),
            color: if hovered {
                self.palette.get(UiColorRole::Vivid)
            } else {
                self.palette.get(UiColorRole::Medium)
            },
            weight: CellWeight::from_index_clamped(if hovered { 3 } else { 2 }),
            ..Cell::default()
        }
    }

    /// The drawing-space backdrop (J): one screen-locked box behind the
    /// bounds of the drawing space module, filled with the drawing file's
    /// background color. Boot renders backdrop groups behind every depth
    /// plane, so the color sits behind all rendered characters regardless
    /// of depth — the window background itself stays the app's own UI
    /// background color.
    fn background_backdrop_group(&self) -> Option<CellGroup> {
        let background = self.background.as_ref()?;
        let rgb = (*background.borrow())?;
        let rect = self.rect();
        // J: pad the fill 2 cells in from every side, so the module's own
        // border plus one cell of UI background shows as a rim around the
        // inner edge before the file background starts.
        const BACKDROP_INSET: i32 = 2;
        let mut group = CellGroup::new(WorldPoint {
            x: rect.x0 + BACKDROP_INSET,
            y: rect.y0 + BACKDROP_INSET,
            z: 0,
        })
        .with_intake_behavior(CellGroupIntakeBehavior::Flat2d)
        .as_backdrop();
        // Cells are LOCAL to the group origin, matching PanelChrome's
        // convention: the origin already carries the rect offset, so
        // absolute positions would double it and the box would drift as
        // the panel is moved or resized (J).
        let width = (rect.x1 - rect.x0 - 2 * BACKDROP_INSET).max(0);
        let height = (rect.y1 - rect.y0 - 2 * BACKDROP_INSET).max(0);
        for y in 0..height {
            for x in 0..width {
                group.insert(Cell {
                    position: CellPoint { x, y, z: 0 },
                    graphic: CellGraphic::Glyph('\u{2588}'),
                    color: CellColor::Flat([
                        rgb[0] as f32 / 255.0,
                        rgb[1] as f32 / 255.0,
                        rgb[2] as f32 / 255.0,
                        1.0,
                    ]),
                    // J: render backdrop fill blocks at weight 3 so the box
                    // reads as a solid surface behind every depth plane.
                    weight: CellWeight::from_index_clamped(3),
                    ..Cell::default()
                });
            }
        }
        Some(group)
    }

    pub fn content_rect(viewport: ModuleRect) -> ModuleRect {
        PanelChrome::content_rect(viewport)
    }

    pub fn is_gizmo_hit(viewport: ModuleRect, x: i32, y: i32) -> bool {
        GizmoBar::standard().hit_test(viewport, x, y).is_some()
            || Self::is_wheel_mode_hit(viewport, x, y)
            || Self::is_background_hit(viewport, x, y)
    }

    fn wheel_mode_local_x(&self) -> i32 {
        self.gizmos.title_start_x()
    }

    fn title_start_x(&self) -> i32 {
        self.background_local_x() + 2
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
    fn backdrop_group(&self) -> Option<CellGroup> {
        self.background_backdrop_group()
    }
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
        let background_x = rect.x0 + self.background_local_x();
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
        ),
            Hotspot::new(
                ModuleRect {
                    x0: background_x,
                    y0: y,
                    x1: background_x,
                    y1: y,
                },
                "drawing background",
                "the drawing file's background color. press with a hand to fill it with that hand's color; the clear hand sets it transparent. saved with the drawing file.",
            ),
        ];
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
            cells.push(self.background_widget_cell(rect));
        }

        CellGroup::from_cells(origin, cells).with_intake_behavior(CellGroupIntakeBehavior::Flat2d)
    }

    fn on_pointer_event(&mut self, event: ModulePointerEvent) {
        match event {
            ModulePointerEvent::Click { x, y, button } => {
                let rect = self.rect();
                if Self::is_wheel_mode_hit(rect, x, y) {
                    self.cycle_wheel_mode();
                    return;
                }
                // A press on the background widget fills the drawing file's
                // background with the pressing hand's color; the clear hand
                // sets it transparent (J). Both buttons apply, each its own
                // hand, matching the toolbox's left/right convention.
                if Self::is_background_hit(rect, x, y) {
                    match button {
                        ModulePointerButton::Left => self.apply_hand_to_background(PaintHand::Left),
                        ModulePointerButton::Right => {
                            self.apply_hand_to_background(PaintHand::Right)
                        }
                        ModulePointerButton::Middle => {}
                    }
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
                self.background_hovered = Self::is_background_hit(rect, x, y);
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
                self.background_hovered = false;
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
        // the background widget takes the next slot, and the title starts
        // two past the widget (J).
        assert!(drawn.iter_cells().any(|cell| cell.position.x == 9
            && cell.position.y == 9
            && cell.graphic == CellGraphic::Glyph('P')));
        assert!(drawn.iter_cells().any(|cell| cell.position.x == 11
            && cell.position.y == 9
            && cell.graphic == CellGraphic::Glyph('B')));
        assert!(drawn.iter_cells().any(|cell| cell.position.x == 13
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

    fn background_fixture()
    -> (
        PaintCanvasBoundsModule,
        Rc<RefCell<Option<[u8; 3]>>>,
        Rc<RefCell<ToolState>>,
    ) {
        let background = Rc::new(RefCell::new(None));
        let state = Rc::new(RefCell::new(ToolState::default()));
        let module = PaintCanvasBoundsModule::new(
            "canvas_bounds",
            Rc::new(RefCell::new(viewport())),
            Rc::new(RefCell::new(DrawingSpaceWheelMode::Pan)),
            UiPalette::default(),
        )
        .with_background(background.clone(), state.clone());
        (module, background, state)
    }

    fn background_widget_screen_point() -> (i32, i32) {
        // The widget sits right of the wheel-mode toggle with a gap on each
        // side, before the DRAWING SPACE title (J); the border row is the
        // last row.
        let rect = viewport();
        (
            rect.x0 + PaintCanvasBoundsModule::background_local_x_for(
                GizmoBar::standard().title_start_x(),
            ),
            rect.y1 - 1,
        )
    }

    #[test]
    fn left_click_fills_the_background_with_the_left_hand_color() {
        let (mut module, background, state) = background_fixture();
        state.borrow_mut().left_hand.color = PaintColor::flat_rgb(250, 10, 20);
        let (x, y) = background_widget_screen_point();

        module.on_pointer_event(ModulePointerEvent::Click {
            x,
            y,
            button: ModulePointerButton::Left,
        });
        assert_eq!(*background.borrow(), Some([250, 10, 20]));

        // The widget itself stays in the gizmos' own color scheme: the 'B'
        // glyph in medium, never the background color (J).
        let group = module.draw();
        let cell = group
            .cells
            .get(&CellPoint { x: x - viewport().x0, y: 9, z: 0 })
            .expect("background widget cell");
        assert_eq!(cell.graphic, CellGraphic::Glyph('B'));
        assert_eq!(cell.color, UiPalette::default().get(UiColorRole::Medium));
    }

    #[test]
    fn hovering_the_background_widget_vivids_it_like_other_gizmos() {
        let (mut module, _background, _state) = background_fixture();
        let (x, y) = background_widget_screen_point();
        module.on_pointer_event(ModulePointerEvent::Move { x, y });

        let group = module.draw();
        let cell = group
            .cells
            .get(&CellPoint { x: x - viewport().x0, y: 9, z: 0 })
            .expect("background widget cell");
        assert_eq!(cell.color, UiPalette::default().get(UiColorRole::Vivid));
    }

    #[test]
    fn the_backdrop_group_fills_the_module_rect_with_the_file_background() {
        let (module, background, _state) = background_fixture();
        assert!(module.backdrop_group().is_none(), "no background, no box");

        *background.borrow_mut() = Some([12, 34, 56]);
        let group = module.backdrop_group().expect("backdrop group");
        assert_eq!(group.intake_behavior, CellGroupIntakeBehavior::Flat2d);
        assert!(group.backdrop, "must stage as a backdrop");

        let rect = viewport();
        // J: the fill is padded 2 cells in from every side.
        const INSET: i32 = 2;
        let width = rect.x1 - rect.x0 - 2 * INSET;
        let height = rect.y1 - rect.y0 - 2 * INSET;
        assert_eq!(group.cells.len(), (width * height) as usize);
        // Cells are local to the group origin (the padded rect corner), so
        // the staged world coverage is origin + local = exactly the padded
        // rect — never a doubled offset (J).
        let corner = group
            .cells
            .get(&CellPoint { x: 0, y: 0, z: 0 })
            .expect("backdrop corner cell");
        assert_eq!(corner.graphic, CellGraphic::Glyph('\u{2588}'));
        assert_eq!(
            corner.color,
            CellColor::Flat([12.0 / 255.0, 34.0 / 255.0, 56.0 / 255.0, 1.0])
        );
        let world = group.world_point_for(corner.position);
        assert_eq!(
            (world.x, world.y),
            (rect.x0 + INSET, rect.y0 + INSET),
            "origin + local == padded rect corner"
        );
        // The rim is real: one cell past the padded edge on every side is
        // uncovered UI background.
        assert!(group
            .world_point_for(CellPoint { x: width - 1, y: height - 1, z: 0 })
            == WorldPoint {
                x: rect.x1 - 1 - INSET,
                y: rect.y1 - 1 - INSET,
                z: 0,
            });
    }

    #[test]
    fn moving_the_panel_moves_the_backdrop_box_with_it() {
        let (module, background, _state) = background_fixture();
        *background.borrow_mut() = Some([12, 34, 56]);
        let rect = viewport();
        let group = module.backdrop_group().expect("backdrop group");
        let width = rect.x1 - rect.x0;
        let height = rect.y1 - rect.y0;

        // Drag the panel by (+3, +4), like set_viewport does.
        let moved = ModuleRect {
            x0: rect.x0 + 3,
            y0: rect.y0 + 4,
            x1: rect.x1 + 3,
            y1: rect.y1 + 4,
        };
        let moved_module = PaintCanvasBoundsModule::new(
            "canvas_bounds",
            Rc::new(RefCell::new(moved)),
            Rc::new(RefCell::new(DrawingSpaceWheelMode::Pan)),
            UiPalette::default(),
        )
        .with_background(background.clone(), _state.clone());
        let moved_group = moved_module.backdrop_group().expect("backdrop group");

        // Same local shape, origin shifted exactly by the drag delta.
        assert_eq!(moved_group.cells.len(), group.cells.len());
        let width = rect.x1 - rect.x0 - 4;
        let height = rect.y1 - rect.y0 - 4;
        let world = moved_group
            .world_point_for(CellPoint { x: width - 1, y: height - 1, z: 0 });
        assert_eq!(
            (world.x, world.y),
            (moved.x1 - 1 - 2, moved.y1 - 1 - 2),
            "padded far corner tracks the moved rect"
        );
    }

    #[test]
    fn right_click_with_the_clear_hand_sets_the_background_transparent() {
        let (mut module, background, _state) = background_fixture();
        // The default right hand holds the blank: painter's clear hand.
        *background.borrow_mut() = Some([1, 2, 3]);
        let (x, y) = background_widget_screen_point();

        module.on_pointer_event(ModulePointerEvent::Click {
            x,
            y,
            button: ModulePointerButton::Right,
        });
        assert_eq!(*background.borrow(), None, "clear hand means transparent");

        // Transparent keeps the widget in the gizmo scheme: the 'B' glyph
        // in medium.
        let group = module.draw();
        let cell = group
            .cells
            .get(&CellPoint { x: x - viewport().x0, y: 9, z: 0 })
            .expect("background widget cell");
        assert_eq!(cell.graphic, CellGraphic::Glyph('B'));
        assert_eq!(cell.color, UiPalette::default().get(UiColorRole::Medium));
    }

    #[test]
    fn right_click_with_a_content_hand_fills_with_the_right_hand_color() {
        let (mut module, background, state) = background_fixture();
        {
            let mut state = state.borrow_mut();
            state.right_hand.graphic = CellGraphic::Glyph('#');
            state.right_hand.color = PaintColor::flat_rgb(9, 8, 7);
        }
        let (x, y) = background_widget_screen_point();

        module.on_pointer_event(ModulePointerEvent::Click {
            x,
            y,
            button: ModulePointerButton::Right,
        });
        assert_eq!(*background.borrow(), Some([9, 8, 7]));
    }

    #[test]
    fn the_background_widget_is_a_gizmo_hit_and_carries_its_tooltip() {
        let (module, _background, _state) = background_fixture();
        let (x, y) = background_widget_screen_point();
        assert!(PaintCanvasBoundsModule::is_gizmo_hit(viewport(), x, y));

        let hotspot = module
            .hotspots()
            .into_iter()
            .find(|hotspot| hotspot.title == "drawing background")
            .expect("background widget tooltip");
        assert!(hotspot.description.contains("transparent"));
    }
}
