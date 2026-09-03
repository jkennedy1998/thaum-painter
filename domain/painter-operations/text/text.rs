//! Text layout over the voxel cell grid: one glyph char per cell, following the
//! old painter's text-entry semantics (`buildTextEntryCell` — typing sets the
//! cell's glyph char; the document's cells are the font).

use thaum_renderer_domain::{CameraViewOrientation, CellPoint};

/// Lays `text` out as one glyph char per cell starting at `origin`:
/// - chars advance along the view orientation's right axis,
/// - spaces advance without placing a cell,
/// - `\n` starts the next line one cell along the *opposite* of the view's up
///   axis (reading order: later lines sit below earlier ones on screen),
/// - tabs advance four cells, `\r` is skipped,
/// - the view's depth axis never moves, so text lies flat in the active view
///   plane at every swing and roll — same contract as `brush_points`.
///
/// Pure geometry: returns (point, char) pairs; the session bridge composes each
/// char with the active brush's color/weight into `PaintedCell`s, exactly as it
/// does for brush strokes.
pub fn text_cells(
    text: &str,
    origin: CellPoint,
    orientation: CameraViewOrientation,
) -> Vec<(CellPoint, char)> {
    text_cells_with_options(text, origin, orientation, DEFAULT_TEXT_LAYOUT_OPTIONS)
}

/// [`text_cells`] with explicit old-painter text tool properties: per-char
/// advance is `spacing` along right and `charlead` down the screen; Enter-style
/// newlines reset to column `enterspace` and step `enterlead` further down.
pub fn text_cells_with_options(
    text: &str,
    origin: CellPoint,
    orientation: CameraViewOrientation,
    options: TextLayoutOptions,
) -> Vec<(CellPoint, char)> {
    let options = options.clamped();
    let right = orientation.right.unit_vector();
    let up = orientation.up.unit_vector();
    let at = |col: i32, row: i32| CellPoint {
        x: origin.x + right[0] * col - up[0] * row,
        y: origin.y + right[1] * col - up[1] * row,
        z: origin.z + right[2] * col - up[2] * row,
    };

    let mut cells = Vec::new();
    let mut col = 0;
    let mut row = 0;
    for ch in text.chars() {
        match ch {
            '\n' => {
                row += options.enterlead;
                col = options.enterspace;
            }
            '\t' => col += TAB_WIDTH,
            '\r' => {}
            ' ' => col += options.spacing,
            glyph => {
                cells.push((at(col, row), glyph));
                col += options.spacing;
                row += options.charlead;
            }
        }
    }
    cells
}

pub const TAB_WIDTH: i32 = 4;

/// Old-painter text tool properties (`text_spacing`, `text_charlead`,
/// `text_enterlead`, `text_enterspace`), clamped to −16..16 like the old save
/// sanitizer. All values are in view-plane cells:
/// - `spacing`: horizontal (along right) advance per character,
/// - `charlead`: vertical advance per character (positive = down the screen),
/// - `enterlead`: vertical advance per Enter (positive = down, accumulates per
///   line: line N starts `enterlead * N` cells below the anchor),
/// - `enterspace`: horizontal offset applied to every line start after Enter.
/// Negative values reverse direction, so right-to-left and upward typing work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextLayoutOptions {
    pub spacing: i32,
    pub charlead: i32,
    pub enterlead: i32,
    pub enterspace: i32,
}

pub const DEFAULT_TEXT_LAYOUT_OPTIONS: TextLayoutOptions = TextLayoutOptions {
    spacing: 1,
    charlead: 0,
    enterlead: 1,
    enterspace: 0,
};

impl TextLayoutOptions {
    pub fn clamped(self) -> Self {
        let clamp = |v: i32| v.clamp(-16, 16);
        Self {
            spacing: clamp(self.spacing),
            charlead: clamp(self.charlead),
            enterlead: clamp(self.enterlead),
            enterspace: clamp(self.enterspace),
        }
    }
}

/// The cell a text cursor sits on, in view-plane coordinates relative to the
/// entry anchor: `col` along the view's right axis, `row` down the screen
/// (opposite the view's up axis). The depth axis never moves.
pub fn view_plane_point(
    origin: CellPoint,
    orientation: CameraViewOrientation,
    col: i32,
    row: i32,
) -> CellPoint {
    let right = orientation.right.unit_vector();
    let up = orientation.up.unit_vector();
    CellPoint {
        x: origin.x + right[0] * col - up[0] * row,
        y: origin.y + right[1] * col - up[1] * row,
        z: origin.z + right[2] * col - up[2] * row,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use thaum_renderer_domain::{camera_view_orientation_for_camera, CameraRoll, CameraSwing};

    fn point(x: i32, y: i32) -> CellPoint {
        CellPoint { x, y, z: 0 }
    }

    fn flat_view() -> CameraViewOrientation {
        camera_view_orientation_for_camera(CameraSwing::PosZ, CameraRoll::Deg0)
    }

    fn posx_view() -> CameraViewOrientation {
        camera_view_orientation_for_camera(CameraSwing::PosX, CameraRoll::Deg0)
    }

    #[test]
    fn chars_advance_along_the_view_right_axis_one_cell_per_glyph() {
        assert_eq!(
            text_cells("AB", point(2, 5), flat_view()),
            vec![(point(2, 5), 'A'), (point(3, 5), 'B')]
        );
    }

    #[test]
    fn newlines_stack_lines_below_earlier_ones() {
        assert_eq!(
            text_cells("AB\nC", point(2, 5), flat_view()),
            vec![
                (point(2, 5), 'A'),
                (point(3, 5), 'B'),
                // Second line sits one cell below the first (opposite of up).
                (point(2, 4), 'C'),
            ]
        );
    }

    #[test]
    fn spaces_advance_without_placing_cells() {
        assert_eq!(
            text_cells("A B", point(2, 5), flat_view()),
            vec![(point(2, 5), 'A'), (point(4, 5), 'B')]
        );
    }

    #[test]
    fn tabs_advance_four_cells() {
        assert_eq!(
            text_cells("A\tB", point(0, 0), flat_view()),
            vec![(point(0, 0), 'A'), (point(5, 0), 'B')]
        );
    }

    #[test]
    fn empty_text_places_nothing() {
        assert!(text_cells("", point(0, 0), flat_view()).is_empty());
        assert!(text_cells("  \n\t\r", point(0, 0), flat_view()).is_empty());
    }

    #[test]
    fn side_view_text_spreads_along_right_and_stacks_along_up_never_into_depth() {
        // At PosX the view plane is z/y and x is the depth axis: text must never
        // move x, mirroring the brush footprint contract.
        let cells = text_cells(
            "AB\nC",
            CellPoint { x: 3, y: 5, z: 7 },
            posx_view(),
        );
        assert_eq!(cells.len(), 3);
        assert!(cells.iter().all(|(p, _)| p.x == 3));
        let zs: Vec<_> = cells.iter().map(|(p, _)| p.z).collect();
        assert_eq!(zs, vec![7, 6, 7]);
        let ys: Vec<_> = cells.iter().map(|(p, _)| p.y).collect();
        assert_eq!(ys, vec![5, 5, 4]);
    }
}
