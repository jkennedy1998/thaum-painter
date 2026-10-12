use std::{cell::RefCell, rc::Rc};

use thaum_renderer_domain::{
    title_hotspot, Cell, CellGraphic, CellGroup, CellGroupIntakeBehavior, CellPoint, CellWeight,
    GizmoBar, GizmoClickOutcome, GizmoKind, GizmoState, Hotspot, Module, ModulePointerButton,
    ModulePointerEvent, ModuleRect, PanelChrome, PersistedModuleUiState, ScrollState, Scrollbar,
    ScrollbarSpan, UiColorRole, UiPalette, WorldPoint,
};

use crate::tool_state::{PaintHand, PaintTool, ToolState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolDef {
    pub tool: PaintTool,
    pub icon: char,
    pub label: &'static str,
    /// Toolbox tooltip copy: what the tool does.
    pub description: &'static str,
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
    /// Row currently under the pointer, for hover coloring.
    hovered_tool: Option<PaintTool>,
    /// Tool-row scroll offset: the list crops into the visible band and
    /// scrolls once the panel is shorter than the tool list.
    scroll: ScrollState,
    /// Vertical scrollbar over the tool rows (shared scrollbar seam),
    /// drawn only when the tool list overflows the panel.
    scrollbar: Scrollbar,
}

impl ToolboxModule {
    pub fn new(
        id: impl Into<String>,
        rect: ModuleRect,
        tool_state: Rc<RefCell<ToolState>>,
        tool_defs: Vec<ToolDef>,
    ) -> Self {
        let mut toolbox = Self {
            id: id.into(),
            rect,
            tool_state,
            tool_defs,
            palette: UiPalette::default(),
            gizmos: GizmoBar::standard(),
            gizmo_state: GizmoState::new(),
            hidden: false,
            hovered_tool: None,
            scroll: ScrollState::new(),
            scrollbar: Scrollbar::new(ScrollbarSpan::Vertical {
                x: 0,
                top: 0,
                bottom: 0,
            }),
        };
        // Interactions run against the bar's span, so seed it honestly;
        // draw re-derives fresh each frame via cells_with_span.
        toolbox.scrollbar.set_span(toolbox.scrollbar_span());
        toolbox
    }

    pub fn with_palette(mut self, palette: UiPalette) -> Self {
        self.palette = palette;
        self
    }

    /// Screen-top row y for one visible slot (larger local y renders
    /// higher): the content's top line, walking downward per slot.
    fn row_y(&self, slot: usize) -> i32 {
        let (_, content_height) = PanelChrome::content_size(self.rect);
        let (_, content_y) = PanelChrome::content_origin();
        content_y + (content_height - 1).max(0) - slot as i32
    }

    /// Tool rows visible at once: the whole content band. When the tool
    /// list is longer the rows crop here and scroll under the scrollbar.
    fn visible_row_count(&self) -> usize {
        let (_, content_height) = PanelChrome::content_size(self.rect);
        content_height.max(0) as usize
    }

    /// Largest valid tool-row offset.
    fn max_scroll_rows(&self) -> usize {
        ScrollState::max_offset(self.tool_defs.len(), self.visible_row_count())
    }

    /// Where the vertical scrollbar lives this frame: the rightmost content
    /// column spanning the whole tool-row band — the graphic picker's band
    /// shape, full height, degenerate-height safe. Derived fresh each use
    /// so resizes and boot renders stay honest.
    fn scrollbar_span(&self) -> ScrollbarSpan {
        let (content_x, content_y) = PanelChrome::content_origin();
        let (content_width, content_height) = PanelChrome::content_size(self.rect);
        let top = content_y + (content_height - 1).max(0);
        let bottom = (top - self.visible_row_count() as i32 + 1).max(content_y);
        ScrollbarSpan::Vertical {
            x: content_x + content_width - 1,
            top,
            bottom,
        }
    }

    /// Press on the bar: thumb grab or track page-jump. Returns whether the
    /// bar consumed the press.
    fn scrollbar_press(&mut self, local_x: i32, local_y: i32) -> bool {
        let content = self.tool_defs.len();
        let viewport = self.visible_row_count();
        let max = ScrollState::max_offset(content, viewport);
        let mut offset = self.scroll.offset();
        let consumed = self.scrollbar.on_down(local_x, local_y, content, viewport, &mut offset);
        self.scroll.set_offset(offset, max);
        consumed
    }

    /// Pointer move during a live bar drag: re-map the grab point.
    fn scrollbar_drag(&mut self, local_x: i32, local_y: i32) -> bool {
        let content = self.tool_defs.len();
        let viewport = self.visible_row_count();
        let max = ScrollState::max_offset(content, viewport);
        let mut offset = self.scroll.offset();
        let moved = self.scrollbar.on_drag(local_x, local_y, content, viewport, &mut offset);
        self.scroll.set_offset(offset, max);
        moved
    }

    /// Whether the scrollbar shows this frame: more tools than rows.
    fn scrollbar_visible(&self) -> bool {
        self.tool_defs.len() > self.visible_row_count()
    }

    /// Absolute screen-space rect of one tool row: the panel's usable
    /// content span on that row, matching `tool_at` hit-testing.
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

    fn tool_at(&self, x: i32, y: i32) -> Option<PaintTool> {
        let local_x = x - self.rect.x0;
        let local_y = y - self.rect.y0;
        let (content_x, _) = PanelChrome::content_origin();
        let (content_width, _) = PanelChrome::content_size(self.rect);
        if local_x < content_x || local_x >= content_x + content_width {
            return None;
        }
        let top_row_y = self.row_y(0);
        if local_y > top_row_y {
            return None;
        }
        let slot = (top_row_y - local_y) as usize;
        if slot >= self.visible_row_count() {
            return None;
        }
        let index = self.scroll.offset() + slot;
        self.tool_defs.get(index).map(|def| def.tool)
    }
}

impl Module for ToolboxModule {
    fn id(&self) -> &str {
        &self.id
    }

    fn rect(&self) -> ModuleRect {
        self.rect
    }

    /// Tooltip hotspots: the module's title (what this panel is) plus one
    /// row hotspot per tool, so hovering a tool explains it ("icon Name" +
    /// what it does) through the shared tooltip implementation.
    fn hotspots(&self) -> Vec<Hotspot> {
        let mut hotspots = self.gizmos.hotspots_with(
            self.rect,
            vec![title_hotspot(
                self.rect,
                self.gizmos.title_start_x(),
                "toolbox module",
                "picks the tool for your given hand. left click for left hand, right click for right hand.",
            )],
        );
        hotspots.extend(
            self.tool_defs
                .iter()
                .enumerate()
                .skip(self.scroll.offset())
                .take(self.visible_row_count())
                .map(|(index, def)| {
                    Hotspot::new(
                        self.row_rect(index),
                        format!("{} {}", def.icon, def.label),
                        format!(
                            "{} left click for left hand, right click for right hand",
                            def.description
                        ),
                    )
                }),
        );
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

        // Labels crop one column short of the bar when it shows (J's
        // bar-aware rule from the graphic picker): no text under the bar.
        let (content_x, _) = PanelChrome::content_origin();
        let (content_width, _) = PanelChrome::content_size(self.rect);
        let label_max_x = content_x + content_width - 1
            - if self.scrollbar_visible() { 1 } else { 0 };
        for slot in 0..self.visible_row_count() {
            let index = self.scroll.offset() + slot;
            let Some(def) = self.tool_defs.get(index) else {
                break;
            };
            let y = self.row_y(slot);
            let is_left = state.left_tool == def.tool;
            let is_right = state.right_tool == def.tool;
            let is_selected = is_left || is_right;
            let is_hovered = self.hovered_tool == Some(def.tool);
            let weight = if is_selected {
                CellWeight::Three
            } else {
                CellWeight::Two
            };
            let indicator = if is_left && is_right {
                ('◆', self.palette.get(UiColorRole::Vivid))
            } else if is_left {
                ('L', self.palette.get(UiColorRole::LeftHand))
            } else if is_right {
                ('R', self.palette.get(UiColorRole::RightHand))
            } else {
                (' ', self.palette.get(UiColorRole::Medium))
            };
            // Per-hand selection drives the row color; hover brightens
            // unselected rows through the shared palette roles.
            let text_color = if is_left && is_right {
                self.palette.get(UiColorRole::Vivid)
            } else if is_left {
                self.palette.get(UiColorRole::LeftHand)
            } else if is_right {
                self.palette.get(UiColorRole::RightHand)
            } else if is_hovered {
                self.palette.get(UiColorRole::Bright)
            } else {
                self.palette.get(UiColorRole::Medium)
            };
            let icon_color = text_color;

            cells.push(Cell {
                position: CellPoint { x: 1, y, z: 0 },
                graphic: CellGraphic::Glyph(indicator.0),
                color: indicator.1,
                weight,
                ..Cell::default()
            });
            cells.push(Cell {
                position: CellPoint { x: 3, y, z: 0 },
                graphic: CellGraphic::Glyph(def.icon),
                color: icon_color,
                weight,
                ..Cell::default()
            });
            for (column, glyph) in def.label.chars().enumerate() {
                let x = 5 + column as i32;
                if x > label_max_x {
                    break;
                }
                cells.push(Cell {
                    position: CellPoint { x, y, z: 0 },
                    graphic: CellGraphic::Glyph(glyph),
                    color: text_color,
                    weight,
                    ..Cell::default()
                });
            }
        }

        // Vertical scrollbar over the tool rows, drawn last so the
        // thumb/track overwrite any row cell they sit on. Only renders when
        // the tool list overflows the panel. The span derives fresh each
        // frame so a boot/restore render is never a stale leftover bar.
        cells.extend(self.scrollbar.cells_with_span(
            self.scrollbar_span(),
            self.tool_defs.len(),
            self.visible_row_count(),
            self.scroll.offset(),
            &self.palette,
            None,
            self.scrollbar.is_active(),
        ));

        CellGroup::from_cells(origin, cells).with_intake_behavior(CellGroupIntakeBehavior::Flat2d)
    }

    fn on_wheel(&mut self, _x: i32, _y: i32, _delta_x: f32, delta_y: f32) -> bool {
        // Only consume the wheel when the tool list actually overflows:
        // a fully visible list falls through so viewport camera behavior
        // keeps working.
        if self.max_scroll_rows() == 0 {
            return false;
        }
        self.scrollbar.set_span(self.scrollbar_span());
        self.scroll.wheel(delta_y, self.max_scroll_rows());
        true
    }

    fn on_pointer_event(&mut self, event: ModulePointerEvent) {
        self.scrollbar.set_span(self.scrollbar_span());
        match event {
            ModulePointerEvent::Click { x, y, button } => {
                if let Some(outcome) = self.gizmo_state.handle_click(&self.gizmos, self.rect, x, y)
                {
                    if outcome == GizmoClickOutcome::Gizmo(GizmoKind::Close) {
                        self.hidden = true;
                    }
                    return;
                }
                // A left press on the scrollbar is bar interaction (thumb
                // grab or track page-jump), never a tool assignment.
                if button == ModulePointerButton::Left
                    && self.scrollbar_press(x - self.rect.x0, y - self.rect.y0)
                {
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
                self.scrollbar.note_pointer(x - self.rect.x0, y - self.rect.y0);
                // A live bar drag owns the pointer: follow the thumb, skip
                // the gizmo drag path entirely.
                if self.scrollbar.is_dragging() {
                    self.scrollbar_drag(x - self.rect.x0, y - self.rect.y0);
                    return;
                }
                self.gizmo_state.note_pointer(&self.gizmos, self.rect, x, y);
                if self.gizmo_state.drag_rect(x, y).is_some() {
                    self.hovered_tool = None;
                } else {
                    self.hovered_tool = self.tool_at(x, y);
                }
                if let Some(next_rect) = self.gizmo_state.drag_rect(x, y) {
                    self.rect = next_rect;
                }
            }
            ModulePointerEvent::Up { .. } => {
                self.scrollbar.on_up();
                self.gizmo_state.end_drag();
            }
            ModulePointerEvent::Enter => self.gizmo_state.set_hovered(true),
            ModulePointerEvent::Leave => {
                self.scrollbar.clear_pointer();
                self.gizmo_state.set_hovered(false);
                self.hovered_tool = None;
            }
            ModulePointerEvent::Down { .. } => {}
        }
    }

    fn wants_pointer_capture(&self) -> bool {
        self.gizmo_state.wants_pointer_capture() || self.scrollbar.is_dragging()
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
        // A restored rect may be shorter than the tool list; keep any
        // restored scroll offset valid for it.
        let max = self.max_scroll_rows();
        if self.scroll.offset() > max {
            self.scroll.to_end(max);
        }
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
                description: "paints cells",
            },
            ToolDef {
                tool: PaintTool::Fill,
                icon: 'F',
                label: "Fill",
                description: "flood-fills cells",
            },
            ToolDef {
                tool: PaintTool::Lasso,
                icon: 'L',
                label: "Lasso",
                description: "fills enclosed cells",
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
        assert_eq!(state.left_tool, PaintTool::Fill);
        assert_eq!(state.right_tool, PaintTool::Brush);
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
        assert_eq!(state.right_tool, PaintTool::Lasso);
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
            group.cells[&CellPoint { x: 1, y: 2, z: 0 }].graphic,
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
        assert_eq!(state.right_tool, PaintTool::Brush);
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
            group.cells[&CellPoint { x: 1, y: 2, z: 0 }].graphic,
            CellGraphic::Glyph('◆')
        );
    }

    #[test]
    fn unselected_rows_draw_weight_two_and_selected_rows_weight_three() {
        let state = tool_state();
        {
            let mut state = state.borrow_mut();
            state.set_tool_for_hand(PaintHand::Left, PaintTool::Brush);
            state.set_tool_for_hand(PaintHand::Right, PaintTool::Fill);
        }
        let toolbox = ToolboxModule::new("toolbox", rect(), state, tool_defs());

        let group = toolbox.draw();

        // Brush (selected by left) and Fill (selected by right) are weight
        // three; Lasso is unselected and stays at the normal weight two.
        assert_eq!(
            group.cells[&CellPoint { x: 5, y: 3, z: 0 }].weight,
            CellWeight::Three
        );
        assert_eq!(
            group.cells[&CellPoint { x: 5, y: 2, z: 0 }].weight,
            CellWeight::Three
        );
        assert_eq!(
            group.cells[&CellPoint { x: 5, y: 1, z: 0 }].weight,
            CellWeight::Two
        );
    }

    #[test]
    fn hovering_an_unselected_row_brightens_it_through_the_shared_palette() {
        let state = tool_state();
        state
            .borrow_mut()
            .set_tool_for_hand(PaintHand::Right, PaintTool::Fill);
        let mut toolbox = ToolboxModule::new("toolbox", rect(), state, tool_defs());

        let bright = toolbox.palette.get(UiColorRole::Bright);
        let left_hand = toolbox.palette.get(UiColorRole::LeftHand);
        toolbox.on_pointer_event(ModulePointerEvent::Move { x: 5, y: 1 });

        let group = toolbox.draw();

        // Lasso (y=1, unselected) brightens on hover; Brush (y=3,
        // left-selected) keeps its hand color.
        assert_eq!(group.cells[&CellPoint { x: 5, y: 1, z: 0 }].color, bright);
        assert_eq!(
            group.cells[&CellPoint { x: 5, y: 3, z: 0 }].color,
            left_hand
        );
    }

    #[test]
    fn leaving_the_panel_clears_the_hover_highlight() {
        let state = tool_state();
        let mut toolbox = ToolboxModule::new("toolbox", rect(), state, tool_defs());
        let medium = toolbox.palette.get(UiColorRole::Medium);

        toolbox.on_pointer_event(ModulePointerEvent::Move { x: 5, y: 1 });
        toolbox.on_pointer_event(ModulePointerEvent::Leave);

        let group = toolbox.draw();
        assert_eq!(group.cells[&CellPoint { x: 5, y: 1, z: 0 }].color, medium);
    }

    #[test]
    fn hotspots_frame_each_tool_row_with_icon_name_and_description() {
        let toolbox = ToolboxModule::new("toolbox", rect(), tool_state(), tool_defs());

        let hotspots = toolbox.hotspots();

        for (index, def) in tool_defs().iter().enumerate() {
            let expected_rect = ModuleRect {
                x0: 1,
                y0: toolbox.row_y(index),
                x1: 16,
                y1: toolbox.row_y(index),
            };
            let hotspot = hotspots
                .iter()
                .find(|hotspot| hotspot.rect == expected_rect)
                .unwrap_or_else(|| panic!("no hotspot for {}", def.label));
            assert_eq!(hotspot.title, format!("{} {}", def.icon, def.label));
            assert_eq!(
                hotspot.description,
                format!(
                    "{} left click for left hand, right click for right hand",
                    def.description
                )
            );
        }
    }

    fn overflowing_tool_defs() -> Vec<ToolDef> {
        vec![
            ToolDef {
                tool: PaintTool::Brush,
                icon: 'B',
                label: "Brush",
                description: "paints cells",
            },
            ToolDef {
                tool: PaintTool::Fill,
                icon: 'F',
                label: "Fill",
                description: "flood-fills cells",
            },
            ToolDef {
                tool: PaintTool::Lasso,
                icon: 'L',
                label: "Lasso",
                description: "fills enclosed cells",
            },
            ToolDef {
                tool: PaintTool::Text,
                icon: 'T',
                label: "Text",
                description: "types cells",
            },
            ToolDef {
                tool: PaintTool::Picker,
                icon: 'P',
                label: "Pick",
                description: "samples a cell",
            },
            ToolDef {
                tool: PaintTool::Stamp,
                icon: 'S',
                label: "LongerLabelTool",
                description: "places copied cells",
            },
        ]
    }

    #[test]
    fn overflowing_tools_draw_a_full_height_scrollbar_without_any_interaction() {
        // Six defs over a 3-row viewport: the bar shows straight out of the
        // box and its track spans the whole tool-row band.
        let toolbox = ToolboxModule::new("toolbox", rect(), tool_state(), overflowing_tool_defs());
        assert!(toolbox.scrollbar_visible());
        let group = toolbox.draw();
        let (bar_x, top, bottom) = match toolbox.scrollbar_span() {
            ScrollbarSpan::Vertical { x, top, bottom } => (x, top, bottom),
            ScrollbarSpan::Horizontal { .. } => unreachable!(),
        };
        assert_eq!((top - bottom + 1) as usize, toolbox.visible_row_count());
        for y in bottom..=top {
            let graphic = group.cells[&CellPoint { x: bar_x, y, z: 0 }].graphic.clone();
            assert!(
                matches!(graphic, CellGraphic::Glyph('┳' | '┻' | '┃' | '║')),
                "non-bar cell at y {y}: {graphic:?}"
            );
        }
    }

    #[test]
    fn no_label_draws_under_the_visible_scrollbar() {
        let toolbox = ToolboxModule::new("toolbox", rect(), tool_state(), overflowing_tool_defs());
        let (bar_x, _, _) = match toolbox.scrollbar_span() {
            ScrollbarSpan::Vertical { x, top, bottom } => (x, top, bottom),
            ScrollbarSpan::Horizontal { .. } => unreachable!(),
        };
        // Scroll to the bottom: the long-label Stamp row is visible. Its
        // label crops one column short of the bar (J's bar-aware rule).
        let mut toolbox = toolbox;
        for _ in 0..3 {
            toolbox.on_wheel(0, 0, 0.0, -1.0);
        }
        assert_eq!(toolbox.scroll.offset(), 3);
        let group = toolbox.draw();
        let stamp_y = toolbox.row_y(2);
        assert_eq!(
            group.cells[&CellPoint { x: 15, y: stamp_y, z: 0 }].graphic,
            CellGraphic::Glyph('l'),
            "the label fills up to the bar column"
        );
        assert!(!matches!(
            group.cells[&CellPoint { x: bar_x, y: stamp_y, z: 0 }].graphic,
            CellGraphic::Glyph('T')
        ));
    }

    #[test]
    fn wheel_scrolls_the_tool_rows_and_click_selects_the_scrolled_tool() {
        let state = tool_state();
        let mut toolbox = ToolboxModule::new("toolbox", rect(), state.clone(), overflowing_tool_defs());
        // Wheel down one row: Brush crops off the top, Fill lands in the
        // first visible slot.
        assert!(toolbox.on_wheel(0, 0, 0.0, -1.0));
        assert_eq!(toolbox.scroll.offset(), 1);
        toolbox.on_pointer_event(ModulePointerEvent::Click {
            x: 3,
            y: toolbox.row_y(0),
            button: ModulePointerButton::Left,
        });
        assert_eq!(state.borrow().left_tool, PaintTool::Fill);
        // Wheel up past the top clamps at 0 and Brush is back.
        for _ in 0..5 {
            toolbox.on_wheel(0, 0, 0.0, 1.0);
        }
        assert_eq!(toolbox.scroll.offset(), 0);
        toolbox.on_pointer_event(ModulePointerEvent::Click {
            x: 3,
            y: toolbox.row_y(0),
            button: ModulePointerButton::Left,
        });
        assert_eq!(state.borrow().left_tool, PaintTool::Brush);
    }

    #[test]
    fn wheel_falls_through_when_every_tool_fits() {
        let mut toolbox = ToolboxModule::new("toolbox", rect(), tool_state(), tool_defs());
        assert!(!toolbox.on_wheel(0, 0, 0.0, -1.0));
    }

    #[test]
    fn thumb_drag_walks_the_tool_rows_to_both_edges() {
        let mut toolbox = ToolboxModule::new("toolbox", rect(), tool_state(), overflowing_tool_defs());
        let (bar_x, top, bottom) = match toolbox.scrollbar_span() {
            ScrollbarSpan::Vertical { x, top, bottom } => (x, top, bottom),
            ScrollbarSpan::Horizontal { .. } => unreachable!(),
        };
        assert!(toolbox.scrollbar_press(bar_x, top));
        assert!(toolbox.scrollbar_drag(bar_x, bottom));
        assert_eq!(toolbox.scroll.offset(), toolbox.max_scroll_rows());
        assert!(toolbox.scrollbar_drag(bar_x, top));
        assert_eq!(toolbox.scroll.offset(), 0);
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
