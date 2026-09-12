use anyhow::{bail, Context, Result};
use serde_json::Value;

pub const FILE_SCHEMA_KIND: &str = "thaum-painter-file";
pub const FILE_SCHEMA_VERSION: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GridPoint {
    pub x: i64,
    pub y: i64,
    pub z: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// One portable visual graphic. Asset files are renderer-asset-root-relative;
/// painter documents intentionally do not store game-local IDs or host paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CellGraphicValue {
    Glyph(char),
    Sprite { asset_file: String },
}

/// One source color assignment. Slots give sprites their native A/B/C palette
/// assignments while remaining valid for glyphs (which resolve through slot A).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CellColorValue {
    Flat(Rgb),
    Material {
        asset_file: String,
    },
    Slots {
        a: CellColorSlotValue,
        b: CellColorSlotValue,
        c: CellColorSlotValue,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CellColorSlotValue {
    Flat(Rgb),
    Material { asset_file: String },
}

/// The complete authored visual value for one non-empty painter cell. This is
/// deliberately source-facing: runtime texture/warble codes and renderer IDs
/// do not leak into transferable painter files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellAppearance {
    pub graphic: CellGraphicValue,
    pub color: CellColorValue,
    pub weight_index: i64,
    pub shader_stack: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSchemaMetadata {
    pub document_id: String,
    pub title: String,
    pub description: String,
    pub tags: Vec<String>,
    pub created_at: String,
    pub modified_at: String,
    pub authoring_origin: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DocumentBounds {
    pub min: GridPoint,
    pub width: i64,
    pub height: i64,
    pub depth: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BreathWindow {
    pub start_breath: u32,
    pub window_start_breath: u32,
    pub window_end_breath: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Voxel {
    pub position: GridPoint,
    pub appearance: CellAppearance,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RasterSegment {
    pub id: String,
    pub start_breath: u32,
    pub end_breath: u32,
    pub voxels: Vec<Voxel>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PropertyBlock {
    pub id: String,
    pub start_breath: u32,
    pub end_breath: u32,
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GroupProperty {
    pub id: String,
    pub kind: String,
    pub blocks: Vec<PropertyBlock>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub id: String,
    pub name: String,
    pub visible: bool,
    pub locked: bool,
    pub opacity: f64,
    /// Fixed authored pivot. Animated `move` property values offset rendered
    /// cells from here and must never rewrite this point.
    pub origin: GridPoint,
    pub placement: GridPoint,
    pub timing: BreathWindow,
    pub raster_segments: Vec<RasterSegment>,
    pub properties: Vec<GroupProperty>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaybackWindow {
    pub frames_per_breath: u32,
    pub loop_enabled: bool,
    pub document_window_start_breath: u32,
    pub document_window_end_breath: u32,
}

/// Declarative configuration consumed by the future general exporter. It says
/// what to export, never where or when an exporter wrote it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlatExport {
    pub file_name: String,
    pub name: String,
    pub facing: String,
    pub interpolation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExportSections {
    pub flat: Option<FlatExport>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DocumentContent {
    pub bounds: DocumentBounds,
    pub group_order: Vec<String>,
    pub groups: Vec<Group>,
    pub playback: PlaybackWindow,
    pub exports: ExportSections,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParticleEffectVisual {
    pub char: char,
    pub display_color: String,
    pub render_index: i64,
    pub weight_index: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParticleEffect {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub spawn_breath: u32,
    pub window_start: u32,
    pub window_end: u32,
    pub processed_breaths: u32,
    pub is_complete: bool,
    pub is_deleted: bool,
    pub visual: ParticleEffectVisual,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TimeAssets {
    pub particle_effects: Vec<ParticleEffect>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SavedCameraDefaults {
    pub orientation: String,
    pub focus_plane: i64,
    pub viewport_scale: f64,
    pub show_all_layers: bool,
    pub center_target_in_view: bool,
    pub pan_x: f64,
    pub pan_y: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LastExport {
    pub profile: String,
    pub exported_at: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ImportExportBookkeeping {
    pub source_import: Option<Value>,
    pub last_export: Option<LastExport>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FileSchema {
    pub version: u32,
    pub metadata: FileSchemaMetadata,
    pub document: DocumentContent,
    pub time_assets: TimeAssets,
    pub saved_camera_defaults: SavedCameraDefaults,
    pub import_export_bookkeeping: ImportExportBookkeeping,
}

fn field<'a>(value: &'a Value, name: &str) -> Result<&'a Value> {
    value
        .get(name)
        .with_context(|| format!("missing field '{name}'"))
}

fn require_str<'a>(value: &'a Value, name: &str) -> Result<&'a str> {
    field(value, name)?
        .as_str()
        .with_context(|| format!("field '{name}' must be a string"))
}

fn require_bool(value: &Value, name: &str) -> Result<bool> {
    field(value, name)?
        .as_bool()
        .with_context(|| format!("field '{name}' must be a boolean"))
}

fn require_f64(value: &Value, name: &str) -> Result<f64> {
    field(value, name)?
        .as_f64()
        .with_context(|| format!("field '{name}' must be a number"))
}

fn require_i64(value: &Value, name: &str) -> Result<i64> {
    field(value, name)?
        .as_i64()
        .with_context(|| format!("field '{name}' must be an integer"))
}

fn require_u32(value: &Value, name: &str) -> Result<u32> {
    let raw = field(value, name)?
        .as_u64()
        .with_context(|| format!("field '{name}' must be a non-negative integer"))?;
    u32::try_from(raw).with_context(|| format!("field '{name}' is out of range"))
}

fn require_char(value: &Value, name: &str) -> Result<char> {
    let raw = require_str(value, name)?;
    let mut chars = raw.chars();
    let first = chars
        .next()
        .with_context(|| format!("field '{name}' must not be empty"))?;
    if chars.next().is_some() {
        bail!("field '{name}' must be exactly one character, got '{raw}'");
    }
    Ok(first)
}

fn require_array<'a>(value: &'a Value, name: &str) -> Result<&'a Vec<Value>> {
    field(value, name)?
        .as_array()
        .with_context(|| format!("field '{name}' must be an array"))
}

fn require_string_array(value: &Value, name: &str) -> Result<Vec<String>> {
    require_array(value, name)?
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            entry
                .as_str()
                .map(str::to_owned)
                .with_context(|| format!("field '{name}[{index}]' must be a string"))
        })
        .collect()
}

pub(crate) fn parse_grid_point(value: &Value) -> Result<GridPoint> {
    Ok(GridPoint {
        x: require_i64(value, "x")?,
        y: require_i64(value, "y")?,
        z: require_i64(value, "z")?,
    })
}

fn parse_rgb(value: &Value) -> Result<Rgb> {
    let clamp = |component: i64, name: &str| -> Result<u8> {
        u8::try_from(component).with_context(|| format!("rgb component '{name}' must be 0-255"))
    };
    Ok(Rgb {
        r: clamp(require_i64(value, "r")?, "r")?,
        g: clamp(require_i64(value, "g")?, "g")?,
        b: clamp(require_i64(value, "b")?, "b")?,
    })
}

fn validate_asset_file(asset_file: &str, field_name: &str) -> Result<()> {
    if asset_file.is_empty()
        || asset_file.starts_with('/')
        || asset_file.starts_with('\\')
        || asset_file.split('/').any(|part| part == "..")
        || asset_file.split('\\').any(|part| part == "..")
    {
        bail!("{field_name} must be a non-empty relative asset filename");
    }
    Ok(())
}

fn require_asset_file(value: &Value, name: &str) -> Result<String> {
    let asset_file = require_str(value, name)?;
    validate_asset_file(asset_file, &format!("field '{name}'"))?;
    Ok(asset_file.to_owned())
}

fn parse_color_slot(value: &Value) -> Result<CellColorSlotValue> {
    match require_str(value, "kind")? {
        "flat" => Ok(CellColorSlotValue::Flat(parse_rgb(field(value, "rgb")?)?)),
        "material" => Ok(CellColorSlotValue::Material {
            asset_file: require_asset_file(value, "asset_file")?,
        }),
        kind => bail!("color slot kind must be 'flat' or 'material', got '{kind}'"),
    }
}

fn parse_color(value: &Value) -> Result<CellColorValue> {
    match require_str(value, "kind")? {
        "flat" => Ok(CellColorValue::Flat(parse_rgb(field(value, "rgb")?)?)),
        "material" => Ok(CellColorValue::Material {
            asset_file: require_asset_file(value, "asset_file")?,
        }),
        "slots" => Ok(CellColorValue::Slots {
            a: parse_color_slot(field(value, "a")?).context("invalid color slot a")?,
            b: parse_color_slot(field(value, "b")?).context("invalid color slot b")?,
            c: parse_color_slot(field(value, "c")?).context("invalid color slot c")?,
        }),
        kind => bail!("color kind must be 'flat', 'material', or 'slots', got '{kind}'"),
    }
}

fn parse_graphic(value: &Value) -> Result<CellGraphicValue> {
    match require_str(value, "kind")? {
        "glyph" => Ok(CellGraphicValue::Glyph(require_char(value, "glyph")?)),
        "sprite" => Ok(CellGraphicValue::Sprite {
            asset_file: require_asset_file(value, "asset_file")?,
        }),
        kind => bail!("graphic kind must be 'glyph' or 'sprite', got '{kind}'"),
    }
}

fn parse_shader_stack(value: &Value) -> Result<Vec<String>> {
    require_array(value, "shader_stack")?
        .iter()
        .enumerate()
        .map(|(index, shader)| {
            let asset_file = shader
                .as_str()
                .with_context(|| format!("shader_stack[{index}] must be a string"))?;
            validate_asset_file(asset_file, &format!("shader_stack[{index}]"))?;
            Ok(asset_file.to_owned())
        })
        .collect()
}

fn parse_cell_appearance(value: &Value) -> Result<CellAppearance> {
    Ok(CellAppearance {
        graphic: parse_graphic(field(value, "graphic")?).context("invalid cell graphic")?,
        color: parse_color(field(value, "color")?).context("invalid cell color")?,
        weight_index: require_i64(value, "weight_index")?,
        shader_stack: parse_shader_stack(value)?,
    })
}

fn parse_breath_window(value: &Value) -> Result<BreathWindow> {
    Ok(BreathWindow {
        start_breath: require_u32(value, "start_breath")?,
        window_start_breath: require_u32(value, "window_start_breath")?,
        window_end_breath: require_u32(value, "window_end_breath")?,
    })
}

fn parse_voxel(value: &Value) -> Result<Voxel> {
    Ok(Voxel {
        position: parse_grid_point(value)?,
        appearance: parse_cell_appearance(field(value, "appearance")?)
            .context("invalid voxel appearance")?,
    })
}

fn parse_raster_segment(value: &Value) -> Result<RasterSegment> {
    let voxels = require_array(value, "voxels")?
        .iter()
        .map(parse_voxel)
        .collect::<Result<Vec<_>>>()
        .context("invalid raster segment voxel")?;
    Ok(RasterSegment {
        id: require_str(value, "id")?.to_owned(),
        start_breath: require_u32(value, "start_breath")?,
        end_breath: require_u32(value, "end_breath")?,
        voxels,
    })
}

fn parse_property_block(value: &Value) -> Result<PropertyBlock> {
    Ok(PropertyBlock {
        id: require_str(value, "id")?.to_owned(),
        start_breath: require_u32(value, "start_breath")?,
        end_breath: require_u32(value, "end_breath")?,
        value: field(value, "value")?.clone(),
    })
}

fn parse_group_property(value: &Value) -> Result<GroupProperty> {
    let blocks = require_array(value, "blocks")?
        .iter()
        .map(parse_property_block)
        .collect::<Result<Vec<_>>>()
        .context("invalid property block")?;
    Ok(GroupProperty {
        id: require_str(value, "id")?.to_owned(),
        kind: require_str(value, "kind")?.to_owned(),
        blocks,
    })
}

fn parse_flat_export(value: &Value) -> Result<FlatExport> {
    let interpolation = require_str(value, "interpolation")?;
    if interpolation != "preserve" {
        bail!("flat export interpolation must be 'preserve', got '{interpolation}'");
    }
    Ok(FlatExport {
        file_name: require_asset_file(value, "file_name")?,
        name: require_str(value, "name")?.to_owned(),
        facing: require_str(value, "facing")?.to_owned(),
        interpolation: interpolation.to_owned(),
    })
}

fn parse_export_sections(value: &Value) -> Result<ExportSections> {
    let flat = match field(value, "flat")? {
        Value::Null => None,
        flat => Some(parse_flat_export(flat).context("invalid flat export")?),
    };
    Ok(ExportSections { flat })
}

fn parse_group(value: &Value) -> Result<Group> {
    let raster_segments = require_array(value, "raster_segments")?
        .iter()
        .map(parse_raster_segment)
        .collect::<Result<Vec<_>>>()
        .context("invalid raster segment")?;
    let properties = require_array(value, "properties")?
        .iter()
        .map(parse_group_property)
        .collect::<Result<Vec<_>>>()
        .context("invalid group property")?;
    Ok(Group {
        id: require_str(value, "id")?.to_owned(),
        name: require_str(value, "name")?.to_owned(),
        visible: require_bool(value, "visible")?,
        locked: require_bool(value, "locked")?,
        opacity: require_f64(value, "opacity")?,
        origin: parse_grid_point(field(value, "origin")?)?,
        placement: parse_grid_point(field(value, "placement")?)?,
        timing: parse_breath_window(field(value, "timing")?)?,
        raster_segments,
        properties,
    })
}

fn parse_document_bounds(value: &Value) -> Result<DocumentBounds> {
    Ok(DocumentBounds {
        min: GridPoint {
            x: require_i64(value, "min_x")?,
            y: require_i64(value, "min_y")?,
            z: require_i64(value, "min_z")?,
        },
        width: require_i64(value, "width")?,
        height: require_i64(value, "height")?,
        depth: require_i64(value, "depth")?,
    })
}

fn parse_playback_window(value: &Value) -> Result<PlaybackWindow> {
    Ok(PlaybackWindow {
        frames_per_breath: require_u32(value, "frames_per_breath")?,
        loop_enabled: require_bool(value, "loop_enabled")?,
        document_window_start_breath: require_u32(value, "document_window_start_breath")?,
        document_window_end_breath: require_u32(value, "document_window_end_breath")?,
    })
}

fn parse_document(value: &Value) -> Result<DocumentContent> {
    let groups = require_array(value, "groups")?
        .iter()
        .map(parse_group)
        .collect::<Result<Vec<_>>>()
        .context("invalid document group")?;
    Ok(DocumentContent {
        bounds: parse_document_bounds(field(value, "bounds")?)?,
        group_order: require_string_array(value, "group_order")?,
        groups,
        playback: parse_playback_window(field(value, "playback")?)?,
        exports: parse_export_sections(field(value, "exports")?)
            .context("invalid document exports")?,
    })
}

fn parse_particle_effect_visual(value: &Value) -> Result<ParticleEffectVisual> {
    Ok(ParticleEffectVisual {
        char: require_char(value, "char")?,
        display_color: require_str(value, "display_color")?.to_owned(),
        render_index: require_i64(value, "render_index")?,
        weight_index: require_i64(value, "weight_index")?,
    })
}

fn parse_particle_effect(value: &Value) -> Result<ParticleEffect> {
    Ok(ParticleEffect {
        id: require_str(value, "id")?.to_owned(),
        kind: require_str(value, "kind")?.to_owned(),
        name: require_str(value, "name")?.to_owned(),
        spawn_breath: require_u32(value, "spawn_breath")?,
        window_start: require_u32(value, "window_start")?,
        window_end: require_u32(value, "window_end")?,
        processed_breaths: require_u32(value, "processed_breaths")?,
        is_complete: require_bool(value, "is_complete")?,
        is_deleted: require_bool(value, "is_deleted")?,
        visual: parse_particle_effect_visual(field(value, "visual")?)?,
    })
}

fn parse_time_assets(value: &Value) -> Result<TimeAssets> {
    let particle_effects = require_array(value, "particle_effects")?
        .iter()
        .map(parse_particle_effect)
        .collect::<Result<Vec<_>>>()
        .context("invalid time asset particle effect")?;
    Ok(TimeAssets { particle_effects })
}

fn parse_saved_camera_defaults(value: &Value) -> Result<SavedCameraDefaults> {
    Ok(SavedCameraDefaults {
        orientation: require_str(value, "orientation")?.to_owned(),
        focus_plane: require_i64(value, "focus_plane")?,
        viewport_scale: require_f64(value, "viewport_scale")?,
        show_all_layers: require_bool(value, "show_all_layers")?,
        center_target_in_view: require_bool(value, "center_target_in_view")?,
        pan_x: require_f64(value, "pan_x")?,
        pan_y: require_f64(value, "pan_y")?,
    })
}

fn parse_last_export(value: &Value) -> Result<LastExport> {
    Ok(LastExport {
        profile: require_str(value, "profile")?.to_owned(),
        exported_at: require_str(value, "exported_at")?.to_owned(),
    })
}

fn parse_import_export_bookkeeping(value: &Value) -> Result<ImportExportBookkeeping> {
    let source_import = match field(value, "source_import")? {
        Value::Null => None,
        other => Some(other.clone()),
    };
    let last_export = match field(value, "last_export")? {
        Value::Null => None,
        other => Some(parse_last_export(other).context("invalid last_export")?),
    };
    Ok(ImportExportBookkeeping {
        source_import,
        last_export,
    })
}

fn parse_metadata(value: &Value) -> Result<FileSchemaMetadata> {
    Ok(FileSchemaMetadata {
        document_id: require_str(value, "document_id")?.to_owned(),
        title: require_str(value, "title")?.to_owned(),
        description: require_str(value, "description")?.to_owned(),
        tags: require_string_array(value, "tags")?,
        created_at: require_str(value, "created_at")?.to_owned(),
        modified_at: require_str(value, "modified_at")?.to_owned(),
        authoring_origin: require_str(value, "authoring_origin")?.to_owned(),
    })
}

/// Parses one saved thaum-painter file schema from an already-decoded JSON value.
///
/// This owns shape validation only: file I/O is `domain/file/storage/`'s job, not this seam's.
pub fn parse_file_schema(value: &Value) -> Result<FileSchema> {
    let kind = require_str(value, "kind")?;
    if kind != FILE_SCHEMA_KIND {
        bail!("file-schema kind must be '{FILE_SCHEMA_KIND}', got '{kind}'");
    }
    let version = require_u32(value, "version")?;
    if version != FILE_SCHEMA_VERSION {
        bail!("file schema version must be {FILE_SCHEMA_VERSION}, got {version}");
    }

    Ok(FileSchema {
        version,
        metadata: parse_metadata(field(value, "metadata")?).context("invalid metadata")?,
        document: parse_document(field(value, "document")?).context("invalid document")?,
        time_assets: parse_time_assets(field(value, "time_assets")?)
            .context("invalid time_assets")?,
        saved_camera_defaults: parse_saved_camera_defaults(field(value, "saved_camera_defaults")?)
            .context("invalid saved_camera_defaults")?,
        import_export_bookkeeping: parse_import_export_bookkeeping(field(
            value,
            "import_export_bookkeeping",
        )?)
        .context("invalid import_export_bookkeeping")?,
    })
}

/// Parses one saved thaum-painter file schema directly from its raw JSON text.
pub fn parse_file_schema_from_str(text: &str) -> Result<FileSchema> {
    let value: Value = serde_json::from_str(text).context("file schema is not valid JSON")?;
    parse_file_schema(&value)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE_FILE_SCHEMA_JSON: &str = include_str!("example-thaum-painter-file-v3.json");
    const EXAMPLE_FILE_SCHEMA_V2_JSON: &str = include_str!("example-thaum-painter-file-v2.json");

    #[test]
    fn parses_the_pinned_example_file_schema_without_error() {
        let schema = parse_file_schema_from_str(EXAMPLE_FILE_SCHEMA_JSON).unwrap();
        assert_eq!(schema.version, 3);
        assert_eq!(schema.metadata.document_id, "doc_cell_language_001");
    }

    #[test]
    fn the_v2_example_is_now_an_unsupported_generation() {
        // The cell-language schema break has no importer or migration pass.
        let error = parse_file_schema_from_str(EXAMPLE_FILE_SCHEMA_V2_JSON).unwrap_err();
        assert!(error.to_string().contains("version"));
        assert!(error.to_string().contains("got 2"));
    }

    #[test]
    fn parses_groups_with_distinct_fixed_origin_and_placement() {
        let schema = parse_file_schema_from_str(EXAMPLE_FILE_SCHEMA_JSON).unwrap();
        assert_eq!(schema.document.group_order, vec!["group_asset"]);
        let asset = &schema.document.groups[0];
        assert_eq!(asset.placement, GridPoint { x: 3, y: 2, z: 1 });
        assert_eq!(asset.origin, GridPoint { x: 1, y: 0, z: 0 });
    }

    #[test]
    fn parses_glyph_sprite_material_slots_and_ordered_shaders() {
        let schema = parse_file_schema_from_str(EXAMPLE_FILE_SCHEMA_JSON).unwrap();
        let segment = &schema.document.groups[0].raster_segments[0];
        assert_eq!(segment.voxels.len(), 4);
        assert_eq!(
            segment.voxels[0].appearance.graphic,
            CellGraphicValue::Glyph('R')
        );
        assert_eq!(
            segment.voxels[1].appearance.color,
            CellColorValue::Material {
                asset_file: "materials/gray-scale.json".to_owned()
            }
        );
        assert_eq!(
            segment.voxels[2].appearance.graphic,
            CellGraphicValue::Sprite {
                asset_file: "cell-sprites/torch.png".to_owned()
            }
        );
        assert!(matches!(
            segment.voxels[3].appearance.color,
            CellColorValue::Slots { .. }
        ));
        assert_eq!(
            segment.voxels[0].appearance.shader_stack,
            vec![
                "cell-shaders/weight-sin.json",
                "cell-shaders/texture-shimmer.json"
            ]
        );
    }

    #[test]
    fn parses_property_blocks_with_arbitrary_json_value_shapes() {
        let schema = parse_file_schema_from_str(EXAMPLE_FILE_SCHEMA_JSON).unwrap();
        let asset = &schema.document.groups[0];
        let move_property = asset
            .properties
            .iter()
            .find(|property| property.kind == "move")
            .unwrap();
        assert_eq!(move_property.blocks.len(), 1);
        assert_eq!(
            move_property.blocks[0].value,
            serde_json::json!({ "x": 1, "y": 0, "z": 0 })
        );
    }

    #[test]
    fn parses_flat_export_and_saved_camera_defaults() {
        let schema = parse_file_schema_from_str(EXAMPLE_FILE_SCHEMA_JSON).unwrap();
        assert!(schema.time_assets.particle_effects.is_empty());
        assert_eq!(schema.saved_camera_defaults.orientation, "xy");
        assert_eq!(schema.saved_camera_defaults.pan_x, 0.0);
        assert_eq!(
            schema.document.exports.flat,
            Some(FlatExport {
                file_name: "exports/cell-language-proof.taf".to_owned(),
                name: "Cell Language Proof".to_owned(),
                facing: "pos-z".to_owned(),
                interpolation: "preserve".to_owned(),
            })
        );
    }

    #[test]
    fn rejects_a_file_schema_with_the_wrong_kind() {
        let value =
            serde_json::json!({ "kind": "not-thaum-painter-file", "version": FILE_SCHEMA_VERSION });
        let error = parse_file_schema(&value).unwrap_err();
        assert!(error.to_string().contains("kind"));
    }

    #[test]
    fn rejects_a_file_schema_with_an_unsupported_version() {
        // v1 is the pre-bars generation and now rejects; so does anything newer.
        let value = serde_json::json!({ "kind": FILE_SCHEMA_KIND, "version": 1 });
        let error = parse_file_schema(&value).unwrap_err();
        assert!(error.to_string().contains("version"));
        let value =
            serde_json::json!({ "kind": FILE_SCHEMA_KIND, "version": FILE_SCHEMA_VERSION + 1 });
        let error = parse_file_schema(&value).unwrap_err();
        assert!(error.to_string().contains("version"));
    }

    #[test]
    fn rejects_a_file_schema_missing_a_required_field() {
        let value = serde_json::json!({ "kind": FILE_SCHEMA_KIND, "version": FILE_SCHEMA_VERSION });
        let error = parse_file_schema(&value).unwrap_err();
        assert!(error.to_string().contains("metadata"));
    }

    #[test]
    fn rejects_a_multi_character_glyph_and_absolute_asset_file() {
        let glyph = serde_json::json!({ "kind": "glyph", "glyph": "ab" });
        assert!(parse_graphic(&glyph)
            .unwrap_err()
            .to_string()
            .contains("exactly one character"));
        let sprite = serde_json::json!({ "kind": "sprite", "asset_file": "/torch.png" });
        assert!(parse_graphic(&sprite)
            .unwrap_err()
            .to_string()
            .contains("relative asset filename"));
    }
}
