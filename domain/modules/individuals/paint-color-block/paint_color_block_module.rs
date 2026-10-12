use std::{cell::RefCell, rc::Rc};

use thaum_renderer_domain::{
    CellColor, CellGraphic, CellWeight, ColorBlockModule, Hotspot, Module, ModulePointerButton,
    ModulePointerEvent, ModuleRect, PersistedModuleUiState, UiPalette,
};

use crate::{
    paint_color::PaintColor,
    tool_state::{PaintHand, ToolState},
};

pub struct PaintColorBlockModule {
    /// The generic renderer color block. `RefCell` because the wrapper's
    /// draw is `&self` while a file-palette edit may need to re-band the
    /// field in the same pass (the trait fixes `draw(&self)`).
    inner: RefCell<ColorBlockModule>,
    /// The module id, kept directly so `Module::id` can return `&str` tied
    /// to the wrapper's lifetime (the inner id sits behind the RefCell).
    id: String,
    tool_state: Rc<RefCell<ToolState>>,
    active_hand: Option<PaintHand>,
    last_synced_left_rgb: Option<[u8; 3]>,
    last_synced_right_rgb: Option<[u8; 3]>,
    /// The palette the inner block currently bands against, so a file
    /// palette edit through the palette module re-bands on the next pass
    /// instead of leaving the field stale.
    applied_palette: RefCell<Vec<[u8; 3]>>,
}

impl PaintColorBlockModule {
    pub fn new(
        id: impl Into<String>,
        rect: ModuleRect,
        tool_state: Rc<RefCell<ToolState>>,
        palette: UiPalette,
    ) -> Self {
        let id = id.into();
        let applied_palette = tool_state.borrow().file_palette.clone();
        Self {
            inner: RefCell::new(
                ColorBlockModule::new(id.clone(), rect, palette)
                    .with_indexed_palette(&applied_palette),
            ),
            id,
            tool_state,
            active_hand: None,
            last_synced_left_rgb: None,
            last_synced_right_rgb: None,
            applied_palette: RefCell::new(applied_palette),
        }
    }

    /// Re-reads the live file palette and re-bands the block when it moved
    /// (a palette module add/remove). The user's current selection stays
    /// where they left it; the next snap resolves against the new list.
    fn sync_palette(&self) {
        let live = self.tool_state.borrow().file_palette.clone();
        if *self.applied_palette.borrow() == live {
            return;
        }
        self.inner.borrow_mut().set_indexed_palette(&live);
        *self.applied_palette.borrow_mut() = live;
    }

    /// Whether the block currently snaps picks to the indexed palette.
    /// The toggle itself lives on the block's own clamp widget.
    pub fn clamping(&self) -> bool {
        self.inner.borrow().clamped()
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
        self.inner.borrow_mut().set_selected_rgb(rgb);
        self.set_last_synced_rgb(hand, rgb);
    }

    fn apply_selected_color_to_hand(&mut self, hand: PaintHand) {
        let [red, green, blue] = self.inner.borrow().selected_rgb();
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
        &self.id
    }

    fn rect(&self) -> ModuleRect {
        self.inner.borrow().rect()
    }

    fn draw(&self) -> thaum_renderer_domain::CellGroup {
        self.sync_palette();
        let mut group = self.inner.borrow().draw();
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
                let previous = self.inner.borrow().selected_rgb();
                self.inner.borrow_mut().on_pointer_event(event);
                if self.inner.borrow().selected_rgb() != previous {
                    self.apply_selected_color_to_hand(PaintHand::Left);
                }
            }
            ModulePointerEvent::Click {
                button: ModulePointerButton::Right,
                ..
            } => {
                self.active_hand = Some(PaintHand::Right);
                self.sync_from_hand(PaintHand::Right);
                let previous = self.inner.borrow().selected_rgb();
                self.inner.borrow_mut().on_pointer_event(event);
                if self.inner.borrow().selected_rgb() != previous {
                    self.apply_selected_color_to_hand(PaintHand::Right);
                }
            }
            ModulePointerEvent::Move { .. } => {
                let previous = self.inner.borrow().selected_rgb();
                self.inner.borrow_mut().on_pointer_event(event);
                if let Some(hand) = self.active_hand {
                    if self.inner.borrow().selected_rgb() != previous {
                        self.apply_selected_color_to_hand(hand);
                    }
                }
            }
            ModulePointerEvent::Up { .. } => {
                self.inner.borrow_mut().on_pointer_event(event);
                self.active_hand = None;
            }
            _ => self.inner.borrow_mut().on_pointer_event(event),
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
        let previous = self.inner.borrow().selected_rgb();
        let handled = self.inner.borrow_mut().on_wheel(x, y, delta_x, delta_y);
        if self.inner.borrow().selected_rgb() != previous {
            self.apply_selected_color_to_hand(hand);
        }
        handled
    }

    fn wants_pointer_capture(&self) -> bool {
        self.inner.borrow().wants_pointer_capture()
    }

    fn is_hidden(&self) -> bool {
        self.inner.borrow().is_hidden()
    }

    fn set_hidden(&mut self, hidden: bool) {
        self.inner.borrow_mut().set_hidden(hidden);
    }

    fn hotspots(&self) -> Vec<Hotspot> {
        self.inner.borrow().hotspots()
    }

    fn persisted_ui_state(&self) -> Option<PersistedModuleUiState> {
        self.inner.borrow().persisted_ui_state()
    }

    fn apply_persisted_ui_state(&mut self, state: &PersistedModuleUiState) {
        self.inner.borrow_mut().apply_persisted_ui_state(state);
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
        let selected = module.inner.borrow().selected_rgb();
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
        let original = tool_state.borrow().right_hand.color.clone();
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
    fn draw_re_bands_the_block_after_a_file_palette_edit() {
        let tool_state = Rc::new(RefCell::new(ToolState::default()));
        let magenta = [255u8, 0, 255];
        let mut module =
            PaintColorBlockModule::new("block", rect(), tool_state.clone(), UiPalette::default());
        // One deterministic pick: the hue-slider row holds saturation/value
        // from the hand color and sets hue to 4/5, so the hand color fully
        // determines what the block bands onto.
        let slider_pick = |module: &mut PaintColorBlockModule| {
            module.on_pointer_event(ModulePointerEvent::Click {
                x: 16,
                y: 7,
                button: ModulePointerButton::Left,
            });
        };
        let magenta_hand = || PaintColor::flat_rgb(255, 0, 255);

        tool_state
            .borrow_mut()
            .set_color_for_hand(PaintHand::Left, magenta_hand());
        // The palette module seam adds magenta, but the block has not drawn
        // since: it still bands against the legacy 37, which has no pure
        // magenta, so the pick drags the hand to the nearest legacy entry.
        assert!(tool_state.borrow_mut().add_file_palette_color(magenta));
        slider_pick(&mut module);
        assert_ne!(
            tool_state.borrow().left_hand.color,
            magenta_hand(),
            "the stale block cannot band onto an unrebanded palette entry"
        );

        // The draw pass re-bands the block against the live file palette;
        // the same pick now holds the hand exactly on the added entry.
        module.draw();
        tool_state
            .borrow_mut()
            .set_color_for_hand(PaintHand::Left, magenta_hand());
        slider_pick(&mut module);
        assert_eq!(
            tool_state.borrow().left_hand.color,
            magenta_hand(),
            "the re-banded block must resolve the pick onto the added entry"
        );

        // Removing the color through the same seam and the next pass retire
        // it as a banding target again.
        assert!(tool_state.borrow_mut().remove_file_palette_color(magenta));
        module.draw();
        tool_state
            .borrow_mut()
            .set_color_for_hand(PaintHand::Left, magenta_hand());
        slider_pick(&mut module);
        assert_ne!(
            tool_state.borrow().left_hand.color,
            magenta_hand(),
            "a removed palette color must stop banding targets"
        );
    }

    #[test]
    fn wheel_scroll_accumulates_across_same_color_plateaus() {
        let tool_state = Rc::new(RefCell::new(ToolState::default()));
        tool_state
            .borrow_mut()
            .set_color_for_hand(PaintHand::Left, PaintColor::flat_rgb(197, 181, 168));
        let mut module =
            PaintColorBlockModule::new("block", rect(), tool_state.clone(), UiPalette::default());
        let original = tool_state.borrow().left_hand.color.clone();

        for _ in 0..12 {
            module.on_wheel(0, 0, 0.0, 1.0);
        }

        assert_ne!(tool_state.borrow().left_hand.color, original);
    }

    #[test]
    fn an_unclamped_pick_carries_the_raw_color_to_the_hand() {
        let tool_state = Rc::new(RefCell::new(ToolState::default()));
        let mut module =
            PaintColorBlockModule::new("block", rect(), tool_state.clone(), UiPalette::default());
        module.inner.borrow_mut().set_clamped(false);
        assert!(!module.clamping());

        module.on_pointer_event(ModulePointerEvent::Click {
            x: 4,
            y: 5,
            button: ModulePointerButton::Left,
        });

        // The raw off-palette pick lands on the hand, so the palette
        // module's `+` (add hand color) can commit it to the palette.
        let picked = module.inner.borrow().selected_rgb();
        assert!(!tool_state.borrow().file_palette.contains(&picked));
        assert_eq!(
            tool_state.borrow().left_hand.color,
            PaintColor::flat_rgb(picked[0], picked[1], picked[2])
        );
    }
}
