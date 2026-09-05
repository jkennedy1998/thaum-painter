use std::{cell::RefCell, rc::Rc};

use thaum_renderer_domain::{
    ColorPickerModule, Module, ModulePointerButton, ModulePointerEvent, ModuleRect,
    PersistedModuleUiState, UiPalette,
};

use crate::{
    legacy_indexed_palette,
    paint_color::PaintColor,
    tool_state::{PaintHand, ToolState},
};

pub struct PaintColorPickerModule {
    inner: ColorPickerModule,
    tool_state: Rc<RefCell<ToolState>>,
}

impl PaintColorPickerModule {
    pub fn new(
        id: impl Into<String>,
        rect: ModuleRect,
        tool_state: Rc<RefCell<ToolState>>,
        palette: UiPalette,
    ) -> Self {
        Self {
            inner: ColorPickerModule::new(id, rect, palette)
                .with_indexed_palette(legacy_indexed_palette()),
            tool_state,
        }
    }
}

impl Module for PaintColorPickerModule {
    fn id(&self) -> &str {
        self.inner.id()
    }

    fn rect(&self) -> ModuleRect {
        self.inner.rect()
    }

    fn draw(&self) -> thaum_renderer_domain::CellGroup {
        self.inner.draw()
    }

    fn on_pointer_event(&mut self, event: ModulePointerEvent) {
        let clicked_hand = match event {
            ModulePointerEvent::Click {
                button: ModulePointerButton::Left,
                ..
            } => Some(PaintHand::Left),
            ModulePointerEvent::Click {
                button: ModulePointerButton::Right,
                ..
            } => Some(PaintHand::Right),
            _ => None,
        };
        let previous = self.inner.selected_rgb();
        self.inner.on_pointer_event(event);
        let Some(hand) = clicked_hand else {
            return;
        };
        let Some(rgb) = self.inner.selected_rgb() else {
            return;
        };
        if Some(rgb) == previous {
            return;
        }
        self.tool_state
            .borrow_mut()
            .set_color_for_hand(hand, PaintColor::flat_rgb(rgb[0], rgb[1], rgb[2]));
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
    use thaum_renderer_domain::ModulePointerButton;

    fn rect() -> ModuleRect {
        ModuleRect {
            x0: 0,
            y0: 0,
            x1: 16,
            y1: 8,
        }
    }

    fn click_x_for_order_index(index: i32) -> i32 {
        1 + index
    }

    fn click_y_for_top_row() -> i32 {
        5
    }

    #[test]
    fn left_click_assigns_the_left_hand_color() {
        let tool_state = Rc::new(RefCell::new(ToolState::default()));
        let mut module =
            PaintColorPickerModule::new("picker", rect(), tool_state.clone(), UiPalette::default());

        module.on_pointer_event(ModulePointerEvent::Click {
            x: click_x_for_order_index(2),
            y: click_y_for_top_row(),
            button: ModulePointerButton::Left,
        });

        let state = tool_state.borrow();
        let expected = legacy_indexed_palette()[2];
        assert_eq!(
            state.left_hand.color,
            PaintColor::flat_rgb(expected[0], expected[1], expected[2])
        );
        assert_eq!(state.active_hand, PaintHand::Left);
    }

    #[test]
    fn right_click_assigns_the_right_hand_color() {
        let tool_state = Rc::new(RefCell::new(ToolState::default()));
        let mut module =
            PaintColorPickerModule::new("picker", rect(), tool_state.clone(), UiPalette::default());

        module.on_pointer_event(ModulePointerEvent::Click {
            x: click_x_for_order_index(3),
            y: click_y_for_top_row(),
            button: ModulePointerButton::Right,
        });

        let state = tool_state.borrow();
        let expected = legacy_indexed_palette()[3];
        assert_eq!(
            state.right_hand.color,
            PaintColor::flat_rgb(expected[0], expected[1], expected[2])
        );
        assert_eq!(state.active_hand, PaintHand::Right);
    }
}
