use anyhow::{Context, Result};
use thaum_renderer_domain::{
    Camera, Cell, CellColor, CellColorSlot, CellGraphic, CellGroup, CellGroupIntakeBehavior,
    CellMaterialId, CellPoint, CellWeight, Composition, DataLanes, SpriteGraphic, WorldPoint,
    CELL_SHADER_TEXTURE_SHIMMER, CELL_SHADER_WARBLE_DIAGONAL, CELL_SHADER_WARBLE_DISTORT_1,
    CELL_SHADER_WARBLE_DISTORT_5, CELL_SHADER_WARBLE_FUDGE_1, CELL_SHADER_WARBLE_FUDGE_5,
    CELL_SHADER_WEIGHT_SIN,
};

use crate::file_schema::{
    parse_grid_point, CellAppearance, CellColorSlotValue, CellColorValue, CellGraphicValue,
    FileSchema, GridPoint, Group, RasterSegment, Rgb,
};

/// The transient renderer handoff assembled from one file schema at one active breath.
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

fn rgb_to_cell_color(rgb: Rgb) -> [f32; 4] {
    [
        rgb.r as f32 / 255.0,
        rgb.g as f32 / 255.0,
        rgb.b as f32 / 255.0,
        1.0,
    ]
}

/// Temporary renderer-asset-root bridge for the one built-in proof material.
/// The source schema remains filename-based; a later renderer material registry
/// replaces this narrow resolver without changing painter files.
fn resolve_material_asset(asset_file: &str) -> Result<CellMaterialId> {
    match asset_file {
        "materials/gray-scale.json" => Ok(CellMaterialId::GrayScale),
        _ => anyhow::bail!("unknown renderer material asset '{asset_file}'"),
    }
}

fn resolve_color_slot(slot: &CellColorSlotValue) -> Result<CellColorSlot> {
    match slot {
        CellColorSlotValue::Flat(rgb) => Ok(CellColorSlot::Flat(rgb_to_cell_color(*rgb))),
        CellColorSlotValue::Material { asset_file } => {
            Ok(CellColorSlot::Material(resolve_material_asset(asset_file)?))
        }
    }
}

fn resolve_color(color: &CellColorValue) -> Result<CellColor> {
    match color {
        CellColorValue::Flat(rgb) => Ok(CellColor::Flat(rgb_to_cell_color(*rgb))),
        CellColorValue::Material { asset_file } => {
            Ok(CellColor::Material(resolve_material_asset(asset_file)?))
        }
        CellColorValue::Slots { a, b, c } => Ok(CellColor::slots(
            resolve_color_slot(a)?,
            resolve_color_slot(b)?,
            resolve_color_slot(c)?,
        )),
    }
}

fn resolve_graphic(graphic: &CellGraphicValue) -> CellGraphic {
    match graphic {
        CellGraphicValue::Glyph(glyph) => CellGraphic::Glyph(*glyph),
        CellGraphicValue::Sprite { asset_file } => {
            CellGraphic::Sprite(SpriteGraphic::new(asset_file))
        }
    }
}

fn resolve_shader_asset(asset_file: &str) -> Result<u32> {
    match asset_file {
        "cell-shaders/weight-sin.json" => Ok(CELL_SHADER_WEIGHT_SIN),
        "cell-shaders/texture-shimmer.json" => Ok(CELL_SHADER_TEXTURE_SHIMMER),
        "cell-shaders/warble-diagonal.json" => Ok(CELL_SHADER_WARBLE_DIAGONAL),
        "cell-shaders/warble-fudge-1.json" => Ok(CELL_SHADER_WARBLE_FUDGE_1),
        "cell-shaders/warble-fudge-5.json" => Ok(CELL_SHADER_WARBLE_FUDGE_5),
        "cell-shaders/warble-distort-1.json" => Ok(CELL_SHADER_WARBLE_DISTORT_1),
        "cell-shaders/warble-distort-5.json" => Ok(CELL_SHADER_WARBLE_DISTORT_5),
        _ => anyhow::bail!("unknown renderer cell shader asset '{asset_file}'"),
    }
}

fn renderer_cell_from_appearance(position: CellPoint, appearance: &CellAppearance) -> Result<Cell> {
    Ok(Cell {
        position,
        graphic: resolve_graphic(&appearance.graphic),
        color: resolve_color(&appearance.color)?,
        weight: CellWeight::from_index_clamped(appearance.weight_index as i32),
        shader_stack: appearance
            .shader_stack
            .iter()
            .map(|asset_file| resolve_shader_asset(asset_file))
            .collect::<Result<Vec<_>>>()?,
        ..Cell::default()
    })
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
    let Some(move_property) = group
        .properties
        .iter()
        .find(|property| property.kind == "move")
    else {
        return Ok(GridPoint::default());
    };
    let Some(block) = move_property
        .blocks
        .iter()
        .find(|block| breath_in_window(active_breath, block.start_breath, block.end_breath))
    else {
        // Binary tiling: a breath not covered by a solid block is an empty —
        // render nothing (interim behavior; per-row interpolation is a later pass).
        return Ok(GridPoint::default());
    };
    parse_grid_point(&block.value).with_context(|| {
        format!(
            "move property block '{}' value must be an {{x,y,z}} offset",
            block.id
        )
    })
}

/// Composites one authored group's active raster segment and move offset into its
/// own `CellGroup`, anchored at the group's directly authored global `placement`.
fn build_group_cell_group(group: &Group, active_breath: u32) -> Result<CellGroup> {
    let mut cell_group = CellGroup::new(world_point_from_grid(group.placement))
        .with_intake_behavior(CellGroupIntakeBehavior::Flat2d);

    if !group.visible {
        return Ok(cell_group);
    }
    let Some(segment) = active_raster_segment(group, active_breath) else {
        // Binary tiling: a breath not covered by a solid segment is an empty —
        // render nothing (interim behavior; per-row interpolation is a later pass).
        return Ok(cell_group);
    };
    let move_offset = active_move_offset(group, active_breath)
        .with_context(|| format!("group '{}' has an invalid move offset", group.id))?;

    for voxel in &segment.voxels {
        let position = cell_point_from_grid(add_grid_points(move_offset, voxel.position));
        cell_group.insert(
            renderer_cell_from_appearance(position, &voxel.appearance)
                .with_context(|| format!("invalid appearance at voxel {:?}", voxel.position))?,
        );
    }
    Ok(cell_group)
}

/// One renderer cell-group per authored group, in `document.group_order` — direct
/// 1:1, no intermediate compositing step (see `context/module-concept-audit.md`'s
/// "superseded" section for why the earlier module-wrapped shape was reversed).
pub fn build_composition(schema: &FileSchema, active_breath: u32) -> Result<Composition> {
    let mut groups = Vec::new();
    for group_id in &schema.document.group_order {
        let group = schema
            .document
            .groups
            .iter()
            .find(|group| &group.id == group_id)
            .with_context(|| {
                format!("document group_order references missing group '{group_id}'")
            })?;
        let cell_group = build_group_cell_group(group, active_breath)
            .with_context(|| format!("failed to composite group '{group_id}'"))?;
        groups.push(cell_group);
    }
    Ok(Composition::ordered(groups))
}

pub fn build_data_lanes(active_breath: u32) -> DataLanes {
    DataLanes::with_breath(active_breath as i32)
}

/// Assembles the render-space handoff for one file schema at one active breath.
///
/// `camera` should already be resolved (saved defaults + live overrides) by
/// `domain/rendering/camera/`; this seam only normalizes it into the handoff shape.
pub fn build_render_space(
    schema: &FileSchema,
    active_breath: u32,
    camera: Camera,
) -> Result<RenderSpace> {
    Ok(RenderSpace {
        camera,
        composition: build_composition(schema, active_breath)?,
        data_lanes: build_data_lanes(active_breath),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_schema::parse_file_schema_from_str;
    use crate::{PaintColor, PaintedCell};
    use thaum_renderer_domain::CellGraphic;

    use crate::storage::{SharedCellPatch, SharedDocumentFile};

    const EXAMPLE_FILE_SCHEMA_JSON: &str =
        include_str!("../../file/file-schema/example-thaum-painter-file-v3.json");

    fn example_file_schema() -> FileSchema {
        parse_file_schema_from_str(EXAMPLE_FILE_SCHEMA_JSON).unwrap()
    }

    #[test]
    fn one_cell_group_is_emitted_at_the_authored_placement() {
        let schema = example_file_schema();
        let composition = build_composition(&schema, 4).unwrap();

        assert_eq!(composition.groups.len(), 1);
        assert_eq!(
            composition.groups[0].origin,
            WorldPoint { x: 3, y: 2, z: 1 }
        );
    }

    #[test]
    fn source_appearance_maps_glyph_sprite_material_slots_and_ordered_shaders() {
        let schema = example_file_schema();
        let composition = build_composition(&schema, 4).unwrap();
        let asset = &composition.groups[0];

        assert_eq!(
            asset.get(CellPoint { x: 1, y: 0, z: 0 }).unwrap().graphic,
            CellGraphic::Glyph('R')
        );
        assert_eq!(
            asset.get(CellPoint { x: 2, y: 0, z: 0 }).unwrap().color,
            CellColor::Material(CellMaterialId::GrayScale)
        );
        assert_eq!(
            asset.get(CellPoint { x: 3, y: 0, z: 0 }).unwrap().graphic,
            CellGraphic::Sprite(SpriteGraphic::new("cell-sprites/torch.png"))
        );
        assert!(matches!(
            asset.get(CellPoint { x: 4, y: 0, z: 0 }).unwrap().color,
            CellColor::Slots { .. }
        ));
        assert_eq!(
            asset
                .get(CellPoint { x: 1, y: 0, z: 0 })
                .unwrap()
                .shader_stack,
            vec![CELL_SHADER_WEIGHT_SIN, CELL_SHADER_TEXTURE_SHIMMER]
        );
    }

    #[test]
    fn move_properties_offset_the_resolved_source_cells() {
        let schema = example_file_schema();
        let composition = build_composition(&schema, 4).unwrap();
        assert!(composition.groups[0]
            .get(CellPoint { x: 0, y: 0, z: 0 })
            .is_none());
        assert_eq!(
            composition.groups[0]
                .get(CellPoint { x: 1, y: 0, z: 0 })
                .unwrap()
                .graphic,
            CellGraphic::Glyph('R')
        );
    }

    #[test]
    fn invisible_groups_contribute_no_cells() {
        let mut schema = example_file_schema();
        schema.document.groups[0].visible = false;
        let composition = build_composition(&schema, 4).unwrap();
        assert!(composition.groups[0].bounds().is_none());
    }

    #[test]
    fn data_lanes_carry_the_active_breath() {
        assert_eq!(build_data_lanes(4).breath(), Some(4));
    }

    #[test]
    fn build_render_space_bundles_camera_composition_and_data_lanes() {
        let schema = example_file_schema();
        let camera = Camera::default();
        let render_space = build_render_space(&schema, 4, camera).unwrap();

        assert_eq!(render_space.camera, camera);
        assert_eq!(render_space.composition.groups.len(), 1);
        assert_eq!(render_space.data_lanes.breath(), Some(4));
    }

    #[test]
    fn an_invisible_layer_renders_no_cell_group() {
        // J 2026-09-12: the eye toggle must gate the live render path, not
        // just the compositor.
        let document = SharedDocumentFile::single_layer("doc-1", "Doc", "layer-1", "Layer 1");
        let mut runtime = SharedDocumentRuntime::new(document);
        assert!(runtime.set_layer_visible("layer-1", false));

        let groups = build_document_layer_cell_groups(&runtime, 0, 0.0, None, None);
        assert!(groups.is_empty());

        assert!(runtime.set_layer_visible("layer-1", true));
        let groups = build_document_layer_cell_groups(&runtime, 0, 0.0, None, None);
        assert_eq!(groups.len(), 1);
    }

    #[test]
    fn document_layer_render_applies_the_active_move_offset() {
        let document = SharedDocumentFile::single_layer("doc-1", "Doc", "layer-1", "Layer 1");
        let mut runtime = SharedDocumentRuntime::new(document);
        let block_id = runtime
            .active_raster_block_id("layer-1", 0)
            .expect("default raster block");
        let painted = PaintedCell {
            graphic: CellGraphic::Glyph('#'),
            color: PaintColor::FlatRgb(255, 255, 255),
            weight_index: 1,
            shader_stack: vec!["cell-shaders/weight-sin.json".to_string()],
        };
        runtime.stage_canvas_patches(
            "layer-1",
            &block_id,
            &[SharedCellPatch::new(
                CellPoint { x: 5, y: 5, z: 0 },
                None,
                Some(&painted),
            )],
        );
        runtime.add_move_offset("layer-1", 0, WorldPoint { x: 2, y: 0, z: 3 });
        assert!(runtime.set_layer_origin("layer-1", WorldPoint { x: 4, y: 5, z: 6 }));

        let groups = build_document_layer_cell_groups(&runtime, 0, 0.0, None, None);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].origin, WorldPoint { x: 4, y: 5, z: 6 });
        let moved_cell = groups[0]
            .iter_cells()
            .find(|cell| cell.position == CellPoint { x: 7, y: 5, z: 3 })
            .expect("moved painted cell");
        assert_eq!(moved_cell.shader_stack, vec![CELL_SHADER_WEIGHT_SIN]);

        // An in-flight drag's pending delta stacks on the committed offset.
        let groups = build_document_layer_cell_groups(
            &runtime,
            0,
            0.0,
            Some(("layer-1", WorldPoint { x: 1, y: 1, z: 0 })),
            None,
        );
        assert!(groups[0]
            .iter_cells()
            .any(|cell| cell.position == CellPoint { x: 8, y: 6, z: 3 }));
    }
}

// --- live document composition ------------------------------------------------

use crate::storage::SharedDocumentRuntime;
use crate::Canvas;

/// Renders `canvas`'s live-painted cells as one `CellGroup` in the renderer's
/// real rotating 3D intake path, not as module chrome, so the painter canvas
/// lives in scene space while the UI panels stay in the flat 2D layer. Every
/// cell shifts by the layer's active move offset — the offset changes where
/// the layer renders, never the raster data itself. The returned group origin
/// is supplied by the document-layer seam, where it remains a fixed pivot.
pub fn build_paint_canvas_cell_group(canvas: &Canvas, move_offset: WorldPoint) -> CellGroup {
    let mut group = CellGroup::new(WorldPoint { x: 0, y: 0, z: 0 });
    for (position, painted) in canvas {
        group.insert(Cell {
            position: CellPoint {
                x: position.x + move_offset.x,
                y: position.y + move_offset.y,
                z: position.z + move_offset.z,
            },
            graphic: painted.graphic.clone(),
            color: painted.color.to_cell_color(),
            weight: CellWeight::from_index_clamped(painted.weight_index as i32),
            // Until the renderer registry lands, live preview can resolve its
            // currently built-in filenames; persistence still retains unknown
            // filenames verbatim for a future asset root.
            shader_stack: painted
                .shader_stack
                .iter()
                .filter_map(|asset_file| resolve_shader_asset(asset_file).ok())
                .collect(),
            ..Cell::default()
        });
    }
    group
}

/// One scene group per document layer, back-to-front in document order.
/// Each layer renders shifted by its active move offset, plus one in-flight
/// vector move drag's pending delta on `pending_layer` — the live WYSIWYG
/// preview of the offset the drag will commit. `breath_fraction` (0..1) is
/// the elapsed portion of the current breath during playback: the move
/// offset resolves at the fractional breath so eased motion samples per
/// display frame (positions still snap to the integer grid). Paused or
/// scrubbing, pass 0.0.
pub fn build_document_layer_cell_groups(
    runtime: &SharedDocumentRuntime,
    current_breath: u32,
    breath_fraction: f32,
    pending_move_offset: Option<(&str, WorldPoint)>,
    graphic_fade: Option<&thaum_renderer_domain::shape_fade::fade::ShapeFade>,
) -> Vec<CellGroup> {
    runtime
        .layers()
        .iter()
        // The layer's visibility toggle gates the live render path too —
        // hidden layers skip compositing here exactly as they do in
        // `composited_canvas_in_layer_order` (J 2026-09-12: the eye toggle
        // previously only affected the compositor, so the viewport ignored
        // it).
        .filter(|layer| layer.visible)
        .filter_map(|layer| {
            // The render path resolves raster interpolation: an interpolating
            // empty blends its surrounding keyframes' canvases instead of
            // rendering nothing (the edit surface still sees the raw block).
            // The injected shape-fade resolver walks matched cells' glyphs
            // through the renderer's gradient tour; `None` falls back to the
            // halfway cutoff.
            let canvas =
                runtime.resolved_canvas_for_layer(&layer.layer_id, current_breath, graphic_fade)?;
            let mut offset = runtime.move_offset_fractional_for_layer(
                &layer.layer_id,
                current_breath,
                breath_fraction,
            );
            if let Some((pending_layer, pending)) = pending_move_offset {
                if pending_layer == layer.layer_id {
                    offset = WorldPoint {
                        x: offset.x + pending.x,
                        y: offset.y + pending.y,
                        z: offset.z + pending.z,
                    };
                }
            }
            let mut group = build_paint_canvas_cell_group(&canvas, offset);
            group.origin = runtime
                .layer_origin(&layer.layer_id)
                .unwrap_or_else(WorldPoint::origin);
            Some(group)
        })
        .collect()
}
