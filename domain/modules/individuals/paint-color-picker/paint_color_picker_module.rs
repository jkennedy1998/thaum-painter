use std::{cell::RefCell, rc::Rc};

use thaum_renderer_domain::{
    Cell, CellColor, CellGraphic, CellGroup, CellPoint, CellWeight, ColorPickerModule, Hotspot,
    Module, ModulePointerButton, ModulePointerEvent, ModuleRect, PersistedModuleUiState,
    UiColorRole, UiPalette, WorldPoint,
};

use crate::{
    color_entry::{ColorEntryEdit, ColorEntryField},
    legacy_indexed_palette::{hex_color_string, rgb_color_string},
    paint_color::PaintColor,
    tool_state::{PaintHand, ToolState},
};

/// The palette edit block's rows, inside the panel's own content area (the
/// COLORS panel stays the single interface for editing the file's index
/// colors). Local y within the module rect — y grows upward, so row 2 is
/// directly above row 1. The rows are the bottom two content rows of the
/// panel's single chrome (the renderer picker owns the border and reserves
/// these rows from its swatch grid via `with_footer_rows`):
/// - row 2: `1` = `−` remove the selected swatch, `2` = `+` commit the
///   entry color, `4..=9` = hex entry display (click opens the in-place
///   hex edit), `11` = the entry color preview swatch.
/// - row 1: `1..=11` = the linked rgb entry display (`RRR GGG BBB`, click
///   opens the in-place rgb edit).
/// The hex and rgb fields stay linked: a complete value in either rewrites
/// the other, and `+` adds that one color to the indexed palette.
const STRIP_ROWS: i32 = 2;
const STRIP_HEX_ROW: i32 = 2;
const STRIP_RGB_ROW: i32 = 1;
const STRIP_MINUS: i32 = 1;
const STRIP_PLUS: i32 = 2;
const STRIP_HEX_START: i32 = 4;
const STRIP_HEX_END: i32 = 9;
const STRIP_PREVIEW: i32 = 11;
const STRIP_RGB_START: i32 = 1;
const STRIP_RGB_END: i32 = 11;

pub struct PaintColorPickerModule {
    /// The generic renderer picker. `RefCell` because the wrapper's draw is
    /// `&self` while a file-palette edit may need to re-seed the swatch list
    /// in the same pass (the trait fixes `draw(&self)`). Its rect is the
    /// module's own rect — one chrome, one bottom border — with the strip
    /// rows reserved out of its swatch grid via `with_footer_rows`.
    inner: RefCell<ColorPickerModule>,
    /// The module id, kept directly so `Module::id` can return `&str` tied
    /// to the wrapper's lifetime (the inner id sits behind the RefCell).
    id: String,
    tool_state: Rc<RefCell<ToolState>>,
    palette: UiPalette,
    /// Color entry edit state, shared with the typing seam (the hand-settings
    /// number fields use the same shape): opened by a click on one of the
    /// strip's entry fields, keys route here, `+`/Enter commits the parsed
    /// color into the live file palette.
    color_entry: Rc<RefCell<Option<ColorEntryEdit>>>,
    /// The palette the inner picker currently draws, so a file-palette edit
    /// re-syncs the swatch list on the next pass instead of leaving the
    /// swatches stale.
    applied_palette: RefCell<Vec<[u8; 3]>>,
}

impl PaintColorPickerModule {
    pub fn new(
        id: impl Into<String>,
        rect: ModuleRect,
        tool_state: Rc<RefCell<ToolState>>,
        palette: UiPalette,
        color_entry: Rc<RefCell<Option<ColorEntryEdit>>>,
    ) -> Self {
        let id = id.into();
        let applied_palette = tool_state.borrow().file_palette.clone();
        Self {
            inner: RefCell::new(
                ColorPickerModule::new(id.clone(), rect, palette.clone())
                    .with_indexed_palette(&applied_palette)
                    .with_footer_rows(STRIP_ROWS),
            ),
            id,
            tool_state,
            palette,
            color_entry,
            applied_palette: RefCell::new(applied_palette),
        }
    }

    /// The active hand's flat color preview as RGB.
    fn active_hand_rgb(&self) -> [u8; 3] {
        let rgb = self
            .tool_state
            .borrow()
            .hand_state(self.tool_state.borrow().active_hand)
            .color
            .preview_rgb();
        [rgb.0, rgb.1, rgb.2]
    }

    /// Re-reads the live file palette and re-seeds the inner picker when it
    /// moved (a `+`/`−` edit, or a restored document). Display order is
    /// derived from the colors themselves; a selection survives when its
    /// color is still in the list.
    fn sync_palette(&self) {
        let live = self.tool_state.borrow().file_palette.clone();
        if *self.applied_palette.borrow() == live {
            return;
        }
        self.inner.borrow_mut().set_indexed_palette(&live);
        *self.applied_palette.borrow_mut() = live;
    }

    /// Whether the in-place color entry is open right now.
    pub fn color_entry_is_open(&self) -> bool {
        self.color_entry.borrow().is_some()
    }

    /// Opens the color entry on `field`, or when it is already open moves
    /// the typing focus to that field without touching either buffer.
    fn open_or_focus_entry(&self, field: ColorEntryField) {
        let hand = self.active_hand_rgb();
        let mut slot = self.color_entry.borrow_mut();
        match slot.as_mut() {
            Some(edit) => edit.field = field,
            None => *slot = Some(ColorEntryEdit::begin(hand, field)),
        }
    }

    fn handle_strip_click(&mut self, offset: i32) {
        match offset {
            STRIP_MINUS => {
                // Copy the selection out before mutating: the strip click's
                // read must not hold the inner borrow through sync_palette.
                let selected = self.inner.borrow().selected_rgb();
                // Where the doomed color sits in the display order, so the
                // color that takes its slot can become the next selection.
                let removed_position = selected.and_then(|rgb| {
                    let inner = self.inner.borrow();
                    inner
                        .display_order()
                        .iter()
                        .position(|&index| inner.indexed_palette()[index] == rgb)
                });
                // Minus drops the palette entry only: painted cells keep
                // their raw RGB (out of scope by decision, J 2026-10-10).
                if let Some(rgb) = selected {
                    self.tool_state.borrow_mut().remove_file_palette_color(rgb);
                }
                // A minus click also closes an open entry: the click left
                // the entry fields.
                self.color_entry.borrow_mut().take();
                self.sync_palette();
                // Wipe flow (J 2026-10-11): the selection carries to the
                // color now at the removed swatch's display position, so
                // pressing `−` again removes the next one in sorted order.
                if let Some(position) = removed_position {
                    self.inner.borrow_mut().select_display_position(position);
                }
            }
            STRIP_PLUS => {
                let open = self.color_entry.borrow().is_some();
                if open {
                    // `+` commits the entry color: whatever the linked hex /
                    // rgb fields currently resolve to. An incomplete entry
                    // refuses and stays open so the typing is not lost.
                    let committed = self
                        .color_entry
                        .borrow_mut()
                        .as_mut()
                        .expect("the entry is open")
                        .commit();
                    if let Some(rgb) = committed {
                        self.color_entry.borrow_mut().take();
                        self.tool_state.borrow_mut().add_file_palette_color(rgb);
                    }
                } else {
                    // No entry open: the fields display the hand color, so
                    // `+` adds exactly that.
                    let rgb = self.active_hand_rgb();
                    self.tool_state.borrow_mut().add_file_palette_color(rgb);
                }
                self.sync_palette();
            }
            STRIP_HEX_START..=STRIP_HEX_END => {
                self.open_or_focus_entry(ColorEntryField::Hex);
            }
            _ => {}
        }
    }

    /// The hex row's text: the open entry's hex buffer, or the hand color
    /// when the entry is closed.
    fn hex_display(&self) -> (String, bool) {
        let edit_open = self.color_entry_is_open();
        let text = if edit_open {
            self.color_entry
                .borrow()
                .as_ref()
                .map(|edit| edit.hex_text().to_string())
                .unwrap_or_default()
        } else {
            hex_color_string(self.active_hand_rgb())[1..].to_string()
        };
        (text, edit_open)
    }

    /// The rgb row's text: the open entry's rgb buffer, or the hand color
    /// when the entry is closed.
    fn rgb_display(&self) -> (String, bool) {
        let edit_open = self.color_entry_is_open();
        let text = if edit_open {
            self.color_entry
                .borrow()
                .as_ref()
                .map(|edit| edit.rgb_text().to_string())
                .unwrap_or_default()
        } else {
            rgb_color_string(self.active_hand_rgb())
        };
        (text, edit_open)
    }

    /// The color the entry currently resolves to — the hand color while the
    /// entry is closed, the last fully valid entry color while it is open.
    fn preview_rgb(&self) -> [u8; 3] {
        if self.color_entry_is_open() {
            self.color_entry
                .borrow()
                .as_ref()
                .map(|edit| edit.preview_rgb())
                .unwrap_or_else(|| self.active_hand_rgb())
        } else {
            self.active_hand_rgb()
        }
    }

    /// The strip block's cells: the rgb entry row and the hex entry row,
    /// rendered as content in the bottom two rows of the panel's own content
    /// area. The panel chrome (border, background, title) stays owned by the
    /// inner picker — nothing is hand-drawn here but the strip's own glyphs.
    fn strip_cells(&self) -> Vec<Cell> {
        let bright = self.palette.get(UiColorRole::Bright);
        let vivid = self.palette.get(UiColorRole::Vivid);
        let dim = self.palette.get(UiColorRole::Medium);

        let mut cells = Vec::new();
        let mut push = |row: i32, offset: i32, glyph: char, color: CellColor, weight: i32| {
            cells.push(Cell {
                position: CellPoint {
                    x: offset,
                    y: row,
                    z: 0,
                },
                graphic: CellGraphic::Glyph(glyph),
                color,
                weight: CellWeight::from_index_clamped(weight),
                ..Cell::default()
            });
        };

        let (hex_text, edit_open) = self.hex_display();
        let text_color = if edit_open { vivid.clone() } else { bright.clone() };
        let has_selection = self.inner.borrow().selected_rgb().is_some();
        push(
            STRIP_HEX_ROW,
            STRIP_MINUS,
            '-',
            if has_selection {
                text_color.clone()
            } else {
                dim.clone()
            },
            2,
        );
        push(STRIP_HEX_ROW, STRIP_PLUS, '+', text_color.clone(), 2);
        let hex_chars: Vec<char> = hex_text.chars().collect();
        for (index, offset) in (STRIP_HEX_START..=STRIP_HEX_END).enumerate() {
            let glyph = hex_chars.get(index).copied().unwrap_or(' ');
            push(STRIP_HEX_ROW, offset, glyph, text_color.clone(), 2);
        }
        let preview = self.preview_rgb();
        push(
            STRIP_HEX_ROW,
            STRIP_PREVIEW,
            ' ',
            CellColor::Flat([
                preview[0] as f32 / 255.0,
                preview[1] as f32 / 255.0,
                preview[2] as f32 / 255.0,
                1.0,
            ]),
            2,
        );

        let (rgb_text, _) = self.rgb_display();
        let rgb_chars: Vec<char> = rgb_text.chars().collect();
        for (index, offset) in (STRIP_RGB_START..=STRIP_RGB_END).enumerate() {
            let glyph = rgb_chars.get(index).copied().unwrap_or(' ');
            push(STRIP_RGB_ROW, offset, glyph, text_color.clone(), 2);
        }
        cells
    }

    /// The strip block's tooltip hotspots, positioned inside the module's
    /// lower rows.
    fn strip_hotspots(&self) -> Vec<Hotspot> {
        let module_rect = self.rect();
        let spot = |offset: i32, row: i32, label: &str, text: &str| -> Hotspot {
            Hotspot::new(
                ModuleRect {
                    x0: module_rect.x0 + offset,
                    y0: module_rect.y0 + row,
                    x1: module_rect.x0 + offset,
                    y1: module_rect.y0 + row,
                },
                label,
                text,
            )
        };
        let hand_hex = hex_color_string(self.active_hand_rgb());
        vec![
            spot(
                STRIP_MINUS,
                STRIP_HEX_ROW,
                "remove color",
                "click to drop the selected swatch from this file's index palette. painted cells keep their rgb.",
            ),
            spot(
                STRIP_PLUS,
                STRIP_HEX_ROW,
                "add color",
                "click to add the entry color (hex or rgb field) to this file's index palette. with no entry open it adds your current hand color.",
            ),
            spot(
                STRIP_HEX_START,
                STRIP_HEX_ROW,
                "hex entry",
                &format!(
                    "click to type a hex color (RRGGBB); the rgb field mirrors it. + adds it to the palette. current hand color {hand_hex}."
                ),
            ),
            spot(
                STRIP_RGB_START,
                STRIP_RGB_ROW,
                "rgb entry",
                &format!(
                    "click to type an rgb color (R G B); the hex field mirrors it. + adds it to the palette. current hand color {hand_hex}."
                ),
            ),
            spot(
                STRIP_PREVIEW,
                STRIP_HEX_ROW,
                "entry color",
                "the color the entry currently resolves to.",
            ),
        ]
    }
}

impl Module for PaintColorPickerModule {
    fn id(&self) -> &str {
        &self.id
    }

    /// The module's bounds: the inner picker owns the full panel chrome,
    /// so its rect is the module's rect.
    fn rect(&self) -> ModuleRect {
        self.inner.borrow().rect()
    }

    fn draw(&self) -> CellGroup {
        self.sync_palette();
        let inner_group = self.inner.borrow().draw();
        let inner_rect = self.inner.borrow().rect();
        let mut combined = CellGroup::new(WorldPoint {
            x: inner_rect.x0,
            y: inner_rect.y0,
            z: 0,
        });
        combined.intake_behavior = inner_group.intake_behavior;
        combined.facing = inner_group.facing;
        for (_, cell) in inner_group.cells {
            combined.insert(cell);
        }
        for cell in self.strip_cells() {
            combined.insert(cell);
        }
        combined
    }

    fn on_pointer_event(&mut self, event: ModulePointerEvent) {
        if let ModulePointerEvent::Click {
            x,
            y,
            button: ModulePointerButton::Left,
        } = event
        {
            let module_rect = self.rect();
            let offset = x - module_rect.x0;
            let row = y - module_rect.y0;
            if row == STRIP_HEX_ROW && (STRIP_MINUS..=STRIP_PREVIEW).contains(&offset) {
                self.handle_strip_click(offset);
                return;
            }
            if row == STRIP_RGB_ROW && (STRIP_RGB_START..=STRIP_RGB_END).contains(&offset) {
                self.open_or_focus_entry(ColorEntryField::Rgb);
                return;
            }
        }
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
        let previous = self.inner.borrow().selected_rgb();
        self.inner.borrow_mut().on_pointer_event(event);
        // Any click that is not a strip click closes an open entry.
        if clicked_hand.is_some() {
            self.color_entry.borrow_mut().take();
        }
        let Some(hand) = clicked_hand else {
            return;
        };
        let Some(rgb) = self.inner.borrow().selected_rgb() else {
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
        self.inner.borrow().wants_pointer_capture()
    }

    fn is_hidden(&self) -> bool {
        self.inner.borrow().is_hidden()
    }

    fn set_hidden(&mut self, hidden: bool) {
        self.inner.borrow_mut().set_hidden(hidden);
    }

    fn hotspots(&self) -> Vec<Hotspot> {
        let mut spots = self.strip_hotspots();
        spots.extend(self.inner.borrow().hotspots());
        spots
    }

    fn persisted_ui_state(&self) -> Option<PersistedModuleUiState> {
        let inner = self.inner.borrow();
        Some(PersistedModuleUiState::new(
            self.id(),
            self.rect(),
            inner.is_seamless(),
            inner.is_hidden(),
        ))
    }

    fn apply_persisted_ui_state(&mut self, state: &PersistedModuleUiState) {
        let mut inner = self.inner.borrow_mut();
        inner.set_rect(state.rect.to_runtime());
        inner.set_seamless(state.is_seamless);
        inner.set_hidden(state.is_hidden);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::legacy_indexed_palette::legacy_indexed_palette;
    use thaum_renderer_domain::ModulePointerButton;

    fn rect() -> ModuleRect {
        ModuleRect {
            x0: 0,
            y0: 0,
            x1: 16,
            y1: 8,
        }
    }

    fn module(tool_state: &Rc<RefCell<ToolState>>) -> PaintColorPickerModule {
        module_with_entry(tool_state, Rc::new(RefCell::new(None)))
    }

    fn module_with_entry(
        tool_state: &Rc<RefCell<ToolState>>,
        color_entry: Rc<RefCell<Option<ColorEntryEdit>>>,
    ) -> PaintColorPickerModule {
        PaintColorPickerModule::new(
            "picker",
            rect(),
            tool_state.clone(),
            UiPalette::default(),
            color_entry,
        )
    }

    /// The x of a swatch's click column given its display order position.
    fn click_x_for_display_position(position: i32) -> i32 {
        1 + position
    }

    fn click_y_for_top_row() -> i32 {
        // The swatch grid is anchored to the content area's top and the
        // strip's footer reserve only clips its capacity, so the top swatch
        // row's local y is unchanged by the strip: rect height 8 minus the
        // 1-cell inset and the 2-row header reserve leaves 5 content rows,
        // and the first display row sits at local y = 5.
        rect().y0 + 5
    }

    fn click(color_entry: &mut PaintColorPickerModule, x: i32, y: i32) {
        color_entry.on_pointer_event(ModulePointerEvent::Click {
            x,
            y,
            button: ModulePointerButton::Left,
        });
    }

    #[test]
    fn left_click_assigns_the_left_hand_color() {
        let tool_state = Rc::new(RefCell::new(ToolState::default()));
        let mut this = module(&tool_state);

        // display position 2, top content row
        click(&mut this, click_x_for_display_position(2), click_y_for_top_row());

        let state = tool_state.borrow();
        let expected = legacy_indexed_palette()[this.inner.borrow().display_order()[2]];
        assert_eq!(
            state.left_hand.color,
            PaintColor::flat_rgb(expected[0], expected[1], expected[2])
        );
        assert_eq!(state.active_hand, PaintHand::Left);
    }

    #[test]
    fn right_click_assigns_the_right_hand_color() {
        let tool_state = Rc::new(RefCell::new(ToolState::default()));
        let mut this = module(&tool_state);

        this.on_pointer_event(ModulePointerEvent::Click {
            x: click_x_for_display_position(3),
            y: click_y_for_top_row(),
            button: ModulePointerButton::Right,
        });

        let state = tool_state.borrow();
        let expected = legacy_indexed_palette()[this.inner.borrow().display_order()[3]];
        assert_eq!(
            state.right_hand.color,
            PaintColor::flat_rgb(expected[0], expected[1], expected[2])
        );
        assert_eq!(state.active_hand, PaintHand::Right);
    }

    #[test]
    fn plus_with_a_closed_entry_adds_the_active_hand_color_to_the_file_palette() {
        let tool_state = Rc::new(RefCell::new(ToolState::default()));
        let mut this = module(&tool_state);
        let hand_rgb: [u8; 3] = {
            let state = tool_state.borrow();
            let rgb = state.hand_state(state.active_hand).color.preview_rgb();
            [rgb.0, rgb.1, rgb.2]
        };
        let before = tool_state.borrow().file_palette.len();

        click(&mut this, rect().x0 + STRIP_PLUS, rect().y0 + STRIP_HEX_ROW);

        let state = tool_state.borrow();
        assert_eq!(state.file_palette.len(), before + 1);
        assert!(state.file_palette.contains(&hand_rgb));
    }

    #[test]
    fn plus_commits_the_open_entry_color_into_the_file_palette() {
        let tool_state = Rc::new(RefCell::new(ToolState::default()));
        let entry: Rc<RefCell<Option<ColorEntryEdit>>> = Rc::new(RefCell::new(None));
        let mut this = module_with_entry(&tool_state, entry.clone());

        // Open the hex field, then type a fresh color over the seed.
        click(
            &mut this,
            rect().x0 + STRIP_HEX_START,
            rect().y0 + STRIP_HEX_ROW,
        );
        assert!(this.color_entry_is_open());
        {
            let mut edit = entry.borrow_mut();
            for ch in "0770aa".chars() {
                edit.as_mut().expect("edit open").push(ch);
            }
        }

        click(&mut this, rect().x0 + STRIP_PLUS, rect().y0 + STRIP_HEX_ROW);

        assert!(!this.color_entry_is_open());
        assert!(tool_state.borrow().file_palette.contains(&[0x07, 0x70, 0xaa]));
    }

    #[test]
    fn plus_commits_the_open_rgb_entry_too() {
        let tool_state = Rc::new(RefCell::new(ToolState::default()));
        let entry: Rc<RefCell<Option<ColorEntryEdit>>> = Rc::new(RefCell::new(None));
        let mut this = module_with_entry(&tool_state, entry.clone());

        click(&mut this, rect().x0 + STRIP_RGB_START, rect().y0 + STRIP_RGB_ROW);
        assert!(this.color_entry_is_open());
        {
            let mut edit = entry.borrow_mut();
            for ch in "12,200,30".chars() {
                edit.as_mut().expect("edit open").push(ch);
            }
        }

        click(&mut this, rect().x0 + STRIP_PLUS, rect().y0 + STRIP_HEX_ROW);

        assert!(!this.color_entry_is_open());
        assert!(tool_state.borrow().file_palette.contains(&[12, 200, 30]));
    }

    #[test]
    fn plus_on_an_incomplete_entry_refuses_and_stays_open() {
        let tool_state = Rc::new(RefCell::new(ToolState::default()));
        let entry: Rc<RefCell<Option<ColorEntryEdit>>> = Rc::new(RefCell::new(None));
        let mut this = module_with_entry(&tool_state, entry.clone());
        let before = tool_state.borrow().file_palette.len();

        click(
            &mut this,
            rect().x0 + STRIP_HEX_START,
            rect().y0 + STRIP_HEX_ROW,
        );
        {
            let mut edit = entry.borrow_mut();
            // A single hex digit does not parse as a color.
            edit.as_mut().expect("edit open").push('7');
        }

        click(&mut this, rect().x0 + STRIP_PLUS, rect().y0 + STRIP_HEX_ROW);

        assert!(this.color_entry_is_open());
        assert_eq!(tool_state.borrow().file_palette.len(), before);
    }

    #[test]
    fn plus_on_an_existing_color_does_not_duplicate() {
        let tool_state = Rc::new(RefCell::new(ToolState::default()));
        let mut this = module(&tool_state);
        tool_state
            .borrow_mut()
            .set_color_for_hand(PaintHand::Left, PaintColor::flat_rgb(0xe3, 0x63, 0x25));
        let before = tool_state.borrow().file_palette.len();

        click(&mut this, rect().x0 + STRIP_PLUS, rect().y0 + STRIP_HEX_ROW);

        assert_eq!(tool_state.borrow().file_palette.len(), before);
    }

    #[test]
    fn minus_drops_the_selected_swatch_without_touching_cells() {
        let tool_state = Rc::new(RefCell::new(ToolState::default()));
        let mut this = module(&tool_state);
        // Click display position 2, which also makes it the active hand color.
        click(&mut this, click_x_for_display_position(2), click_y_for_top_row());
        let selected = this.inner.borrow().selected_rgb().expect("a swatch was clicked");
        let before = tool_state.borrow().file_palette.len();

        click(&mut this, rect().x0 + STRIP_MINUS, rect().y0 + STRIP_HEX_ROW);

        let state = tool_state.borrow();
        assert_eq!(state.file_palette.len(), before - 1);
        assert!(!state.file_palette.contains(&selected));
        // The hand keeps its color: palette membership is bookkeeping only.
        assert_eq!(
            state.left_hand.color,
            PaintColor::flat_rgb(selected[0], selected[1], selected[2])
        );
    }

    #[test]
    fn minus_carries_the_selection_to_the_color_taking_the_removed_slot() {
        let tool_state = Rc::new(RefCell::new(ToolState::default()));
        let mut this = module(&tool_state);
        click(&mut this, click_x_for_display_position(2), click_y_for_top_row());
        let removed = this.inner.borrow().selected_rgb().expect("a swatch was clicked");
        let successor = {
            let inner = this.inner.borrow();
            let order = inner.display_order();
            inner.indexed_palette()[order[3]]
        };

        click(&mut this, rect().x0 + STRIP_MINUS, rect().y0 + STRIP_HEX_ROW);

        assert!(!tool_state.borrow().file_palette.contains(&removed));
        // The color that took display position 2 is now the selection, so
        // pressing - again removes it (wipe without re-clicking a swatch).
        assert_eq!(this.inner.borrow().selected_rgb(), Some(successor));
        click(&mut this, rect().x0 + STRIP_MINUS, rect().y0 + STRIP_HEX_ROW);
        assert!(!tool_state.borrow().file_palette.contains(&successor));
    }

    #[test]
    fn minus_refuses_to_remove_the_last_remaining_color() {
        let tool_state = Rc::new(RefCell::new(ToolState::default()));
        let mut this = module(&tool_state);
        // Strip the palette down to one color.
        while tool_state.borrow().file_palette.len() > 1 {
            let rgb = tool_state.borrow().file_palette[0];
            tool_state.borrow_mut().remove_file_palette_color(rgb);
        }
        this.draw();
        let last = tool_state.borrow().file_palette[0];
        click(&mut this, click_x_for_display_position(0), click_y_for_top_row());
        assert_eq!(this.inner.borrow().selected_rgb(), Some(last));

        click(&mut this, rect().x0 + STRIP_MINUS, rect().y0 + STRIP_HEX_ROW);

        // The palette never empties: the last color is refused and stays
        // selected, so nearest-indexed resolution and the saved palette
        // section always hold at least one color.
        assert_eq!(tool_state.borrow().file_palette.len(), 1);
        assert_eq!(this.inner.borrow().selected_rgb(), Some(last));
    }

    #[test]
    fn an_edited_file_palette_flows_back_into_the_swatches() {
        let tool_state = Rc::new(RefCell::new(ToolState::default()));
        let mut this = module(&tool_state);
        let rgb = [0x12, 0x34, 0x56];
        tool_state.borrow_mut().add_file_palette_color(rgb);

        this.draw();

        let palette = this.inner.borrow().indexed_palette().to_vec();
        assert!(palette.contains(&rgb));
    }

    #[test]
    fn clicking_the_hex_field_opens_the_entry_seeded_with_the_hand_color() {
        let tool_state = Rc::new(RefCell::new(ToolState::default()));
        let entry: Rc<RefCell<Option<ColorEntryEdit>>> = Rc::new(RefCell::new(None));
        let mut this = module_with_entry(&tool_state, entry.clone());
        let hand_hex = {
            let state = tool_state.borrow();
            let rgb = state.hand_state(state.active_hand).color.preview_rgb();
            hex_color_string([rgb.0, rgb.1, rgb.2])
        };

        click(
            &mut this,
            rect().x0 + STRIP_HEX_START,
            rect().y0 + STRIP_HEX_ROW,
        );

        let edit = entry.borrow().clone().expect("edit opened");
        assert_eq!(edit.hex_text(), hand_hex.trim_start_matches('#'));
        assert_eq!(edit.commit().is_some(), true);
    }

    #[test]
    fn clicking_the_rgb_field_opens_the_entry_on_the_rgb_field() {
        let tool_state = Rc::new(RefCell::new(ToolState::default()));
        let entry: Rc<RefCell<Option<ColorEntryEdit>>> = Rc::new(RefCell::new(None));
        let mut this = module_with_entry(&tool_state, entry.clone());
        let hand_rgb = {
            let state = tool_state.borrow();
            let rgb = state.hand_state(state.active_hand).color.preview_rgb();
            [rgb.0, rgb.1, rgb.2]
        };

        click(&mut this, rect().x0 + STRIP_RGB_START, rect().y0 + STRIP_RGB_ROW);

        let edit = entry.borrow().clone().expect("edit opened");
        assert_eq!(edit.field, ColorEntryField::Rgb);
        assert_eq!(edit.rgb_text(), rgb_color_string(hand_rgb));
    }

    #[test]
    fn clicking_the_other_field_while_the_entry_is_open_switches_focus_without_resetting() {
        let tool_state = Rc::new(RefCell::new(ToolState::default()));
        let entry: Rc<RefCell<Option<ColorEntryEdit>>> = Rc::new(RefCell::new(None));
        let mut this = module_with_entry(&tool_state, entry.clone());

        click(
            &mut this,
            rect().x0 + STRIP_HEX_START,
            rect().y0 + STRIP_HEX_ROW,
        );
        {
            let mut edit = entry.borrow_mut();
            for ch in "112233".chars() {
                edit.as_mut().expect("edit open").push(ch);
            }
        }
        // Switch to the rgb field; the typed value stays.
        click(&mut this, rect().x0 + STRIP_RGB_START, rect().y0 + STRIP_RGB_ROW);

        let edit = entry.borrow().clone().expect("edit still open");
        assert_eq!(edit.field, ColorEntryField::Rgb);
        assert_eq!(edit.rgb_text(), "017 034 051");
    }

    #[test]
    fn a_non_strip_click_closes_an_open_entry() {
        let tool_state = Rc::new(RefCell::new(ToolState::default()));
        let entry: Rc<RefCell<Option<ColorEntryEdit>>> = Rc::new(RefCell::new(None));
        let mut this = module_with_entry(&tool_state, entry.clone());

        click(
            &mut this,
            rect().x0 + STRIP_HEX_START,
            rect().y0 + STRIP_HEX_ROW,
        );
        assert!(this.color_entry_is_open());
        // A swatch click (outside the strip rows) closes the entry.
        let position = this.inner.borrow().display_order()[0] as i32;
        click(
            &mut this,
            click_x_for_display_position(position),
            click_y_for_top_row(),
        );

        assert!(!this.color_entry_is_open());
    }

    #[test]
    fn the_strip_rows_draw_inside_the_module_bounds() {
        let tool_state = Rc::new(RefCell::new(ToolState::default()));
        let this = module(&tool_state);
        let hand = this.active_hand_rgb();

        let group = this.draw();
        // Hex row text begins at the '-' gizmo on row 2; rgb text on row 1;
        // row 0 is the single chrome bottom border the inner picker owns.
        let glyph_at = |x: i32, y: i32| group.cells[&CellPoint { x, y, z: 0 }].graphic.clone();
        assert_eq!(glyph_at(STRIP_MINUS, STRIP_HEX_ROW), CellGraphic::Glyph('-'));
        assert_eq!(glyph_at(STRIP_PLUS, STRIP_HEX_ROW), CellGraphic::Glyph('+'));
        let expected_rgb: Vec<char> = rgb_color_string(hand).chars().collect();
        for (index, offset) in (STRIP_RGB_START..=STRIP_RGB_END).enumerate() {
            assert_eq!(
                glyph_at(offset, STRIP_RGB_ROW),
                CellGraphic::Glyph(expected_rgb[index])
            );
        }
        assert_eq!(glyph_at(0, 0), CellGraphic::Glyph('┗'));
        assert_eq!(glyph_at(rect().x1 - rect().x0, 0), CellGraphic::Glyph('┛'));
    }
}
