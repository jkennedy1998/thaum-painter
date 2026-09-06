use std::{cell::RefCell, rc::Rc};

use thaum_renderer_domain::{
    CellColor, CellGraphic, CellWeight, ColorBlockModule, Hotspot, Module, ModulePointerButton,
    ModulePointerEvent, ModuleRect, PersistedModuleUiState, UiPalette,
};

use crate::{
    legacy_indexed_palette,
    paint_color::PaintColor,
    tool_state::{PaintHand, ToolState},
};

pub struct PaintColorBlockModule {
    inner: ColorBlockModule,
    tool_state: Rc<RefCell<ToolState>>,
    active_hand: Option<PaintHand>,
    last_synced_left_rgb: Option<[u8; 3]>,
    last_synced_right_rgb: Option<[u8; 3]>,
}

impl PaintColorBlockModule {
    pub fn new(
        id: impl Into<String>,
        rect: ModuleRect,
        tool_state: Rc<RefCell<ToolState>>,
        palette: UiPalette,
    ) -> Self {
        Self {
            inner: ColorBlockModule::new(id, rect, palette)
                .with_indexed_palette(legacy_indexed_palette()),
            tool_state,
            active_hand: None,
            last_synced_left_rgb: None,
            last_synced_right_rgb: None,
        }
    }

    fn hand_rgb(&self, hand: PaintHand) -> [u8; 3] {
        let rgb = self
            .tool_state
            .borrow()
            .hand_state(hand)
            .color
            .preview_rgb();
        [rgb.0, rgb.1, rgb.2]
    }

    fn last_synced_rgb(&self, hand: PaintHand) -> Option<[u8; 3]> {
        match hand {
            PaintHand::Left => self.last_synced_left_rgb,
            PaintHand::Right => self.last_synced_right_rgb,
        }
    }

    fn set_last_synced_rgb(&mut self, hand: PaintHand, rgb: [u8; 3]) {
        match hand {
            PaintHand::Left => self.last_synced_left_rgb = Some(rgb),
            PaintHand::Right => self.last_synced_right_rgb = Some(rgb),
        }
    }

    fn sync_from_hand(&mut self, hand: PaintHand) {
        let rgb = self.hand_rgb(hand);
        self.inner.set_selected_rgb(rgb);
        self.set_last_synced_rgb(hand, rgb);
    }

    fn apply_selected_color_to_hand(&mut self, hand: PaintHand) {
        let [red, green, blue] = self.inner.selected_rgb();
        let rgb = [red, green, blue];
        self.tool_state
            .borrow_mut()
            .set_color_for_hand(hand, PaintColor::flat_rgb(red, green, blue));
        self.set_last_synced_rgb(hand, rgb);
    }
}

fn flat_rgb(color: CellColor) -> Option<[u8; 3]> {
    match color {
        CellColor::Flat([red, green, blue, _]) => Some([
            (red * 255.0).round().clamp(0.0, 255.0) as u8,
            (green * 255.0).round().clamp(0.0, 255.0) as u8,
            (blue * 255.0).round().clamp(0.0, 255.0) as u8,
        ]),
        _ => None,
    }
}

impl Module for PaintColorBlockModule {
    fn id(&self) -> &str {
        self.inner.id()
    }

    fn rect(&self) -> ModuleRect {
        self.inner.rect()
    }

    fn draw(&self) -> thaum_renderer_domain::CellGroup {
        let mut group = self.inner.draw();
        // Selection highlight: color cells matching either hand's live color
        // draw at weight 3, every other color cell at weight 1. Resolved from
        // tool state at draw time so the highlight can never go stale when a
        // hand color changes elsewhere (hand settings, other picker, etc.).
        let left = self.hand_rgb(PaintHand::Left);
        let right = self.hand_rgb(PaintHand::Right);
        for cell in group.cells.values_mut() {
            if !matches!(cell.graphic, CellGraphic::Glyph('█')) {
                continue;
            }
            let selected = flat_rgb(cell.color).is_some_and(|rgb| rgb == left || rgb == right);
            cell.weight = CellWeight::from_index_clamped(if selected { 3 } else { 1 });
        }
        group
    }

    fn on_pointer_event(&mut self, event: ModulePointerEvent) {
        match event {
            ModulePointerEvent::Click {
                button: ModulePointerButton::Left,
                ..
            } => {
                self.active_hand = Some(PaintHand::Left);
                self.sync_from_hand(PaintHand::Left);
                let previous = self.inner.selected_rgb();
                self.inner.on_pointer_event(event);
                if self.inner.selected_rgb() != previous {
                    self.apply_selected_color_to_hand(PaintHand::Left);
                }
            }
            ModulePointerEvent::Click {
                button: ModulePointerButton::Right,
                ..
            } => {
                self.active_hand = Some(PaintHand::Right);
                self.sync_from_hand(PaintHand::Right);
                let previous = self.inner.selected_rgb();
                self.inner.on_pointer_event(event);
                if self.inner.selected_rgb() != previous {
                    self.apply_selected_color_to_hand(PaintHand::Right);
                }
            }
            ModulePointerEvent::Move { .. } => {
                let previous = self.inner.selected_rgb();
                self.inner.on_pointer_event(event);
                if let Some(hand) = self.active_hand {
                    if self.inner.selected_rgb() != previous {
                        self.apply_selected_color_to_hand(hand);
                    }
                }
            }
            ModulePointerEvent::Up { .. } => {
                self.inner.on_pointer_event(event);
                self.active_hand = None;
            }
            _ => self.inner.on_pointer_event(event),
        }
    }

    fn on_wheel(&mut self, x: i32, y: i32, delta_x: f32, delta_y: f32) -> bool {
        let hand = self
            .active_hand
            .unwrap_or(self.tool_state.borrow().active_hand);
        let hand_rgb = self.hand_rgb(hand);
        if self.active_hand.is_none() && self.last_synced_rgb(hand) != Some(hand_rgb) {
            self.sync_from_hand(hand);
        }
        let previous = self.inner.selected_rgb();
        let handled = self.inner.on_wheel(x, y, delta_x, delta_y);
        if self.inner.selected_rgb() != previous {
            self.apply_selected_color_to_hand(hand);
        }
        handled
    }

    fn wants_pointer_capture(&self) -> bool {
        self.inner.wants_pointer_capture()
    }

    fn is_hidden(&self) -> bool {
        self.inner.is_hidden()
    }

    fn set_hidden(&mut self, hidden: bool) {
        self.inner.set_hidden(hidden);
    }

    fn hotspots(&self) -> Vec<Hotspot> {
        self.inner.hotspots()
    }

    fn persisted_ui_state(&self) -> Option<PersistedModuleUiState> {
        self.inner.persisted_ui_state()
    }

    fn apply_persisted_ui_state(&mut self, state: &PersistedModuleUiState) {
        self.inner.apply_persisted_ui_state(state);
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
            y1: 11,
        }
    }

    #[test]
    fn drawing_highlights_hand_colors_at_weight_three_and_other_colors_at_weight_one() {
        let tool_state = Rc::new(RefCell::new(ToolState::default()));
        let module =
            PaintColorBlockModule::new("block", rect(), tool_state.clone(), UiPalette::default());
        // Anchor one hand to the picker's current selection so at least one
        // drawn color cell (the preview swatch) is guaranteed to match.
        let selected = module.inner.selected_rgb();
        tool_state.borrow_mut().set_color_for_hand(
            PaintHand::Left,
            PaintColor::flat_rgb(selected[0], selected[1], selected[2]),
        );

        let group = module.draw();
        let mut highlighted = 0;
        let mut plain = 0;
        for cell in group.cells.values() {
            if !matches!(cell.graphic, CellGraphic::Glyph('█')) {
                continue;
            }
            let Some(rgb) = flat_rgb(cell.color) else {
                continue;
            };
            if rgb == selected {
                assert_eq!(cell.weight, CellWeight::from_index_clamped(3));
                highlighted += 1;
            } else {
                assert_eq!(cell.weight, CellWeight::from_index_clamped(1));
                plain += 1;
            }
        }
        assert!(highlighted >= 1);
        assert!(plain > 0);
    }

    #[test]
    fn left_click_assigns_a_flat_color_to_the_left_hand() {
        let tool_state = Rc::new(RefCell::new(ToolState::default()));
        let mut module =
            PaintColorBlockModule::new("block", rect(), tool_state.clone(), UiPalette::default());

        module.on_pointer_event(ModulePointerEvent::Click {
            x: 4,
            y: 5,
            button: ModulePointerButton::Left,
        });

        assert!(matches!(
            tool_state.borrow().left_hand.color,
            PaintColor::FlatRgb(_, _, _)
        ));
    }

    #[test]
    fn dragging_updates_the_active_hand_color() {
        let tool_state = Rc::new(RefCell::new(ToolState::default()));
        let original = tool_state.borrow().right_hand.color;
        let mut module =
            PaintColorBlockModule::new("block", rect(), tool_state.clone(), UiPalette::default());

        module.on_pointer_event(ModulePointerEvent::Click {
            x: 4,
            y: 5,
            button: ModulePointerButton::Right,
        });
        module.on_pointer_event(ModulePointerEvent::Move { x: 10, y: 9 });

        assert_ne!(tool_state.borrow().right_hand.color, original);
    }

    #[test]
    fn wheel_scroll_accumulates_across_same_color_plateaus() {
        let tool_state = Rc::new(RefCell::new(ToolState::default()));
        tool_state
            .borrow_mut()
            .set_color_for_hand(PaintHand::Left, PaintColor::flat_rgb(197, 181, 168));
        let mut module =
            PaintColorBlockModule::new("block", rect(), tool_state.clone(), UiPalette::default());
        let original = tool_state.borrow().left_hand.color;

        for _ in 0..12 {
            module.on_wheel(0, 0, 0.0, 1.0);
        }

        assert_ne!(tool_state.borrow().left_hand.color, original);
    }
}
