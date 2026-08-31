use anyhow::{bail, Context, Result};
use serde_json::Value;

pub const MANIFEST_KIND: &str = "thaum-painter-file";
pub const MANIFEST_VERSION: u32 = 1;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestMetadata {
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Voxel {
    pub position: GridPoint,
    pub char: char,
    pub rgb: Rgb,
    pub weight_index: i64,
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
    pub local_placement: GridPoint,
    pub timing: BreathWindow,
    pub raster_segments: Vec<RasterSegment>,
    pub properties: Vec<GroupProperty>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Module {
    pub id: String,
    pub name: String,
    pub visible: bool,
    pub locked: bool,
    pub opacity: f64,
    pub placement: GridPoint,
    pub group_order: Vec<String>,
    pub groups: Vec<Group>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaybackWindow {
    pub frames_per_breath: u32,
    pub loop_enabled: bool,
    pub document_window_start_breath: u32,
    pub document_window_end_breath: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DocumentContent {
    pub bounds: DocumentBounds,
    pub module_order: Vec<String>,
    pub modules: Vec<Module>,
    pub playback: PlaybackWindow,
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
pub struct Manifest {
    pub version: u32,
    pub metadata: ManifestMetadata,
    pub document: DocumentContent,
    pub time_assets: TimeAssets,
    pub saved_camera_defaults: SavedCameraDefaults,
    pub import_export_bookkeeping: ImportExportBookkeeping,
}

fn field<'a>(value: &'a Value, name: &str) -> Result<&'a Value> {
    value.get(name).with_context(|| format!("missing field '{name}'"))
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

fn parse_grid_point(value: &Value) -> Result<GridPoint> {
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
        char: require_char(value, "char")?,
        rgb: parse_rgb(field(value, "rgb")?)?,
        weight_index: require_i64(value, "weight_index")?,
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
        local_placement: parse_grid_point(field(value, "local_placement")?)?,
        timing: parse_breath_window(field(value, "timing")?)?,
        raster_segments,
        properties,
    })
}

fn parse_module(value: &Value) -> Result<Module> {
    let groups = require_array(value, "groups")?
        .iter()
        .map(parse_group)
        .collect::<Result<Vec<_>>>()
        .context("invalid module group")?;
    Ok(Module {
        id: require_str(value, "id")?.to_owned(),
        name: require_str(value, "name")?.to_owned(),
        visible: require_bool(value, "visible")?,
        locked: require_bool(value, "locked")?,
        opacity: require_f64(value, "opacity")?,
        placement: parse_grid_point(field(value, "placement")?)?,
        group_order: require_string_array(value, "group_order")?,
        groups,
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
    let modules = require_array(value, "modules")?
        .iter()
        .map(parse_module)
        .collect::<Result<Vec<_>>>()
        .context("invalid document module")?;
    Ok(DocumentContent {
        bounds: parse_document_bounds(field(value, "bounds")?)?,
        module_order: require_string_array(value, "module_order")?,
        modules,
        playback: parse_playback_window(field(value, "playback")?)?,
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

fn parse_metadata(value: &Value) -> Result<ManifestMetadata> {
    Ok(ManifestMetadata {
        document_id: require_str(value, "document_id")?.to_owned(),
        title: require_str(value, "title")?.to_owned(),
        description: require_str(value, "description")?.to_owned(),
        tags: require_string_array(value, "tags")?,
        created_at: require_str(value, "created_at")?.to_owned(),
        modified_at: require_str(value, "modified_at")?.to_owned(),
        authoring_origin: require_str(value, "authoring_origin")?.to_owned(),
    })
}

/// Parses one saved thaum-painter file manifest from an already-decoded JSON value.
///
/// This owns shape validation only: file I/O is `domain/file/storage/`'s job, not this seam's.
pub fn parse_manifest(value: &Value) -> Result<Manifest> {
    let kind = require_str(value, "kind")?;
    if kind != MANIFEST_KIND {
        bail!("manifest kind must be '{MANIFEST_KIND}', got '{kind}'");
    }
    let version = require_u32(value, "version")?;
    if version != MANIFEST_VERSION {
        bail!("manifest version must be {MANIFEST_VERSION}, got {version}");
    }

    Ok(Manifest {
        version,
        metadata: parse_metadata(field(value, "metadata")?).context("invalid metadata")?,
        document: parse_document(field(value, "document")?).context("invalid document")?,
        time_assets: parse_time_assets(field(value, "time_assets")?).context("invalid time_assets")?,
        saved_camera_defaults: parse_saved_camera_defaults(field(value, "saved_camera_defaults")?)
            .context("invalid saved_camera_defaults")?,
        import_export_bookkeeping: parse_import_export_bookkeeping(field(
            value,
            "import_export_bookkeeping",
        )?)
        .context("invalid import_export_bookkeeping")?,
    })
}

/// Parses one saved thaum-painter file manifest directly from its raw JSON text.
pub fn parse_manifest_from_str(text: &str) -> Result<Manifest> {
    let value: Value = serde_json::from_str(text).context("manifest is not valid JSON")?;
    parse_manifest(&value)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE_MANIFEST_JSON: &str = include_str!("example-thaum-painter-file-v1.json");

    #[test]
    fn parses_the_pinned_example_manifest_without_error() {
        let manifest = parse_manifest_from_str(EXAMPLE_MANIFEST_JSON).unwrap();
        assert_eq!(manifest.version, 1);
        assert_eq!(manifest.metadata.document_id, "doc_cavern_sign_001");
    }

    #[test]
    fn parses_two_modules_each_with_their_own_placement_and_groups() {
        let manifest = parse_manifest_from_str(EXAMPLE_MANIFEST_JSON).unwrap();
        assert_eq!(manifest.document.module_order, vec!["module_cavern_sign", "module_torch_marker"]);
        assert_eq!(manifest.document.modules.len(), 2);

        let sign = &manifest.document.modules[0];
        assert_eq!(sign.id, "module_cavern_sign");
        assert_eq!(sign.placement, GridPoint { x: 0, y: 0, z: 0 });
        assert_eq!(sign.groups.len(), 3);

        let torch = &manifest.document.modules[1];
        assert_eq!(torch.id, "module_torch_marker");
        assert_eq!(torch.placement, GridPoint { x: 18, y: 0, z: 0 });
        assert_eq!(torch.groups.len(), 1);
    }

    #[test]
    fn group_local_placement_stays_separate_from_module_placement() {
        let manifest = parse_manifest_from_str(EXAMPLE_MANIFEST_JSON).unwrap();
        let sign = &manifest.document.modules[0];
        let glow = sign
            .groups
            .iter()
            .find(|group| group.id == "group_glow")
            .unwrap();
        assert_eq!(glow.local_placement, GridPoint { x: 2, y: 1, z: 2 });
        assert_ne!(glow.local_placement, sign.placement);
    }

    #[test]
    fn parses_raster_segment_voxels_with_color_and_weight() {
        let manifest = parse_manifest_from_str(EXAMPLE_MANIFEST_JSON).unwrap();
        let letters = &manifest.document.modules[0].groups[1];
        assert_eq!(letters.id, "group_letters");
        let segment = &letters.raster_segments[0];
        assert_eq!(segment.voxels.len(), 3);
        assert_eq!(segment.voxels[0].char, 'R');
        assert_eq!(segment.voxels[0].rgb, Rgb { r: 255, g: 210, b: 120 });
        assert_eq!(segment.voxels[0].weight_index, 2);
    }

    #[test]
    fn parses_property_blocks_with_arbitrary_json_value_shapes() {
        let manifest = parse_manifest_from_str(EXAMPLE_MANIFEST_JSON).unwrap();
        let glow = &manifest.document.modules[0].groups[2];
        let move_property = glow
            .properties
            .iter()
            .find(|property| property.kind == "move")
            .unwrap();
        assert_eq!(move_property.blocks.len(), 2);
        assert_eq!(
            move_property.blocks[1].value,
            serde_json::json!({ "x": 1, "y": 0, "z": 0 })
        );
    }

    #[test]
    fn parses_time_assets_and_saved_camera_defaults() {
        let manifest = parse_manifest_from_str(EXAMPLE_MANIFEST_JSON).unwrap();
        assert_eq!(manifest.time_assets.particle_effects.len(), 1);
        assert_eq!(manifest.time_assets.particle_effects[0].visual.char, '*');
        assert_eq!(manifest.saved_camera_defaults.orientation, "xy");
        assert_eq!(manifest.saved_camera_defaults.pan_x, 0.0);
    }

    #[test]
    fn parses_import_export_bookkeeping_with_null_source_import() {
        let manifest = parse_manifest_from_str(EXAMPLE_MANIFEST_JSON).unwrap();
        assert!(manifest.import_export_bookkeeping.source_import.is_none());
        let last_export = manifest.import_export_bookkeeping.last_export.unwrap();
        assert_eq!(last_export.profile, "renderer-scene-preview");
    }

    #[test]
    fn rejects_a_manifest_with_the_wrong_kind() {
        let value = serde_json::json!({ "kind": "not-thaum-painter-file", "version": 1 });
        let error = parse_manifest(&value).unwrap_err();
        assert!(error.to_string().contains("kind"));
    }

    #[test]
    fn rejects_a_manifest_with_an_unsupported_version() {
        let value = serde_json::json!({ "kind": MANIFEST_KIND, "version": 2 });
        let error = parse_manifest(&value).unwrap_err();
        assert!(error.to_string().contains("version"));
    }

    #[test]
    fn rejects_a_manifest_missing_a_required_field() {
        let value = serde_json::json!({ "kind": MANIFEST_KIND, "version": 1 });
        let error = parse_manifest(&value).unwrap_err();
        assert!(error.to_string().contains("metadata"));
    }

    #[test]
    fn rejects_a_multi_character_voxel_char() {
        let value = serde_json::json!({
            "x": 0, "y": 0, "z": 0,
            "char": "ab",
            "rgb": { "r": 0, "g": 0, "b": 0 },
            "weight_index": 0
        });
        let error = parse_voxel(&value).unwrap_err();
        assert!(error.to_string().contains("exactly one character"));
    }
}
