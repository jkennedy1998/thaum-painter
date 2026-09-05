//! Text layout over the voxel cell grid: one glyph char per cell, following
//! the old painter's text-entry semantics (`buildTextEntryCell` — typing sets
//! the cell's glyph char; the document's cells are the font).

use thaum_renderer_domain::{CameraViewOrientation, CellPoint};

/// Lays `text` out as one glyph char per cell starting at `origin`:
/// - chars advance by the char step in view-relative axes (default: along the
///   view orientation's right axis),
/// - spaces advance without placing a cell,
/// - `\n` steps the line start by the enter step (default: one cell along the
///   *opposite* of the view's up axis — reading order: later lines sit below
///   earlier ones on screen),
/// - tabs advance four cells, `\r` is skipped.
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

/// [`text_cells`] with explicit text tool properties: per-character advance is
/// `char_step` in (right, down, depth) view-relative cells; each Enter moves
/// the next line's start by `enter_step` from the previous line's start.
pub fn text_cells_with_options(
    text: &str,
    origin: CellPoint,
    orientation: CameraViewOrientation,
    options: TextLayoutOptions,
) -> Vec<(CellPoint, char)> {
    let options = options.clamped();
    let mut cells = Vec::new();
    let mut cursor = (0, 0, 0);
    let mut line = 0;
    for ch in text.chars() {
        match ch {
            '\n' => {
                line += 1;
                cursor = stepped_from_zero(options.enter_step, line);
            }
            '\t' => cursor.0 += TAB_WIDTH,
            '\r' => {}
            ' ' => cursor = stepped(cursor, options.char_step),
            glyph => {
                cells.push((view_plane_point(origin, orientation, cursor), glyph));
                cursor = stepped(cursor, options.char_step);
            }
        }
    }
    cells
}

fn stepped(cursor: (i32, i32, i32), step: (i32, i32, i32)) -> (i32, i32, i32) {
    (cursor.0 + step.0, cursor.1 + step.1, cursor.2 + step.2)
}

fn stepped_from_zero(step: (i32, i32, i32), line: i32) -> (i32, i32, i32) {
    (step.0 * line, step.1 * line, step.2 * line)
}

pub const TAB_WIDTH: i32 = 4;

/// Text tool spacing properties as two view-relative 3D steps, each axis
/// clamped to −9..9 (the properties panel's 2-char signed fields):
/// - `char_step`: cursor advance per character, (along right, down the screen,
///   into depth). Negative values reverse direction, so right-to-left,
///   upward, and into/out-of-screen typing all work.
/// - `enter_step`: line-start advance per Enter, same axes: line N starts
///   `enter_step * N` from the anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextLayoutOptions {
    pub char_step: (i32, i32, i32),
    pub enter_step: (i32, i32, i32),
}

pub const DEFAULT_TEXT_LAYOUT_OPTIONS: TextLayoutOptions = TextLayoutOptions {
    char_step: (1, 0, 0),
    enter_step: (0, 1, 0),
};

impl TextLayoutOptions {
    pub fn clamped(self) -> Self {
        let clamp = |v: i32| v.clamp(-9, 9);
        Self {
            char_step: (
                clamp(self.char_step.0),
                clamp(self.char_step.1),
                clamp(self.char_step.2),
            ),
            enter_step: (
                clamp(self.enter_step.0),
                clamp(self.enter_step.1),
                clamp(self.enter_step.2),
            ),
        }
    }
}

/// The cell a text cursor sits on, in view-relative cells from the entry
/// anchor: `col` along the view's right axis, `row` down the screen (opposite
/// the view's up axis), `depth` along the view's depth axis.
pub fn view_plane_point(
    origin: CellPoint,
    orientation: CameraViewOrientation,
    col_row_depth: (i32, i32, i32),
) -> CellPoint {
    let (col, row, depth) = col_row_depth;
    let right = orientation.right.unit_vector();
    let up = orientation.up.unit_vector();
    let depth_axis = orientation.depth.unit_vector();
    CellPoint {
        x: origin.x + right[0] * col - up[0] * row + depth_axis[0] * depth,
        y: origin.y + right[1] * col - up[1] * row + depth_axis[1] * depth,
        z: origin.z + right[2] * col - up[2] * row + depth_axis[2] * depth,
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
    fn char_step_moves_each_glyph_by_the_full_3d_step() {
        let cells = text_cells_with_options(
            "AB",
            point(2, 5),
            flat_view(),
            TextLayoutOptions {
                char_step: (2, 1, 3),
                ..DEFAULT_TEXT_LAYOUT_OPTIONS
            },
        );
        assert_eq!(
            cells,
            vec![
                (CellPoint { x: 2, y: 5, z: 0 }, 'A'),
                // right +2, down −1 (y decreases), depth +3 (south = +z at PosZ).
                (CellPoint { x: 4, y: 4, z: 3 }, 'B'),
            ]
        );
    }

    #[test]
    fn enter_step_accumulates_each_line_from_the_anchor() {
        let cells = text_cells_with_options(
            "A\nB\nC",
            point(0, 0),
            flat_view(),
            TextLayoutOptions {
                enter_step: (2, 1, 0),
                ..DEFAULT_TEXT_LAYOUT_OPTIONS
            },
        );
        assert_eq!(
            cells,
            vec![(point(0, 0), 'A'), (point(2, -1), 'B'), (point(4, -2), 'C'),]
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
        // At PosX the view plane is z/y and x is the depth axis: with the
        // default steps text must never move x, mirroring the brush footprint
        // contract (depth movement only happens via explicit step components).
        let cells = text_cells("AB\nC", CellPoint { x: 3, y: 5, z: 7 }, posx_view());
        assert_eq!(cells.len(), 3);
        assert!(cells.iter().all(|(p, _)| p.x == 3));
        let zs: Vec<_> = cells.iter().map(|(p, _)| p.z).collect();
        assert_eq!(zs, vec![7, 6, 7]);
        let ys: Vec<_> = cells.iter().map(|(p, _)| p.y).collect();
        assert_eq!(ys, vec![5, 5, 4]);
    }
}
