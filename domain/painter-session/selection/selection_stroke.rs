//! Live selection stroke accumulation and the renderer cell group that draws
//! the document-owned 3D selection (including in-progress stroke previews).

use std::collections::BTreeSet;

use thaum_renderer_domain::{
    Cell, CellColor, CellGraphic, CellGroup, CellPoint, CellWeight, WorldPoint,
    CELL_SHADER_VIVID_FLASH, CELL_SHADER_VIVID_FLASH_ALT,
};

use crate::brush::Canvas;
use crate::selection_state::{PainterSelection, SelectionMode};
use crate::tool_state::PaintHand;

#[derive(Debug, Clone)]
pub struct SelectionStroke {
    pub hand: PaintHand,
    pub mode: SelectionMode,
    pub points: BTreeSet<CellPoint>,
}

impl SelectionStroke {
    pub fn new(hand: PaintHand, mode: SelectionMode) -> Self {
        Self {
            hand,
            mode,
            points: BTreeSet::new(),
        }
    }

    pub fn extend<I>(&mut self, points: I)
    where
        I: IntoIterator<Item = CellPoint>,
    {
        self.points.extend(points);
    }
}

pub fn interpolate_cell_path(start: CellPoint, end: CellPoint) -> Vec<CellPoint> {
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    let dz = end.z - start.z;
    let steps = dx.abs().max(dy.abs()).max(dz.abs());
    if steps == 0 {
        return vec![start];
    }

    let mut points = Vec::with_capacity(steps as usize + 1);
    for step in 0..=steps {
        let t = step as f32 / steps as f32;
        points.push(CellPoint {
            x: start.x + (dx as f32 * t).round() as i32,
            y: start.y + (dy as f32 * t).round() as i32,
            z: start.z + (dz as f32 * t).round() as i32,
        });
    }
    points.dedup();
    points
}

/// The document-owned 3D selection overlay (including in-progress stroke
/// previews), built on the renderer's vivid flash shader pair: each selected
/// cell flashes between its character and weight recolored to the vivid UI
/// color and the exact appearance currently drawn. Empty cells flash between
/// the vivid and dim selection box so blank selected areas stay visible.
/// The shaders gate visibility only — both appearances are baked into the
/// overlay cells, one per phase.
pub fn build_plane_selection_cell_groups(
    selection: &PainterSelection,
    stroke: Option<&SelectionStroke>,
    canvas: &Canvas,
    vivid: CellColor,
) -> Vec<CellGroup> {
    let preview = match stroke {
        Some(stroke) => {
            selection.preview_plane_with_mode(stroke.points.iter().copied(), stroke.mode)
        }
        None => selection.plane().clone(),
    };

    let mut vivid_group = CellGroup::new(WorldPoint { x: 0, y: 0, z: 0 });
    let mut true_group = CellGroup::new(WorldPoint { x: 0, y: 0, z: 0 });
    // Render every selected cell, not just a plane slice or border: selection is a
    // 3D bitmap shared across depths, so cells light up at any depth the camera can
    // see (the composition already shows whatever falls inside the camera window).
    for position in preview.iter() {
        let existing = canvas.get(&position);
        vivid_group.insert(Cell {
            position,
            graphic: existing
                .map(|cell| cell.graphic.clone())
                .unwrap_or(CellGraphic::Glyph('□')),
            color: vivid,
            weight: CellWeight::from_index_clamped(
                existing.map_or(3, |cell| cell.weight_index as i32),
            ),
            shader_stack: vec![CELL_SHADER_VIVID_FLASH_ALT],
            ..Cell::default()
        });
        true_group.insert(Cell {
            position,
            graphic: existing
                .map(|cell| cell.graphic.clone())
                .unwrap_or(CellGraphic::Glyph('□')),
            color: existing
                .map(|cell| cell.color.to_cell_color())
                .unwrap_or(CellColor::Flat([0.8, 0.6, 0.1, 1.0])),
            weight: CellWeight::from_index_clamped(
                existing.map_or(3, |cell| cell.weight_index as i32),
            ),
            shader_stack: vec![CELL_SHADER_VIVID_FLASH],
            ..Cell::default()
        });
    }
    vec![vivid_group, true_group]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolate_cell_path_fills_every_step_between_two_points() {
        let points = interpolate_cell_path(
            CellPoint { x: 1, y: 1, z: 0 },
            CellPoint { x: 4, y: 4, z: 0 },
        );
        assert_eq!(
            points,
            vec![
                CellPoint { x: 1, y: 1, z: 0 },
                CellPoint { x: 2, y: 2, z: 0 },
                CellPoint { x: 3, y: 3, z: 0 },
                CellPoint { x: 4, y: 4, z: 0 },
            ]
        );
    }
}
