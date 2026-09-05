//! Live lasso stroke accumulation and its overlay preview cell groups.
//! A lasso drag records the freehand bound; release rasterizes it through
//! the pure `lasso` operation and applies it through the hand's tool state.

use thaum_renderer_domain::{
    Cell, CellColor, CellGraphic, CellGroup, CellPoint, CellWeight, WorldPoint,
    CELL_SHADER_VIVID_FLASH, CELL_SHADER_VIVID_FLASH_ALT,
};

use crate::brush::{is_blank_cell, PaintedCell};
use crate::tool_state::PaintHand;

/// One in-progress lasso bound: the acting hand plus the freehand path
/// recorded so far. Release turns the path into enclosed cells; the stroke
/// itself never paints.
#[derive(Debug, Clone)]
pub struct LassoStroke {
    pub hand: PaintHand,
    pub path: Vec<CellPoint>,
}

impl LassoStroke {
    pub fn new(hand: PaintHand, position: CellPoint) -> Self {
        Self {
            hand,
            path: vec![position],
        }
    }

    pub fn extend(&mut self, points: &[CellPoint]) {
        self.path.extend_from_slice(points);
    }
}

/// One previewed lasso cell: what the cell currently displays and what the
/// release commit will paint there. Both halves feed the flashing overlay.
#[derive(Debug, Clone)]
pub struct LassoPreviewCell {
    pub point: CellPoint,
    /// The cell as currently drawn (None where the canvas is empty).
    pub current: Option<PaintedCell>,
    /// The painted cell the release commit will produce at this point.
    pub upcoming: PaintedCell,
}

/// Overlay preview for an in-progress lasso: draws the bound path itself.
pub fn build_lasso_path_cell_group(stroke: &LassoStroke) -> CellGroup {
    let mut group = CellGroup::new(WorldPoint { x: 0, y: 0, z: 0 });
    for position in &stroke.path {
        group.insert(Cell {
            position: *position,
            graphic: CellGraphic::Glyph('◌'),
            color: CellColor::Flat([0.55, 0.75, 1.0, 1.0]),
            weight: CellWeight::from_index_clamped(3),
            ..Cell::default()
        });
    }
    group
}

/// Live interior preview for an in-progress lasso, built from the same seam
/// the release commit consumes. Two cell groups share the renderer's vivid
/// flash shader pair: the current cell's character and weight recolored to
/// the vivid UI color in one phase, the exact appearance release will paint
/// in the other. The shaders gate visibility only — both appearances are
/// fully baked into the overlay cells, one graphic override per phase.
pub fn build_lasso_preview_cell_groups(
    previews: &[LassoPreviewCell],
    vivid: CellColor,
) -> Vec<CellGroup> {
    let mut current_group = CellGroup::new(WorldPoint { x: 0, y: 0, z: 0 });
    let mut upcoming_group = CellGroup::new(WorldPoint { x: 0, y: 0, z: 0 });
    for preview in previews {
        // Unified empty-cell rule: an authored blank counts as empty. In the
        // as-it-is phase an empty cell shows J's void marker — a vivid '●' at
        // weight 0 — instead of an invisible space, so lasso/stamp coverage
        // over voids stays readable while the flash shows the canvas as-is.
        let current = preview
            .current
            .as_ref()
            .filter(|cell| !is_blank_cell(cell));
        current_group.insert(Cell {
            position: preview.point,
            graphic: current
                .map(|cell| cell.graphic.clone())
                .unwrap_or(CellGraphic::Glyph('●')),
            color: vivid,
            weight: CellWeight::from_index_clamped(current.map_or(0, |c| c.weight_index as i32)),
            shader_stack: vec![CELL_SHADER_VIVID_FLASH_ALT],
            ..Cell::default()
        });
        upcoming_group.insert(Cell {
            position: preview.point,
            graphic: preview.upcoming.graphic.clone(),
            color: preview.upcoming.color.to_cell_color(),
            weight: CellWeight::from_index_clamped(preview.upcoming.weight_index as i32),
            shader_stack: vec![CELL_SHADER_VIVID_FLASH],
            ..Cell::default()
        });
    }
    vec![current_group, upcoming_group]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paint_color::PaintColor;

    fn point(x: i32, y: i32) -> CellPoint {
        CellPoint { x, y, z: 0 }
    }

    fn painted(graphic: char, rgb: (u8, u8, u8)) -> PaintedCell {
        PaintedCell {
            graphic: CellGraphic::Glyph(graphic),
            color: PaintColor::FlatRgb(rgb.0, rgb.1, rgb.2),
            weight_index: 2,
        }
    }

    fn vivid() -> CellColor {
        CellColor::Flat([0.5, 1.0, 0.75, 1.0])
    }

    #[test]
    fn a_lasso_stroke_starts_at_its_press_cell() {
        let stroke = LassoStroke::new(PaintHand::Left, point(2, 3));
        assert_eq!(stroke.hand, PaintHand::Left);
        assert_eq!(stroke.path, vec![point(2, 3)]);
    }

    #[test]
    fn extending_appends_the_interpolated_drag_points() {
        let mut stroke = LassoStroke::new(PaintHand::Right, point(0, 0));
        stroke.extend(&[point(1, 0), point(2, 0)]);
        assert_eq!(stroke.path, vec![point(0, 0), point(1, 0), point(2, 0)]);
    }

    #[test]
    fn the_preview_flashes_current_in_vivid_against_the_upcoming_paint() {
        let previews = vec![LassoPreviewCell {
            point: point(1, 1),
            current: Some(painted('#', (255, 255, 255))),
            upcoming: painted('.', (9, 8, 7)),
        }];
        let groups = build_lasso_preview_cell_groups(&previews, vivid());

        // Current half: the drawn character and weight, recolored vivid,
        // shown only in the off phase (ALT shader).
        let current: Vec<&Cell> = groups[0].iter_cells().collect();
        assert_eq!(current.len(), 1);
        assert_eq!(current[0].graphic, CellGraphic::Glyph('#'));
        assert_eq!(current[0].color, vivid());
        assert_eq!(current[0].weight, CellWeight::from_index_clamped(2));
        assert_eq!(current[0].shader_stack, vec![CELL_SHADER_VIVID_FLASH_ALT]);

        // Upcoming half: the exact painted appearance release produces,
        // shown only in the lit phase (FLASH shader).
        let upcoming: Vec<&Cell> = groups[1].iter_cells().collect();
        assert_eq!(upcoming.len(), 1);
        assert_eq!(upcoming[0].graphic, CellGraphic::Glyph('.'));
        assert_eq!(upcoming[0].color, PaintColor::flat_rgb(9, 8, 7).to_cell_color());
        assert_eq!(upcoming[0].shader_stack, vec![CELL_SHADER_VIVID_FLASH]);
    }

    #[test]
    fn an_empty_canvas_cell_flashes_a_vivid_void_marker_against_the_upcoming_paint() {
        let previews = vec![LassoPreviewCell {
            point: point(0, 0),
            current: None,
            upcoming: painted('A', (1, 2, 3)),
        }];
        let groups = build_lasso_preview_cell_groups(&previews, vivid());
        let current: Vec<&Cell> = groups[0].iter_cells().collect();
        // Void marker: vivid '●' at weight 0 in the as-it-is phase.
        assert_eq!(current[0].graphic, CellGraphic::Glyph('●'));
        assert_eq!(current[0].color, vivid());
        assert_eq!(current[0].weight, CellWeight::from_index_clamped(0));
        let upcoming: Vec<&Cell> = groups[1].iter_cells().collect();
        assert_eq!(upcoming[0].graphic, CellGraphic::Glyph('A'));
    }

    #[test]
    fn a_stored_blank_flashes_as_a_void_marker_in_the_as_is_phase() {
        let previews = vec![LassoPreviewCell {
            point: point(0, 0),
            current: Some(PaintedCell {
                graphic: CellGraphic::Glyph(' '),
                color: PaintColor::flat_rgb(1, 2, 3),
                weight_index: 2,
            }),
            upcoming: painted('A', (1, 2, 3)),
        }];
        let groups = build_lasso_preview_cell_groups(&previews, vivid());
        let current: Vec<&Cell> = groups[0].iter_cells().collect();
        assert_eq!(current[0].graphic, CellGraphic::Glyph('●'));
        assert_eq!(current[0].weight, CellWeight::from_index_clamped(0));
    }

    #[test]
    fn an_empty_preview_region_builds_two_empty_groups() {
        let groups = build_lasso_preview_cell_groups(&[], vivid());
        assert_eq!(groups.len(), 2);
        assert!(groups.iter().all(|g| g.iter_cells().next().is_none()));
    }
}
