//! 3D world clipboard for painter sessions: copy/paste payload shape, per-user
//! clipboard buffers for multiplayer, and the OS-clipboard text codec.
//!
//! Ownership (contract): this seam owns the in-app copy-buffer state, the
//! payload shape, and payload codecs. It does not own selection, the OS
//! clipboard transport (the caller reads/writes the actual clipboard), or the
//! document commit path — paste callers stage into the canvas and commit
//! through the same `session_document` stroke seam as brush/fill, so a paste
//! is one undo step like any stroke.
//!
//! 3D truth (source-of-truth from J): the clipboard must be world/3D — cells
//! carry full x/y/z offsets from an anchor, not a flat 2D grid. This mirrors
//! the old system's `world_selection.ts` `WorldCopyData` (anchor + dx/dy/dz
//! cells, `THAUM3D:` prefix on the OS clipboard) rather than the old 2D
//! `copy_paste.ts` grid.
//!
//! Multiplayer truth (source-of-truth from J): clipboards are user-based.
//! [`UserClipboards`] keys one buffer per `user_id` so each collaborator
//! copies and pastes independently; the OS-clipboard codec stays the shared
//! cross-session bridge.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thaum_renderer_domain::{CellGraphic, CellPoint};

use crate::brush::{is_blank_cell, Canvas, PaintedCell};
use crate::paint_color::PaintColor;
use crate::paint_color::PaintColorSlot;
use crate::selection_state::PainterSelection;

/// The OS-clipboard text prefix. Copy/paste payloads that start with this are
/// thaum 3D world copies; anything else is foreign text and not understood
/// here (plain-text paste is future work).
pub const OS_CLIPBOARD_PREFIX: &str = "THAUM3D:";

/// One user's copied world content: an anchor plus sparse cells offset from
/// that anchor. Offsets (not absolute positions) are what make a copy
/// pasteable anywhere; the anchor records where the copy was taken from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldCopyData {
    /// World point the copy was anchored at (selection bounds min on copy).
    pub anchor: CellPoint,
    /// Offset from `anchor` of the copy's center cell: the bounds center of
    /// the copied region, empty tiles included. `paste_points` places this
    /// offset at the paste cursor, so the cursor sits in the middle of the
    /// content instead of at its minimum corner.
    pub center: CellPoint,
    /// Sparse copied cells keyed by offset from `anchor`. Empty cells are not
    /// stored: pasting writes only copied cells and never erases.
    pub cells: BTreeMap<CellPoint, PaintedCell>,
}

impl Default for WorldCopyData {
    fn default() -> Self {
        Self {
            anchor: CellPoint { x: 0, y: 0, z: 0 },
            center: CellPoint { x: 0, y: 0, z: 0 },
            cells: BTreeMap::new(),
        }
    }
}

impl WorldCopyData {
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// Resolves the paste targets for this copy placed at `anchor`: absolute
    /// world points paired with their painted cells. The copy's center cell
    /// lands on `anchor`, so the paste cursor sits mid-content.
    pub fn paste_points(&self, anchor: CellPoint) -> Vec<(CellPoint, PaintedCell)> {
        self.cells
            .iter()
            // Authored blanks carry no content: a sparse paste never erases,
            // so a blank payload cell (legacy payload or hand-built copy)
            // places nothing.
            .filter(|(_, cell)| !is_blank_cell(cell))
            .map(|(offset, cell)| {
                (
                    CellPoint {
                        x: anchor.x + offset.x - self.center.x,
                        y: anchor.y + offset.y - self.center.y,
                        z: anchor.z + offset.z - self.center.z,
                    },
                    cell.clone(),
                )
            })
            .collect()
    }
}

/// Per-user clipboard buffers for multiplayer sessions: one [`WorldCopyData`]
/// per `user_id`. Live session state — cross-launch persistence rides the OS
/// clipboard codec, not this map.
#[derive(Debug, Clone, Default)]
pub struct UserClipboards {
    buffers: BTreeMap<String, WorldCopyData>,
}

impl UserClipboards {
    pub fn new() -> Self {
        Self::default()
    }

    /// Stores one user's copy, replacing their previous clipboard content.
    pub fn copy_for_user(&mut self, user_id: impl Into<String>, data: WorldCopyData) {
        self.buffers.insert(user_id.into(), data);
    }

    /// The user's current clipboard content, if they have copied anything.
    pub fn clipboard_for_user(&self, user_id: &str) -> Option<&WorldCopyData> {
        self.buffers.get(user_id)
    }

    /// Clears one user's clipboard without touching other users' buffers.
    pub fn clear_for_user(&mut self, user_id: &str) {
        self.buffers.remove(user_id);
    }

    /// Which users currently hold clipboard content, in stable key order.
    pub fn user_ids(&self) -> impl Iterator<Item = &str> {
        self.buffers.keys().map(String::as_str)
    }
}

/// Copies the selected region of `canvas` into world copy data. The world
/// selection wins when non-empty; otherwise the plane selection is copied.
/// The anchor is the selection's bounds minimum, the center is the bounds
/// center cell (empty tiles included; odd extents center exactly, even
/// extents round toward the min), and only painted cells are captured
/// (sparse copies paste without erasing).
pub fn copy_from_canvas(canvas: &Canvas, selection: &PainterSelection) -> Option<WorldCopyData> {
    let world = selection.world();
    let points: Vec<CellPoint> = if world.has_selection() {
        world.iter().collect()
    } else {
        selection.plane().iter().collect()
    };
    if points.is_empty() {
        return None;
    }

    let anchor = min_corner(&points);
    let center = center_offset(&points, anchor);
    let cells = points
        .into_iter()
        .filter_map(|point| {
            // Authored blanks are empty under the unified empty-cell rule:
            // copies never carry invisible colored spaces.
            canvas
                .get(&point)
                .filter(|cell| !is_blank_cell(cell))
                .cloned()
                .map(|cell| (offset_from(anchor, point), cell))
        })
        .collect();
    Some(WorldCopyData {
        anchor,
        center,
        cells,
    })
}

fn min_corner(points: &[CellPoint]) -> CellPoint {
    points.iter().fold(points[0], |acc, point| CellPoint {
        x: acc.x.min(point.x),
        y: acc.y.min(point.y),
        z: acc.z.min(point.z),
    })
}

/// The offset from `anchor` of the copied region's bounds-center cell,
/// empty tiles included. Per axis: `extent / 2`, so odd extents center
/// exactly and even extents round away from the min.
fn center_offset(points: &[CellPoint], anchor: CellPoint) -> CellPoint {
    let max_corner = points.iter().fold(points[0], |acc, point| CellPoint {
        x: acc.x.max(point.x),
        y: acc.y.max(point.y),
        z: acc.z.max(point.z),
    });
    CellPoint {
        x: (max_corner.x - anchor.x + 1) / 2,
        y: (max_corner.y - anchor.y + 1) / 2,
        z: (max_corner.z - anchor.z + 1) / 2,
    }
}

fn offset_from(anchor: CellPoint, point: CellPoint) -> CellPoint {
    CellPoint {
        x: point.x - anchor.x,
        y: point.y - anchor.y,
        z: point.z - anchor.z,
    }
}

/// Encodes world copy data for the user's actual OS clipboard as
/// `THAUM3D:` + JSON, so structures copied in one session (or by one user)
/// can be pasted in another — the user sees the structure emerge in whatever
/// session pastes it.
pub fn encode_os_clipboard(data: &WorldCopyData) -> String {
    let payload = ClipboardPayload::from_runtime(data);
    format!(
        "{OS_CLIPBOARD_PREFIX}{}",
        serde_json::to_string(&payload).expect("clipboard payload serializes")
    )
}

/// Decodes a thaum 3D world copy from OS-clipboard text. Returns `None` for
/// anything that is not a `THAUM3D:` payload (including plain text) so callers
/// can fall back or ignore.
pub fn decode_os_clipboard(text: &str) -> Option<WorldCopyData> {
    let payload_text = text.strip_prefix(OS_CLIPBOARD_PREFIX)?;
    let payload = serde_json::from_str::<ClipboardPayload>(payload_text).ok()?;
    Some(payload.into_runtime())
}

/// Serde payload mirror of [`WorldCopyData`]. Renderer and paint-color types
/// are not serde, so the codec owns its own persisted shape — same pattern as
/// `file/storage`'s persisted cell records.
#[derive(Debug, Serialize, Deserialize)]
struct ClipboardPayload {
    anchor: [i32; 3],
    /// Copy center offset; defaulted so clipboard payloads written before
    /// centering still decode (they paste min-corner at the cursor).
    #[serde(default)]
    center: [i32; 3],
    cells: Vec<ClipboardCellPayload>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ClipboardCellPayload {
    offset: [i32; 3],
    graphic: ClipboardGraphicPayload,
    color: ClipboardColorPayload,
    weight_index: i64,
    #[serde(default)]
    shader_stack: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
enum ClipboardGraphicPayload {
    None,
    Glyph(char),
    Sprite(String),
}

#[derive(Debug, Serialize, Deserialize)]
enum ClipboardColorPayload {
    FlatRgb {
        red: u8,
        green: u8,
        blue: u8,
    },
    Material {
        asset_file: String,
    },
    Slots {
        a: ClipboardColorSlotPayload,
        b: ClipboardColorSlotPayload,
        c: ClipboardColorSlotPayload,
    },
}

#[derive(Debug, Serialize, Deserialize)]
enum ClipboardColorSlotPayload {
    FlatRgb { red: u8, green: u8, blue: u8 },
    Material { asset_file: String },
}

fn clipboard_color_from_runtime(color: &PaintColor) -> ClipboardColorPayload {
    match color {
        PaintColor::FlatRgb(red, green, blue) => ClipboardColorPayload::FlatRgb {
            red: *red,
            green: *green,
            blue: *blue,
        },
        PaintColor::Material { asset_file } => ClipboardColorPayload::Material {
            asset_file: asset_file.clone(),
        },
        PaintColor::Slots { a, b, c } => ClipboardColorPayload::Slots {
            a: clipboard_slot_from_runtime(a),
            b: clipboard_slot_from_runtime(b),
            c: clipboard_slot_from_runtime(c),
        },
    }
}

fn clipboard_color_to_runtime(color: ClipboardColorPayload) -> PaintColor {
    match color {
        ClipboardColorPayload::FlatRgb { red, green, blue } => {
            PaintColor::flat_rgb(red, green, blue)
        }
        ClipboardColorPayload::Material { asset_file } => PaintColor::material_asset(asset_file),
        ClipboardColorPayload::Slots { a, b, c } => PaintColor::slots(
            clipboard_slot_to_runtime(a),
            clipboard_slot_to_runtime(b),
            clipboard_slot_to_runtime(c),
        ),
    }
}

fn clipboard_slot_from_runtime(slot: &PaintColorSlot) -> ClipboardColorSlotPayload {
    match slot {
        PaintColorSlot::FlatRgb(red, green, blue) => ClipboardColorSlotPayload::FlatRgb {
            red: *red,
            green: *green,
            blue: *blue,
        },
        PaintColorSlot::Material { asset_file } => ClipboardColorSlotPayload::Material {
            asset_file: asset_file.clone(),
        },
    }
}

fn clipboard_slot_to_runtime(slot: ClipboardColorSlotPayload) -> PaintColorSlot {
    match slot {
        ClipboardColorSlotPayload::FlatRgb { red, green, blue } => {
            PaintColor::flat_slot(red, green, blue)
        }
        ClipboardColorSlotPayload::Material { asset_file } => PaintColor::material_slot(asset_file),
    }
}

impl ClipboardPayload {
    fn from_runtime(data: &WorldCopyData) -> Self {
        Self {
            anchor: [data.anchor.x, data.anchor.y, data.anchor.z],
            center: [data.center.x, data.center.y, data.center.z],
            cells: data
                .cells
                .iter()
                .map(|(offset, cell)| ClipboardCellPayload {
                    offset: [offset.x, offset.y, offset.z],
                    graphic: match &cell.graphic {
                        CellGraphic::None => ClipboardGraphicPayload::None,
                        CellGraphic::Glyph(glyph) => ClipboardGraphicPayload::Glyph(*glyph),
                        CellGraphic::Sprite(sprite) => ClipboardGraphicPayload::Sprite(
                            sprite.atlas_relative_path().to_string_lossy().into_owned(),
                        ),
                    },
                    color: clipboard_color_from_runtime(&cell.color),
                    weight_index: cell.weight_index,
                    shader_stack: cell.shader_stack.clone(),
                })
                .collect(),
        }
    }

    fn into_runtime(self) -> WorldCopyData {
        WorldCopyData {
            anchor: point_from(self.anchor),
            center: point_from(self.center),
            cells: self
                .cells
                .into_iter()
                .filter_map(|cell| {
                    let graphic = match cell.graphic {
                        ClipboardGraphicPayload::None => CellGraphic::None,
                        ClipboardGraphicPayload::Glyph(glyph) => CellGraphic::Glyph(glyph),
                        ClipboardGraphicPayload::Sprite(path) => {
                            CellGraphic::Sprite(thaum_renderer_domain::SpriteGraphic::new(path))
                        }
                    };
                    Some((
                        point_from(cell.offset),
                        PaintedCell {
                            graphic,
                            color: clipboard_color_to_runtime(cell.color),
                            weight_index: cell.weight_index,
                            shader_stack: cell.shader_stack,
                        },
                    ))
                })
                .collect(),
        }
    }
}

fn point_from(value: [i32; 3]) -> CellPoint {
    CellPoint {
        x: value[0],
        y: value[1],
        z: value[2],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds() -> crate::fill::CanvasBounds {
        crate::fill::CanvasBounds {
            x0: -8,
            y0: -8,
            x1: 8,
            y1: 8,
            z: 0,
            plane_axis: crate::fill::CanvasPlaneAxis::Z,
        }
    }

    fn point(x: i32, y: i32, z: i32) -> CellPoint {
        CellPoint { x, y, z }
    }

    fn cell(glyph: char) -> PaintedCell {
        PaintedCell {
            graphic: CellGraphic::Glyph(glyph),
            color: PaintColor::flat_rgb(255, 255, 255),
            weight_index: 1,
            shader_stack: Vec::new(),
        }
    }

    fn selection_with(points: &[CellPoint]) -> PainterSelection {
        let mut selection = PainterSelection::new(bounds());
        for point in points {
            selection.world_mut().set_selected(*point, true);
        }
        selection
    }

    #[test]
    fn copy_captures_selected_cells_across_z_relative_to_the_min_corner() {
        let mut canvas = Canvas::new();
        canvas.insert(point(0, 0, 0), cell('A'));
        canvas.insert(point(1, 0, 0), cell('B'));
        canvas.insert(point(0, 0, -1), cell('C')); // different z layer
        let selection = selection_with(&[point(0, 0, 0), point(1, 0, 0), point(0, 0, -1)]);

        let data = copy_from_canvas(&canvas, &selection).expect("copy");

        assert_eq!(data.anchor, point(0, 0, -1));
        assert_eq!(data.cells.len(), 3);
        assert_eq!(
            data.cells.get(&point(0, 0, 0)).unwrap().graphic,
            CellGraphic::Glyph('C')
        );
        assert_eq!(
            data.cells.get(&point(1, 0, 1)).unwrap().graphic,
            CellGraphic::Glyph('B')
        );
    }

    #[test]
    fn copy_ignores_selected_but_unpainted_cells_and_prefers_the_world_selection() {
        let mut canvas = Canvas::new();
        canvas.insert(point(0, 0, 0), cell('A'));
        let mut selection = selection_with(&[point(0, 0, 0), point(2, 2, 2)]);
        // A plane selection exists too; the world selection must win.
        selection.apply_plane_points([point(5, 5, 5)]);

        let data = copy_from_canvas(&canvas, &selection).expect("copy");

        assert_eq!(data.cells.len(), 1);
        assert_eq!(data.anchor, point(0, 0, 0));
    }

    #[test]
    fn copy_with_no_selection_is_none() {
        let canvas = Canvas::new();
        let selection = selection_with(&[]);
        assert!(copy_from_canvas(&canvas, &selection).is_none());
    }

    #[test]
    fn paste_points_places_copies_relative_to_the_paste_anchor() {
        let data = WorldCopyData {
            anchor: point(0, 0, 0),
            cells: BTreeMap::from([(point(0, 0, 0), cell('A')), (point(1, 1, 1), cell('B'))]),
            ..WorldCopyData::default()
        };

        let points = data.paste_points(point(10, 20, -3));

        assert_eq!(points.len(), 2);
        assert!(points.contains(&(point(10, 20, -3), cell('A'))));
        assert!(points.contains(&(point(11, 21, -2), cell('B'))));
    }

    #[test]
    fn paste_points_lands_the_copy_center_on_the_cursor() {
        let data = WorldCopyData {
            anchor: point(0, 0, 0),
            // Center offset (1, 0): the (1,0) cell is the bounds-center of
            // the 2-wide copy, so the cursor lands mid-content.
            center: point(1, 0, 0),
            cells: BTreeMap::from([(point(0, 0, 0), cell('A')), (point(1, 0, 0), cell('B'))]),
        };

        let points = data.paste_points(point(10, 20, 0));

        assert!(points.contains(&(point(9, 20, 0), cell('A'))));
        assert!(points.contains(&(point(10, 20, 0), cell('B'))));
    }

    #[test]
    fn copy_centers_the_cursor_on_the_copied_bounds_including_empty_tiles() {
        // Selection spans three cells; only the ends are painted.
        let mut canvas = Canvas::new();
        canvas.insert(point(0, 0, 0), cell('A'));
        canvas.insert(point(2, 0, 0), cell('B'));
        let selection = selection_with(&[point(0, 0, 0), point(1, 0, 0), point(2, 0, 0)]);

        let data = copy_from_canvas(&canvas, &selection).expect("copy");

        // The empty middle tile counts toward the center: extent 3 → center
        // offset 1, so pasting at the cursor lands 'B' to the right of it.
        assert_eq!(data.center, point(1, 0, 0));
        let pasted = data.paste_points(point(5, 5, 0));
        assert!(pasted.contains(&(point(4, 5, 0), cell('A'))));
        assert!(pasted.contains(&(point(6, 5, 0), cell('B'))));
    }

    #[test]
    fn clipboards_are_isolated_per_user() {
        let mut clipboards = UserClipboards::new();
        let mut mine = WorldCopyData::default();
        mine.cells.insert(point(0, 0, 0), cell('M'));
        let mut theirs = WorldCopyData::default();
        theirs.cells.insert(point(0, 0, 0), cell('T'));

        clipboards.copy_for_user("j", mine);
        clipboards.copy_for_user("other", theirs);

        assert_eq!(
            clipboards
                .clipboard_for_user("j")
                .unwrap()
                .cells
                .get(&point(0, 0, 0))
                .unwrap()
                .graphic,
            CellGraphic::Glyph('M')
        );
        assert_eq!(
            clipboards
                .clipboard_for_user("other")
                .unwrap()
                .cells
                .get(&point(0, 0, 0))
                .unwrap()
                .graphic,
            CellGraphic::Glyph('T')
        );

        clipboards.clear_for_user("j");
        assert!(clipboards.clipboard_for_user("j").is_none());
        assert!(clipboards.clipboard_for_user("other").is_some());
        assert_eq!(clipboards.user_ids().collect::<Vec<_>>(), vec!["other"]);
    }

    #[test]
    fn copy_skips_authored_blanks_and_paste_ignores_blank_payload_cells() {
        let mut canvas = Canvas::new();
        canvas.insert(point(0, 0, 0), cell('A'));
        canvas.insert(
            point(1, 0, 0),
            PaintedCell {
                graphic: CellGraphic::Glyph(' '),
                color: PaintColor::flat_rgb(1, 2, 3),
                weight_index: 2,
                shader_stack: Vec::new(),
            },
        );
        let selection = selection_with(&[point(0, 0, 0), point(1, 0, 0)]);

        let data = copy_from_canvas(&canvas, &selection).expect("copy");

        // The stored blank is empty under the unified rule: never copied.
        assert_eq!(data.cells.len(), 1);
        assert_eq!(
            data.cells.get(&point(0, 0, 0)).unwrap().graphic,
            CellGraphic::Glyph('A')
        );

        // And a hand-built payload carrying a blank places nothing there.
        let mut legacy = WorldCopyData::default();
        legacy.cells.insert(
            point(0, 0, 0),
            PaintedCell {
                graphic: CellGraphic::Glyph(' '),
                color: PaintColor::flat_rgb(1, 2, 3),
                weight_index: 2,
                shader_stack: Vec::new(),
            },
        );
        legacy.cells.insert(point(1, 0, 0), cell('B'));
        assert_eq!(legacy.paste_points(point(5, 5, 0)).len(), 1);
    }

    #[test]
    fn os_clipboard_payload_roundtrips() {
        let mut data = WorldCopyData {
            anchor: point(-2, 3, 1),
            center: point(1, 1, 1),
            cells: BTreeMap::new(),
        };
        data.cells.insert(
            point(0, 0, 0),
            PaintedCell {
                graphic: CellGraphic::Sprite(thaum_renderer_domain::SpriteGraphic::new(
                    "cell-sprites/torch.png",
                )),
                color: PaintColor::slots(
                    PaintColor::material_slot("materials/fire.json"),
                    PaintColor::flat_slot(79, 157, 53),
                    PaintColor::material_slot("materials/smoke.json"),
                ),
                weight_index: 3,
                shader_stack: vec![
                    "cell-shaders/fire-low.json".to_string(),
                    "cell-shaders/weight-sin.json".to_string(),
                ],
            },
        );
        data.cells.insert(
            point(1, 0, 2),
            PaintedCell {
                graphic: CellGraphic::Glyph('x'),
                color: PaintColor::flat_rgb(79, 157, 53),
                weight_index: 0,
                shader_stack: Vec::new(),
            },
        );

        let encoded = encode_os_clipboard(&data);
        assert!(encoded.starts_with(OS_CLIPBOARD_PREFIX));
        assert_eq!(decode_os_clipboard(&encoded).unwrap(), data);
    }

    #[test]
    fn foreign_clipboard_text_is_not_understood() {
        assert!(decode_os_clipboard("just some ascii\nlines").is_none());
        assert!(decode_os_clipboard("THAUM3D:not json").is_none());
    }
}
