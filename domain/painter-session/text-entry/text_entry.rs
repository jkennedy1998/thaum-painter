//! Stateful text-entry session: the live typing mode behind the future text
//! tool, ported from the old painter's `text_mode_active` editor. Typing is a
//! STATE, not a batch stamp: while active, the cursor owns the keyboard (the
//! entrypoint must route keys here BEFORE shortcut dispatch and suppress
//! everything except the reserved camera/depth bindings), each keystroke
//! applies one cell change live, and pending changes commit as ONE undo-able
//! 'Type Text' action at Enter/exit — the old commit granularity.

use thaum_renderer_domain::{
    CameraViewOrientation, Cell, CellColor, CellGraphic, CellGroup, CellPoint, CellWeight,
    WorldPoint,
};

use crate::brush::PaintedCell;
use crate::text::{view_plane_point, TextLayoutOptions};

/// The keys a text-entry session consumes. The entrypoint translates physical
/// keys into these; everything else during typing is either a reserved binding
/// (camera/depth, owned by the entrypoint) or suppressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextEntryKey {
    Char(char),
    Space,
    Enter,
    Escape,
    Backspace,
    Delete,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    ArrowDown,
}

/// What one key did. `Applied` changes must be staged onto the live canvas by
/// the caller immediately (the old tool painted per keystroke).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextEntryOutcome {
    /// Typing is not active — the caller should not have asked.
    Idle,
    /// Active, key not consumed — reserved bindings may run, anything else is
    /// suppressed while typing.
    Ignored,
    /// One live cell change to stage (insert or erase) at the cursor.
    Applied {
        point: CellPoint,
        cell: Option<PaintedCell>,
    },
    /// Enter committed the pending segment; typing continues on the next line.
    /// Caller commits pending changes as one record, then keeps the session.
    Committed,
    /// Escape/exit: caller commits pending changes as one record and ends the
    /// session.
    Finished,
}

/// One pending cell change: `None` = the cell was erased/cleared.
pub type PendingChange = (CellPoint, Option<PaintedCell>);

pub struct TextEntryState {
    active: bool,
    origin: CellPoint,
    orientation: CameraViewOrientation,
    options: TextLayoutOptions,
    space_replace: bool,
    brush: PaintedCell,
    /// Cursor in view-relative cells: (along right, down the screen, into
    /// depth), from the anchor.
    cursor: (i32, i32, i32),
    /// Current line index (Enter accumulates `enter_step * line` from the
    /// anchor).
    line: i32,
    /// Per-line end cursors for Backspace's wrap to the previous line.
    line_ends: Vec<(i32, i32, i32)>,
    pending: Vec<PendingChange>,
}

impl TextEntryState {
    /// Begins a typing session anchored at `origin` with the click's brush
    /// captured (the old tool captured `getBrushForButton(text_mode_button)` at
    /// pointer-down, so later brush changes did not affect the open session).
    pub fn begin(
        origin: CellPoint,
        orientation: CameraViewOrientation,
        options: TextLayoutOptions,
        space_replace: bool,
        brush: PaintedCell,
    ) -> Self {
        Self {
            active: true,
            origin,
            orientation,
            options: options.clamped(),
            space_replace,
            brush,
            cursor: (0, 0, 0),
            line: 0,
            line_ends: vec![(0, 0, 0)],
            pending: Vec::new(),
        }
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Re-anchors the session to the live camera view. Typing is
    /// camera-relative: arrows, character advance, and Enter all resolve
    /// through the current orientation, so a mid-session camera swing keeps
    /// the cursor moving the way the screen points.
    pub fn set_orientation(&mut self, orientation: CameraViewOrientation) {
        self.orientation = orientation;
    }

    pub fn cursor_point(&self) -> CellPoint {
        view_plane_point(self.origin, self.orientation, self.cursor)
    }

    pub fn pending_changes(&self) -> &[PendingChange] {
        &self.pending
    }

    /// Takes (and clears) the pending changes for commit. The caller hands them
    /// to the session bridge as one 'Type Text' action.
    pub fn take_pending(&mut self) -> Vec<PendingChange> {
        std::mem::take(&mut self.pending)
    }

    fn line_start(&self, line: i32) -> (i32, i32, i32) {
        let step = self.options.enter_step;
        (step.0 * line, step.1 * line, step.2 * line)
    }

    pub fn handle_key(&mut self, key: TextEntryKey) -> TextEntryOutcome {
        if !self.active {
            return TextEntryOutcome::Idle;
        }
        match key {
            TextEntryKey::Escape => {
                self.active = false;
                TextEntryOutcome::Finished
            }
            TextEntryKey::Enter => {
                let start = self.line_start(self.line);
                self.line_ends[self.line as usize] = self.cursor;
                self.line += 1;
                self.cursor = self.line_start(self.line);
                self.line_ends.push(self.cursor);
                let _ = start;
                TextEntryOutcome::Committed
            }
            TextEntryKey::Char(glyph) => {
                let point = self.cursor_point();
                let mut cell = self.brush.clone();
                cell.graphic = CellGraphic::Glyph(glyph);
                self.pending.push((point, Some(cell)));
                self.advance_cursor();
                TextEntryOutcome::Applied {
                    point,
                    cell: self.pending.last().and_then(|(_, c)| c.clone()),
                }
            }
            TextEntryKey::Space => {
                if self.space_replace {
                    let point = self.cursor_point();
                    self.pending.push((point, None));
                    self.advance_cursor();
                    TextEntryOutcome::Applied { point, cell: None }
                } else {
                    self.advance_cursor();
                    TextEntryOutcome::Ignored
                }
            }
            TextEntryKey::Backspace => {
                let line_start = self.line_start(self.line);
                let at_line_start = self.cursor == line_start;
                if at_line_start && self.line > 0 {
                    self.line -= 1;
                    self.cursor = self.line_ends[self.line as usize];
                } else if !at_line_start {
                    let step = self.options.char_step;
                    self.cursor = (
                        self.cursor.0 - step.0,
                        self.cursor.1 - step.1,
                        self.cursor.2 - step.2,
                    );
                } else {
                    return TextEntryOutcome::Ignored;
                }
                let point = self.cursor_point();
                self.pending.push((point, None));
                TextEntryOutcome::Applied { point, cell: None }
            }
            TextEntryKey::Delete => {
                let point = self.cursor_point();
                self.pending.push((point, None));
                TextEntryOutcome::Applied { point, cell: None }
            }
            TextEntryKey::ArrowLeft => {
                self.cursor.0 -= 1;
                TextEntryOutcome::Ignored
            }
            TextEntryKey::ArrowRight => {
                self.cursor.0 += 1;
                TextEntryOutcome::Ignored
            }
            TextEntryKey::ArrowUp => {
                self.cursor.1 -= 1;
                TextEntryOutcome::Ignored
            }
            TextEntryKey::ArrowDown => {
                self.cursor.1 += 1;
                TextEntryOutcome::Ignored
            }
        }
    }

    fn advance_cursor(&mut self) {
        let step = self.options.char_step;
        self.cursor = (
            self.cursor.0 + step.0,
            self.cursor.1 + step.1,
            self.cursor.2 + step.2,
        );
        self.line_ends[self.line as usize] = self.cursor;
    }
}

/// A rendered overlay marking the cell about to receive the next character.
/// Purely visual: the entrypoint composes this group on top of the document
/// each frame and never stages it, so the flashing cursor is never part of the
/// drawing.
/// The cursor overlay is always present while typing; the blink phase swaps
/// the glyph between a filled block and an outline so the cursor cell stays
/// visible at every moment.
pub fn cursor_overlay_group(point: CellPoint, glyph: char) -> CellGroup {
    let mut group = CellGroup::new(WorldPoint { x: 0, y: 0, z: 0 });
    group.insert(Cell {
        position: point,
        graphic: CellGraphic::Glyph(glyph),
        color: CellColor::Flat([1.0, 0.9, 0.25, 1.0]),
        weight: CellWeight::from_index_clamped(3),
        ..Cell::default()
    });
    group
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paint_color::PaintColor;
    use thaum_renderer_domain::CellGraphic;
    use thaum_renderer_domain::{camera_view_orientation_for_camera, CameraRoll, CameraSwing};

    fn flat_view() -> CameraViewOrientation {
        camera_view_orientation_for_camera(CameraSwing::PosZ, CameraRoll::Deg0)
    }

    fn brush(glyph: char) -> PaintedCell {
        PaintedCell {
            graphic: CellGraphic::Glyph(glyph),
            color: PaintColor::flat_rgb(255, 255, 255),
            weight_index: 1,
        }
    }

    fn point(x: i32, y: i32) -> CellPoint {
        CellPoint { x, y, z: 0 }
    }

    fn typing() -> TextEntryState {
        TextEntryState::begin(
            point(5, 5),
            flat_view(),
            crate::text::DEFAULT_TEXT_LAYOUT_OPTIONS,
            true,
            brush('?'),
        )
    }

    #[test]
    fn chars_apply_at_the_cursor_and_advance_with_the_char_step() {
        let mut state = TextEntryState::begin(
            point(5, 5),
            flat_view(),
            TextLayoutOptions {
                char_step: (2, 1, 0),
                ..crate::text::DEFAULT_TEXT_LAYOUT_OPTIONS
            },
            true,
            brush('X'),
        );
        assert_eq!(
            state.handle_key(TextEntryKey::Char('A')),
            TextEntryOutcome::Applied {
                point: point(5, 5),
                cell: Some(brush('A'))
            }
        );
        assert_eq!(
            state.handle_key(TextEntryKey::Char('B')),
            TextEntryOutcome::Applied {
                point: point(7, 4),
                cell: Some(brush('B'))
            }
        );
        assert_eq!(state.pending_changes().len(), 2);
    }

    #[test]
    fn enter_commits_and_starts_the_next_line_using_the_enter_step() {
        let mut state = TextEntryState::begin(
            point(5, 5),
            flat_view(),
            TextLayoutOptions {
                enter_step: (3, 2, 0),
                ..crate::text::DEFAULT_TEXT_LAYOUT_OPTIONS
            },
            true,
            brush('?'),
        );
        state.handle_key(TextEntryKey::Char('A'));
        assert_eq!(
            state.handle_key(TextEntryKey::Enter),
            TextEntryOutcome::Committed
        );
        assert_eq!(state.cursor_point(), point(8, 3));
        // Cursor on the new line writes there.
        assert_eq!(
            state.handle_key(TextEntryKey::Char('B')),
            TextEntryOutcome::Applied {
                point: point(8, 3),
                cell: Some(brush('B'))
            }
        );
    }

    #[test]
    fn space_advances_and_clears_the_cell_only_when_space_replace_is_on() {
        let mut state = typing();
        state.handle_key(TextEntryKey::Char('A'));
        assert_eq!(
            state.handle_key(TextEntryKey::Space),
            TextEntryOutcome::Applied {
                point: point(6, 5),
                cell: None
            }
        );
        let mut keep = TextEntryState::begin(
            point(5, 5),
            flat_view(),
            crate::text::DEFAULT_TEXT_LAYOUT_OPTIONS,
            false,
            brush('?'),
        );
        keep.handle_key(TextEntryKey::Char('A'));
        assert_eq!(
            keep.handle_key(TextEntryKey::Space),
            TextEntryOutcome::Ignored
        );
        assert_eq!(keep.cursor_point(), point(7, 5));
    }

    #[test]
    fn backspace_steps_back_erasing_and_wraps_to_the_previous_line_end() {
        let mut state = typing();
        state.handle_key(TextEntryKey::Char('A'));
        state.handle_key(TextEntryKey::Char('B'));
        // Steps back one char cell (char_step 1) and erases it.
        assert_eq!(
            state.handle_key(TextEntryKey::Backspace),
            TextEntryOutcome::Applied {
                point: point(6, 5),
                cell: None
            }
        );
        // Enter, then backspace at the new line start wraps to the previous
        // line's end and erases there.
        state.handle_key(TextEntryKey::Enter);
        assert_eq!(
            state.handle_key(TextEntryKey::Backspace),
            TextEntryOutcome::Applied {
                point: point(6, 5),
                cell: None
            }
        );
        // Backspace at the very anchor is a no-op.
        state.handle_key(TextEntryKey::Backspace);
        state.handle_key(TextEntryKey::Backspace);
        assert_eq!(
            state.handle_key(TextEntryKey::Backspace),
            TextEntryOutcome::Ignored
        );
    }

    #[test]
    fn arrows_move_the_cursor_freely_one_cell_at_a_time() {
        let mut state = typing();
        state.handle_key(TextEntryKey::ArrowRight);
        state.handle_key(TextEntryKey::ArrowDown);
        // "Down the screen" is opposite the view's up axis (−y at PosZ).
        assert_eq!(state.cursor_point(), point(6, 4));
        state.handle_key(TextEntryKey::ArrowUp);
        state.handle_key(TextEntryKey::ArrowLeft);
        assert_eq!(state.cursor_point(), point(5, 5));
    }

    #[test]
    fn a_mid_session_camera_swing_rebases_arrows_and_typing_to_the_new_view() {
        let mut state = typing();
        state.handle_key(TextEntryKey::Char('A'));
        // Swing to PosX mid-session: the session re-bases through
        // `set_orientation`, so "right" is now screen-right at PosX.
        state.set_orientation(camera_view_orientation_for_camera(
            CameraSwing::PosX,
            CameraRoll::Deg0,
        ));
        state.handle_key(TextEntryKey::ArrowRight);
        // At PosX the view plane is z/y (x is depth): right steps along −z,
        // and typed cells keep x anchored.
        assert_eq!(state.cursor_point(), CellPoint { x: 5, y: 5, z: -2 });
        state.handle_key(TextEntryKey::ArrowUp);
        assert_eq!(state.cursor_point(), CellPoint { x: 5, y: 6, z: -2 });
        assert_eq!(
            state.handle_key(TextEntryKey::Char('B')),
            TextEntryOutcome::Applied {
                point: CellPoint { x: 5, y: 6, z: -2 },
                cell: Some(brush('B')),
            }
        );
    }

    #[test]
    fn char_step_depth_component_moves_typing_through_depth_and_backspace_reverses_it() {
        let mut state = TextEntryState::begin(
            point(0, 0),
            flat_view(),
            TextLayoutOptions {
                char_step: (1, 0, 2),
                ..crate::text::DEFAULT_TEXT_LAYOUT_OPTIONS
            },
            true,
            brush('?'),
        );
        state.handle_key(TextEntryKey::Char('A'));
        // Depth +2 moves south (+z at PosZ).
        assert_eq!(state.cursor_point(), CellPoint { x: 1, y: 0, z: 2 });
        // Backspace undoes the full 3D step.
        state.handle_key(TextEntryKey::Backspace);
        assert_eq!(state.cursor_point(), point(0, 0));
    }

    #[test]
    fn escape_finishes_and_later_keys_are_idle() {
        let mut state = typing();
        state.handle_key(TextEntryKey::Char('A'));
        assert_eq!(
            state.handle_key(TextEntryKey::Escape),
            TextEntryOutcome::Finished
        );
        assert!(!state.is_active());
        assert_eq!(
            state.handle_key(TextEntryKey::Char('B')),
            TextEntryOutcome::Idle
        );
        // Pending changes survive for the caller's final commit.
        assert_eq!(state.pending_changes().len(), 1);
    }
}
