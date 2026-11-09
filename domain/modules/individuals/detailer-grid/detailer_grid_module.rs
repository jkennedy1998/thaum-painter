use std::{cell::RefCell, rc::Rc};

use thaum_renderer_domain::{
    title_hotspot, Cell, CellGraphic, CellGroup, CellGroupIntakeBehavior, CellPoint, CellWeight,
    GizmoBar, GizmoClickOutcome, GizmoKind, GizmoState, Hotspot, Module, ModulePointerButton,
    ModulePointerEvent, ModuleRect, PanelChrome, PersistedModuleUiState, UiColorRole, UiPalette,
    WorldPoint,
};

use crate::detailer_stroke::{DetailerSession, DetailerStroke};
use crate::tool_state::PaintHand;

/// Each comparison pixel becomes this many UI cells in each direction. The
/// source comparison tile remains its typeface-derived size; this only makes
/// its alpha values legible to a person drawing into it.
const GRID_SCALE: i32 = 2;
const GRID_BRUSH_STRENGTH: u8 = u8::MAX;

type ResolveDetailerGlyph = dyn Fn(&DetailerStroke) -> Option<char>;

/// Painter-owned enlarged view over the active detailer's comparison tile.
/// It owns subcell pointer mapping, but leaves stroke data and release writes
/// to the shared detail session and canvas-pointer respectively.
pub struct DetailerGridModule {
    id: String,
    rect: ModuleRect,
    session: Rc<RefCell<DetailerSession>>,
    resolve: Rc<ResolveDetailerGlyph>,
    palette: UiPalette,
    gizmos: GizmoBar,
    gizmo_state: GizmoState,
    grid_dragging: bool,
    hidden: bool,
}

impl DetailerGridModule {
    pub fn new(
        id: impl Into<String>,
        rect: ModuleRect,
        session: Rc<RefCell<DetailerSession>>,
        resolve: Rc<ResolveDetailerGlyph>,
    ) -> Self {
        Self {
            id: id.into(),
            rect,
            session,
            resolve,
            palette: UiPalette::default(),
            gizmos: GizmoBar::standard(),
            gizmo_state: GizmoState::new(),
            grid_dragging: false,
            hidden: false,
        }
    }

    pub fn with_palette(mut self, palette: UiPalette) -> Self {
        self.palette = palette;
        self
    }

    /// The local top-left of a comparison pixel. Flat2d local y grows toward
    /// screen-top, so pixel rows walk down by subtracting from this edge.
    fn grid_top_left(&self) -> (i32, i32) {
        let (content_x, content_y) = PanelChrome::content_origin();
        let (_, content_height) = PanelChrome::content_size(self.rect);
        (content_x, content_y + content_height - 1)
    }

    /// Maps a module-space pointer to one typeface-derived comparison pixel.
    /// The tile itself is authoritative: out-of-grid panel space never edits.
    fn pixel_at(&self, screen_x: i32, screen_y: i32) -> Option<(usize, usize)> {
        let tile = self.session.borrow().active_tile()?;
        let (left, top) = self.grid_top_left();
        let local_x = screen_x - self.rect.x0;
        let local_y = screen_y - self.rect.y0;
        if local_x < left || local_y > top {
            return None;
        }
        let x = ((local_x - left) / GRID_SCALE) as usize;
        let y = ((top - local_y) / GRID_SCALE) as usize;
        (x < tile.width && y < tile.height).then_some((x, y))
    }

    fn draw_tile_cells(&self, cells: &mut Vec<Cell>) {
        let Some(tile) = self.session.borrow().active_tile() else {
            return;
        };
        let (left, top) = self.grid_top_left();
        for y in 0..tile.height {
            for x in 0..tile.width {
                let (glyph, role, weight) = match tile.alpha[y * tile.width + x] {
                    0 => ('.', UiColorRole::Medium, 0),
                    1..=85 => (':', UiColorRole::Medium, 1),
                    86..=170 => ('o', UiColorRole::Bright, 2),
                    _ => ('#', UiColorRole::Vivid, 3),
                };
                for dy in 0..GRID_SCALE {
                    for dx in 0..GRID_SCALE {
                        cells.push(Cell {
                            position: CellPoint {
                                x: left + x as i32 * GRID_SCALE + dx,
                                y: top - y as i32 * GRID_SCALE - dy,
                                z: 0,
                            },
                            graphic: CellGraphic::Glyph(glyph),
                            color: self.palette.get(role),
                            weight: CellWeight::from_index_clamped(weight),
                            ..Cell::default()
                        });
                    }
                }
            }
        }
    }

    fn paint_at(&mut self, screen_x: i32, screen_y: i32) -> bool {
        let Some((x, y)) = self.pixel_at(screen_x, screen_y) else {
            return false;
        };
        let resolve = &self.resolve;
        self.session
            .borrow_mut()
            .paint(x, y, GRID_BRUSH_STRENGTH, |stroke| resolve(stroke));
        true
    }
}

impl Module for DetailerGridModule {
    fn id(&self) -> &str {
        &self.id
    }

    fn rect(&self) -> ModuleRect {
        self.rect
    }

    fn hotspots(&self) -> Vec<Hotspot> {
        let mut custom = vec![title_hotspot(
            self.rect,
            self.gizmos.title_start_x(),
            "detail grid module",
            "draw into the enlarged glyph-shape grid; the canvas previews the nearest character before release.",
        )];
        if self.session.borrow().is_active() {
            let (left, top) = self.grid_top_left();
            let tile = self
                .session
                .borrow()
                .active_tile()
                .expect("active session has a tile");
            custom.push(Hotspot::new(
                ModuleRect {
                    x0: self.rect.x0 + left,
                    y0: self.rect.y0 + top - tile.height as i32 * GRID_SCALE + 1,
                    x1: self.rect.x0 + left + tile.width as i32 * GRID_SCALE - 1,
                    y1: self.rect.y0 + top,
                },
                "detail pixels",
                "drag to add or erase subcell ink; release requests the displayed glyph only.",
            ));
        }
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
                        .with_title("DETAIL")
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
        self.draw_tile_cells(&mut cells);
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
                if self.session.borrow().active_hand() != Some(hand) {
                    return;
                }
                self.grid_dragging = self.paint_at(x, y);
            }
            ModulePointerEvent::Move { x, y } => {
                if self.grid_dragging {
                    self.paint_at(x, y);
                    return;
                }
                self.gizmo_state.note_pointer(&self.gizmos, self.rect, x, y);
                if let Some(next_rect) = self.gizmo_state.drag_rect(x, y) {
                    self.rect = next_rect;
                }
            }
            ModulePointerEvent::Up { .. } => {
                if self.grid_dragging {
                    self.session.borrow_mut().finish();
                    self.grid_dragging = false;
                }
                self.gizmo_state.end_drag();
            }
            ModulePointerEvent::Enter => self.gizmo_state.set_hovered(true),
            ModulePointerEvent::Leave => self.gizmo_state.set_hovered(false),
            ModulePointerEvent::Down { .. } => {}
        }
    }

    fn wants_pointer_capture(&self) -> bool {
        self.grid_dragging || self.gizmo_state.wants_pointer_capture()
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
    use crate::detailer_stroke::{DetailerMode, DetailerStroke};

    fn rect() -> ModuleRect {
        ModuleRect {
            x0: 10,
            y0: 20,
            x1: 29,
            y1: 39,
        }
    }

    fn session_with_active_stroke() -> Rc<RefCell<DetailerSession>> {
        let session = Rc::new(RefCell::new(DetailerSession::default()));
        session.borrow_mut().begin(
            DetailerStroke::begin(
                PaintHand::Left,
                CellPoint { x: 4, y: 5, z: 0 },
                DetailerMode::Additive,
                CellWeight::TWO,
                thaum_renderer_domain::GlyphTileRaster {
                    width: 3,
                    height: 2,
                    alpha: vec![0; 6],
                },
            ),
            |_| Some('.'),
        );
        session
    }

    fn module(session: Rc<RefCell<DetailerSession>>) -> DetailerGridModule {
        DetailerGridModule::new(
            "detailer_grid",
            rect(),
            session,
            Rc::new(|stroke| (stroke.tile().alpha[0] == 255).then_some('#').or(Some('.'))),
        )
    }

    #[test]
    fn pointer_mapping_scales_the_live_comparison_tile_without_fixed_dimensions() {
        let module = module(session_with_active_stroke());
        let (left, top) = module.grid_top_left();
        assert_eq!(
            module.pixel_at(rect().x0 + left, rect().y0 + top),
            Some((0, 0))
        );
        assert_eq!(
            module.pixel_at(rect().x0 + left + 5, rect().y0 + top - 3),
            Some((2, 1))
        );
        assert_eq!(module.pixel_at(rect().x0 + left + 6, rect().y0 + top), None);
        assert_eq!(module.pixel_at(rect().x0 + left, rect().y0 + top - 4), None);
    }

    #[test]
    fn grid_drag_updates_the_pending_glyph_then_releases_one_commit() {
        let session = session_with_active_stroke();
        let mut module = module(session.clone());
        let (left, top) = module.grid_top_left();
        let x = rect().x0 + left;
        let y = rect().y0 + top;

        module.on_pointer_event(ModulePointerEvent::Click {
            x,
            y,
            button: ModulePointerButton::Left,
        });
        assert!(module.wants_pointer_capture());
        assert_eq!(session.borrow().pending_graphic(), Some('#'));

        module.on_pointer_event(ModulePointerEvent::Up { x, y });
        assert!(!module.wants_pointer_capture());
        assert_eq!(
            session.borrow_mut().take_pending_commit(),
            Some(crate::detailer_stroke::DetailerCommit {
                position: CellPoint { x: 4, y: 5, z: 0 },
                graphic: '#',
            })
        );
    }

    #[test]
    fn wrong_hand_and_outside_grid_never_paint_or_capture() {
        let session = session_with_active_stroke();
        let mut module = module(session.clone());
        let (left, top) = module.grid_top_left();
        module.on_pointer_event(ModulePointerEvent::Click {
            x: rect().x0 + left,
            y: rect().y0 + top,
            button: ModulePointerButton::Right,
        });
        module.on_pointer_event(ModulePointerEvent::Click {
            x: rect().x0 + left + 12,
            y: rect().y0 + top,
            button: ModulePointerButton::Left,
        });

        assert!(!module.wants_pointer_capture());
        assert_eq!(session.borrow().active_tile().unwrap().alpha, vec![0; 6]);
    }

    #[test]
    fn no_target_draws_no_grid_and_ignores_pointer_events() {
        let session = Rc::new(RefCell::new(DetailerSession::default()));
        let mut module = module(session);
        assert!(module.pixel_at(12, 38).is_none());
        module.on_pointer_event(ModulePointerEvent::Click {
            x: 12,
            y: 38,
            button: ModulePointerButton::Left,
        });
        assert!(!module.wants_pointer_capture());
    }
}
