use anyhow::{Context, Result};
use thaum_renderer_domain::{
    Camera, Cell, CellColor, CellGraphic, CellGroup, CellGroupIntakeBehavior, CellPoint,
    CellWeight, Composition, DataLanes, WorldPoint,
};

use crate::manifest::{parse_grid_point, GridPoint, Group, Manifest, Module, RasterSegment, Rgb};

/// The transient renderer handoff assembled from one manifest at one active breath.
///
/// `camera` is a fully resolved `thaum-renderer` camera, expected to come from
/// `domain/rendering/camera/` (app camera intent + saved defaults), not from this seam.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderSpace {
    pub camera: Camera,
    pub composition: Composition,
    pub data_lanes: DataLanes,
}

fn world_point_from_grid(point: GridPoint) -> WorldPoint {
    WorldPoint {
        x: point.x as i32,
        y: point.y as i32,
        z: point.z as i32,
    }
}

fn cell_point_from_grid(point: GridPoint) -> CellPoint {
    CellPoint {
        x: point.x as i32,
        y: point.y as i32,
        z: point.z as i32,
    }
}

fn add_grid_points(a: GridPoint, b: GridPoint) -> GridPoint {
    GridPoint {
        x: a.x + b.x,
        y: a.y + b.y,
        z: a.z + b.z,
    }
}

fn rgb_to_cell_color(rgb: Rgb) -> CellColor {
    CellColor::Flat([
        rgb.r as f32 / 255.0,
        rgb.g as f32 / 255.0,
        rgb.b as f32 / 255.0,
        1.0,
    ])
}

fn breath_in_window(breath: u32, start: u32, end: u32) -> bool {
    breath >= start && breath <= end
}

fn active_raster_segment(group: &Group, active_breath: u32) -> Option<&RasterSegment> {
    group
        .raster_segments
        .iter()
        .find(|segment| breath_in_window(active_breath, segment.start_breath, segment.end_breath))
}

/// The active `move` property block's offset at `active_breath`, or the zero offset
/// if the group has no `move` property or no block covers this breath.
fn active_move_offset(group: &Group, active_breath: u32) -> Result<GridPoint> {
    let Some(move_property) = group.properties.iter().find(|property| property.kind == "move")
    else {
        return Ok(GridPoint::default());
    };
    let Some(block) = move_property
        .blocks
        .iter()
        .find(|block| breath_in_window(active_breath, block.start_breath, block.end_breath))
    else {
        return Ok(GridPoint::default());
    };
    parse_grid_point(&block.value)
        .with_context(|| format!("move property block '{}' value must be an {{x,y,z}} offset", block.id))
}

fn composite_group_cells(group: &Group, active_breath: u32, cell_group: &mut CellGroup) -> Result<()> {
    if !group.visible {
        return Ok(());
    }
    let Some(segment) = active_raster_segment(group, active_breath) else {
        return Ok(());
    };
    let move_offset = active_move_offset(group, active_breath)
        .with_context(|| format!("group '{}' has an invalid move offset", group.id))?;
    let effective_local_placement = add_grid_points(group.local_placement, move_offset);

    for voxel in &segment.voxels {
        let position = cell_point_from_grid(add_grid_points(effective_local_placement, voxel.position));
        cell_group.insert(Cell {
            position,
            graphic: CellGraphic::Glyph(voxel.char),
            color: rgb_to_cell_color(voxel.rgb),
            weight: CellWeight::from_index_clamped(voxel.weight_index as i32),
            ..Cell::default()
        });
    }
    Ok(())
}

/// Composites one authored module's intra-module groups, in `group_order`, into the
/// single `CellGroup` the renderer sees for that module.
fn build_module_cell_group(module: &Module, active_breath: u32) -> Result<CellGroup> {
    let mut cell_group = CellGroup::new(world_point_from_grid(module.placement))
        .with_intake_behavior(CellGroupIntakeBehavior::Flat2d);

    if !module.visible {
        return Ok(cell_group);
    }

    for group_id in &module.group_order {
        let group = module
            .groups
            .iter()
            .find(|group| &group.id == group_id)
            .with_context(|| format!("module '{}' group_order references missing group '{group_id}'", module.id))?;
        composite_group_cells(group, active_breath, &mut cell_group)
            .with_context(|| format!("module '{}' failed to composite group '{group_id}'", module.id))?;
    }

    Ok(cell_group)
}

/// One renderer cell-group per authored module, in `document.module_order`.
pub fn build_composition(manifest: &Manifest, active_breath: u32) -> Result<Composition> {
    let mut groups = Vec::new();
    for module_id in &manifest.document.module_order {
        let module = manifest
            .document
            .modules
            .iter()
            .find(|module| &module.id == module_id)
            .with_context(|| format!("document module_order references missing module '{module_id}'"))?;
        let cell_group = build_module_cell_group(module, active_breath)
            .with_context(|| format!("failed to composite module '{module_id}'"))?;
        groups.push(cell_group);
    }
    Ok(Composition::ordered(groups))
}

pub fn build_data_lanes(active_breath: u32) -> DataLanes {
    DataLanes::with_breath(active_breath as i32)
}

/// Assembles the render-space handoff for one manifest at one active breath.
///
/// `camera` should already be resolved (saved defaults + live overrides) by
/// `domain/rendering/camera/`; this seam only normalizes it into the handoff shape.
pub fn build_render_space(manifest: &Manifest, active_breath: u32, camera: Camera) -> Result<RenderSpace> {
    Ok(RenderSpace {
        camera,
        composition: build_composition(manifest, active_breath)?,
        data_lanes: build_data_lanes(active_breath),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::parse_manifest_from_str;

    const EXAMPLE_MANIFEST_JSON: &str = include_str!("../../file/manifest/example-thaum-painter-file-v1.json");

    fn example_manifest() -> Manifest {
        parse_manifest_from_str(EXAMPLE_MANIFEST_JSON).unwrap()
    }

    #[test]
    fn one_cell_group_is_emitted_per_module_in_module_order() {
        let manifest = example_manifest();
        let composition = build_composition(&manifest, 4).unwrap();

        assert_eq!(composition.groups.len(), 2);
        assert_eq!(composition.groups[0].origin, WorldPoint { x: 0, y: 0, z: 0 });
        assert_eq!(composition.groups[1].origin, WorldPoint { x: 18, y: 0, z: 0 });
    }

    #[test]
    fn intra_module_groups_composite_into_the_owning_modules_single_cell_group() {
        let manifest = example_manifest();
        let composition = build_composition(&manifest, 4).unwrap();
        let sign = &composition.groups[0];

        assert_eq!(
            sign.get(CellPoint { x: 0, y: 0, z: 0 }).unwrap().graphic,
            CellGraphic::Glyph('.')
        );
        assert_eq!(
            sign.get(CellPoint { x: 3, y: 2, z: 1 }).unwrap().graphic,
            CellGraphic::Glyph('R')
        );
        assert_eq!(
            sign.get(CellPoint { x: 4, y: 2, z: 1 }).unwrap().graphic,
            CellGraphic::Glyph('U')
        );
        assert_eq!(
            sign.get(CellPoint { x: 5, y: 2, z: 1 }).unwrap().graphic,
            CellGraphic::Glyph('N')
        );
    }

    #[test]
    fn breath_resolution_picks_the_segment_and_move_offset_active_at_the_chosen_breath() {
        let manifest = example_manifest();

        // at breath 2: dim glow segment, zero move offset
        let dim = build_composition(&manifest, 2).unwrap();
        let dim_cell = dim.groups[0].get(CellPoint { x: 2, y: 1, z: 2 }).unwrap();
        assert_eq!(dim_cell.graphic, CellGraphic::Glyph('░'));

        // at breath 4: bright glow segment, shifted by the active move block's +1 x offset
        let bright = build_composition(&manifest, 4).unwrap();
        assert!(bright.groups[0].get(CellPoint { x: 2, y: 1, z: 2 }).is_none());
        let bright_cell = bright.groups[0].get(CellPoint { x: 3, y: 1, z: 2 }).unwrap();
        assert_eq!(bright_cell.graphic, CellGraphic::Glyph('█'));
        assert_eq!(bright_cell.weight, CellWeight::Three);
    }

    #[test]
    fn cell_color_maps_rgb_zero_to_two_fifty_five_into_a_flat_zero_to_one_color() {
        let manifest = example_manifest();
        let composition = build_composition(&manifest, 4).unwrap();
        let torch = &composition.groups[1];

        assert_eq!(
            torch.get(CellPoint::origin()).unwrap().color,
            CellColor::Flat([255.0 / 255.0, 140.0 / 255.0, 40.0 / 255.0, 1.0])
        );
    }

    #[test]
    fn invisible_modules_and_groups_contribute_no_cells() {
        let mut manifest = example_manifest();
        manifest.document.modules[1].visible = false;
        manifest.document.modules[0].groups[1].visible = false;

        let composition = build_composition(&manifest, 4).unwrap();

        assert!(composition.groups[1].bounds().is_none());
        assert!(composition.groups[0]
            .get(CellPoint { x: 3, y: 2, z: 1 })
            .is_none());
        assert!(composition.groups[0]
            .get(CellPoint { x: 0, y: 0, z: 0 })
            .is_some());
    }

    #[test]
    fn data_lanes_carry_the_active_breath() {
        assert_eq!(build_data_lanes(4).breath(), Some(4));
    }

    #[test]
    fn build_render_space_bundles_camera_composition_and_data_lanes() {
        let manifest = example_manifest();
        let camera = Camera::default();
        let render_space = build_render_space(&manifest, 4, camera).unwrap();

        assert_eq!(render_space.camera, camera);
        assert_eq!(render_space.composition.groups.len(), 2);
        assert_eq!(render_space.data_lanes.breath(), Some(4));
    }
}
