use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use thaum_renderer_domain::{CellGraphic, CellMaterialId, CellPoint, SpriteGraphic};

use crate::{Canvas, PaintColor, PaintedCell};

pub const SHARED_DOCUMENT_KIND: &str = "thaum-painter-shared-document";
pub const SHARED_DOCUMENT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedDocumentPropertyBlock {
    pub id: String,
    pub start_breath: u32,
    #[serde(default = "default_layer_length_breaths")]
    pub length_breaths: u32,
    #[serde(default)]
    pub is_blank: bool,
}

/// Which content neighbor a blank property block merges into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropertyBlockMergeDirection {
    Left,
    Right,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedDocumentPropertyTrack {
    pub property_id: String,
    #[serde(default)]
    pub blocks: Vec<SharedDocumentPropertyBlock>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedDocumentLayer {
    pub layer_id: String,
    pub name: String,
    #[serde(default = "default_layer_visible")]
    pub visible: bool,
    #[serde(default)]
    pub locked: bool,
    /// The breath the layer's own timeline bar starts at, shown/edited on the layers-panel
    /// timeline row. Purely an authoring/UI concept for now — it does not yet gate compositing.
    #[serde(default)]
    pub start_breath: u32,
    #[serde(default = "default_layer_length_breaths")]
    pub length_breaths: u32,
    #[serde(default)]
    pub property_tracks: Vec<SharedDocumentPropertyTrack>,
}

fn default_layer_visible() -> bool {
    true
}

fn default_layer_length_breaths() -> u32 {
    24
}

fn default_raster_property_track(start_breath: u32, length_breaths: u32) -> SharedDocumentPropertyTrack {
    SharedDocumentPropertyTrack {
        property_id: "raster".to_string(),
        blocks: vec![SharedDocumentPropertyBlock {
            id: "block-1".to_string(),
            start_breath,
            length_breaths: length_breaths.max(1),
            is_blank: false,
        }],
    }
}

fn next_property_block_id(blocks: &[SharedDocumentPropertyBlock]) -> String {
    let used: BTreeMap<u32, ()> = blocks
        .iter()
        .filter_map(|block| block.id.strip_prefix("block-")?.parse::<u32>().ok())
        .map(|number| (number, ()))
        .collect();
    let mut next = 1;
    while used.contains_key(&next) {
        next += 1;
    }
    format!("block-{next}")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedDocumentFile {
    pub file_kind: String,
    pub schema_version: u32,
    pub document_id: String,
    pub title: String,
    pub layers: Vec<SharedDocumentLayer>,
}

impl SharedDocumentFile {
    pub fn single_layer(
        document_id: impl Into<String>,
        title: impl Into<String>,
        layer_id: impl Into<String>,
        layer_name: impl Into<String>,
    ) -> Self {
        Self {
            file_kind: SHARED_DOCUMENT_KIND.to_string(),
            schema_version: SHARED_DOCUMENT_SCHEMA_VERSION,
            document_id: document_id.into(),
            title: title.into(),
            layers: vec![SharedDocumentLayer {
                layer_id: layer_id.into(),
                name: layer_name.into(),
                visible: true,
                locked: false,
                start_breath: 0,
                length_breaths: default_layer_length_breaths(),
                property_tracks: vec![default_raster_property_track(0, default_layer_length_breaths())],
            }],
        }
    }

    pub fn has_layer(&self, layer_id: &str) -> bool {
        self.layers.iter().any(|layer| layer.layer_id == layer_id)
    }

    pub fn first_layer_id(&self) -> Option<&str> {
        self.layers.first().map(|layer| layer.layer_id.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedDocumentPaths {
    pub root: PathBuf,
    pub document_file_path: PathBuf,
    pub actions_file_path: PathBuf,
}

impl SharedDocumentPaths {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            document_file_path: root.join("document.json"),
            actions_file_path: root.join("actions.jsonl"),
            root,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedCellPoint {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl From<CellPoint> for PersistedCellPoint {
    fn from(value: CellPoint) -> Self {
        Self {
            x: value.x,
            y: value.y,
            z: value.z,
        }
    }
}

impl PersistedCellPoint {
    pub fn to_runtime(&self) -> CellPoint {
        CellPoint {
            x: self.x,
            y: self.y,
            z: self.z,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum PersistedSharedPaintColor {
    FlatRgb { red: u8, green: u8, blue: u8 },
    Material { material: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum PersistedSharedGraphic {
    None,
    Glyph { glyph: char },
    Sprite { atlas_relative_path: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedSharedPaintedCell {
    pub graphic: PersistedSharedGraphic,
    pub color: PersistedSharedPaintColor,
    pub weight_index: i64,
}

impl PersistedSharedPaintedCell {
    pub fn from_runtime(cell: &PaintedCell) -> Self {
        Self {
            graphic: PersistedSharedGraphic::from_runtime(&cell.graphic),
            color: PersistedSharedPaintColor::from_runtime(cell.color),
            weight_index: cell.weight_index,
        }
    }

    pub fn to_runtime(&self) -> PaintedCell {
        PaintedCell {
            graphic: self.graphic.to_runtime(),
            color: self.color.to_runtime(),
            weight_index: self.weight_index,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedCellPatch {
    pub position: PersistedCellPoint,
    pub before: Option<PersistedSharedPaintedCell>,
    pub after: Option<PersistedSharedPaintedCell>,
}

impl SharedCellPatch {
    pub fn new(
        position: CellPoint,
        before: Option<&PaintedCell>,
        after: Option<&PaintedCell>,
    ) -> Self {
        Self {
            position: position.into(),
            before: before.map(PersistedSharedPaintedCell::from_runtime),
            after: after.map(PersistedSharedPaintedCell::from_runtime),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SharedDocumentAction {
    CellPatchSet { patches: Vec<SharedCellPatch> },
    Undo,
    Redo,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedDocumentActionRecord {
    pub action_id: String,
    pub document_id: String,
    pub layer_id: String,
    pub user_id: String,
    pub created_at: String,
    pub action: SharedDocumentAction,
}

impl SharedDocumentActionRecord {
    pub fn cell_patch_set(
        action_id: impl Into<String>,
        document_id: impl Into<String>,
        layer_id: impl Into<String>,
        user_id: impl Into<String>,
        created_at: impl Into<String>,
        patches: Vec<SharedCellPatch>,
    ) -> Self {
        Self {
            action_id: action_id.into(),
            document_id: document_id.into(),
            layer_id: layer_id.into(),
            user_id: user_id.into(),
            created_at: created_at.into(),
            action: SharedDocumentAction::CellPatchSet { patches },
        }
    }

    pub fn undo(
        action_id: impl Into<String>,
        document_id: impl Into<String>,
        layer_id: impl Into<String>,
        user_id: impl Into<String>,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            action_id: action_id.into(),
            document_id: document_id.into(),
            layer_id: layer_id.into(),
            user_id: user_id.into(),
            created_at: created_at.into(),
            action: SharedDocumentAction::Undo,
        }
    }

    pub fn redo(
        action_id: impl Into<String>,
        document_id: impl Into<String>,
        layer_id: impl Into<String>,
        user_id: impl Into<String>,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            action_id: action_id.into(),
            document_id: document_id.into(),
            layer_id: layer_id.into(),
            user_id: user_id.into(),
            created_at: created_at.into(),
            action: SharedDocumentAction::Redo,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SharedDocumentRuntime {
    pub document: SharedDocumentFile,
    pub actions: Vec<SharedDocumentActionRecord>,
    layer_canvases: BTreeMap<String, Canvas>,
    applied_action_ids_by_layer: BTreeMap<String, Vec<String>>,
    undone_action_ids_by_layer: BTreeMap<String, Vec<String>>,
    patches_by_action_id: BTreeMap<String, Vec<SharedCellPatch>>,
}

impl SharedDocumentRuntime {
    pub fn new(document: SharedDocumentFile) -> Self {
        let mut layer_canvases = BTreeMap::new();
        let mut applied_action_ids_by_layer = BTreeMap::new();
        let mut undone_action_ids_by_layer = BTreeMap::new();
        for layer in &document.layers {
            layer_canvases.insert(layer.layer_id.clone(), Canvas::new());
            applied_action_ids_by_layer.insert(layer.layer_id.clone(), Vec::new());
            undone_action_ids_by_layer.insert(layer.layer_id.clone(), Vec::new());
        }
        Self {
            document,
            actions: Vec::new(),
            layer_canvases,
            applied_action_ids_by_layer,
            undone_action_ids_by_layer,
            patches_by_action_id: BTreeMap::new(),
        }
    }

    pub fn replay(document: SharedDocumentFile, actions: Vec<SharedDocumentActionRecord>) -> Self {
        let mut runtime = Self::new(document);
        for action in actions {
            runtime.apply_action_record(action);
        }
        runtime
    }

    pub fn canvas_for_layer(&self, layer_id: &str) -> Option<&Canvas> {
        self.layer_canvases.get(layer_id)
    }

    pub fn layers(&self) -> &[SharedDocumentLayer] {
        &self.document.layers
    }

    pub fn property_track(&self, layer_id: &str, property_id: &str) -> Option<&SharedDocumentPropertyTrack> {
        self.document
            .layers
            .iter()
            .find(|layer| layer.layer_id == layer_id)?
            .property_tracks
            .iter()
            .find(|track| track.property_id == property_id)
    }

    fn ensure_property_track_mut(
        &mut self,
        layer_id: &str,
        property_id: &str,
    ) -> Option<&mut SharedDocumentPropertyTrack> {
        let layer = self
            .document
            .layers
            .iter_mut()
            .find(|layer| layer.layer_id == layer_id)?;
        if !layer
            .property_tracks
            .iter()
            .any(|track| track.property_id == property_id)
        {
            let track = if property_id == "raster" {
                default_raster_property_track(layer.start_breath, layer.length_breaths)
            } else {
                SharedDocumentPropertyTrack {
                    property_id: property_id.to_string(),
                    blocks: Vec::new(),
                }
            };
            layer.property_tracks.push(track);
        }
        layer
            .property_tracks
            .iter_mut()
            .find(|track| track.property_id == property_id)
    }

    pub fn add_layer(
        &mut self,
        layer_id: impl Into<String>,
        name: impl Into<String>,
    ) -> Option<&SharedDocumentLayer> {
        let layer_id = layer_id.into();
        if self.document.has_layer(&layer_id) {
            return self
                .document
                .layers
                .iter()
                .find(|layer| layer.layer_id == layer_id);
        }

        self.document.layers.push(SharedDocumentLayer {
            layer_id: layer_id.clone(),
            name: name.into(),
            visible: true,
            locked: false,
            start_breath: 0,
            length_breaths: default_layer_length_breaths(),
            property_tracks: vec![default_raster_property_track(0, default_layer_length_breaths())],
        });
        self.layer_canvases.insert(layer_id.clone(), Canvas::new());
        self.applied_action_ids_by_layer
            .insert(layer_id.clone(), Vec::new());
        self.undone_action_ids_by_layer
            .insert(layer_id.clone(), Vec::new());
        self.document
            .layers
            .iter()
            .find(|layer| layer.layer_id == layer_id)
    }

    /// Removes a layer's document entry and its canvas/undo bookkeeping.
    /// Action-record history for the layer is left in place (it stays
    /// meaningful for anyone replaying the raw log) but no longer surfaces
    /// through `layers()` or compositing. Returns `false` if no such layer
    /// exists.
    pub fn remove_layer(&mut self, layer_id: &str) -> bool {
        let before = self.document.layers.len();
        self.document.layers.retain(|layer| layer.layer_id != layer_id);
        if self.document.layers.len() == before {
            return false;
        }
        self.layer_canvases.remove(layer_id);
        self.applied_action_ids_by_layer.remove(layer_id);
        self.undone_action_ids_by_layer.remove(layer_id);
        true
    }

    /// Renames a layer's display name in place. Returns `false` if no such
    /// layer exists.
    pub fn rename_layer(&mut self, layer_id: &str, name: impl Into<String>) -> bool {
        let Some(layer) = self
            .document
            .layers
            .iter_mut()
            .find(|layer| layer.layer_id == layer_id)
        else {
            return false;
        };
        layer.name = name.into();
        true
    }

    /// Sets a layer's visibility, which controls whether it contributes to
    /// `composited_canvas_in_layer_order`. Returns `false` if no such layer
    /// exists.
    pub fn set_layer_visible(&mut self, layer_id: &str, visible: bool) -> bool {
        let Some(layer) = self
            .document
            .layers
            .iter_mut()
            .find(|layer| layer.layer_id == layer_id)
        else {
            return false;
        };
        layer.visible = visible;
        true
    }

    /// Sets a layer's lock state. Locking is advisory bookkeeping only here —
    /// enforcing it against paint edits belongs to the caller that owns the
    /// active-layer/tool routing. Returns `false` if no such layer exists.
    pub fn set_layer_locked(&mut self, layer_id: &str, locked: bool) -> bool {
        let Some(layer) = self
            .document
            .layers
            .iter_mut()
            .find(|layer| layer.layer_id == layer_id)
        else {
            return false;
        };
        layer.locked = locked;
        true
    }

    /// Sets the breath range shown by a layer's own timeline bar in the layers panel. Purely an
    /// authoring/UI concept for now — it does not gate compositing. Returns `false` if no such
    /// layer exists.
    pub fn set_layer_timing(&mut self, layer_id: &str, start_breath: u32, length_breaths: u32) -> bool {
        let Some(layer) = self
            .document
            .layers
            .iter_mut()
            .find(|layer| layer.layer_id == layer_id)
        else {
            return false;
        };
        layer.start_breath = start_breath;
        layer.length_breaths = length_breaths.max(1);
        true
    }

    pub fn set_property_block_timing(
        &mut self,
        layer_id: &str,
        property_id: &str,
        block_id: &str,
        start_breath: u32,
        length_breaths: u32,
    ) -> bool {
        let Some(track) = self.ensure_property_track_mut(layer_id, property_id) else {
            return false;
        };
        let Some(block) = track.blocks.iter_mut().find(|block| block.id == block_id) else {
            return false;
        };
        block.start_breath = start_breath;
        block.length_breaths = length_breaths.max(1);
        true
    }

    pub fn split_property_block(
        &mut self,
        layer_id: &str,
        property_id: &str,
        block_id: &str,
        split_breath: u32,
    ) -> bool {
        let Some(track) = self.ensure_property_track_mut(layer_id, property_id) else {
            return false;
        };
        let Some(index) = track.blocks.iter().position(|block| block.id == block_id) else {
            return false;
        };
        let block = track.blocks[index].clone();
        if block.length_breaths <= 1 {
            return false;
        }
        let block_end = block.start_breath + block.length_breaths - 1;
        if split_breath <= block.start_breath || split_breath > block_end {
            return false;
        }

        let left_length = split_breath - block.start_breath;
        let right_start = split_breath;
        let right_length = block_end - split_breath + 1;
        if left_length == 0 || right_length == 0 {
            return false;
        }

        track.blocks[index].length_breaths = left_length;
        let next_id = next_property_block_id(&track.blocks);
        track.blocks.insert(
            index + 1,
            SharedDocumentPropertyBlock {
                id: next_id,
                start_breath: right_start,
                length_breaths: right_length,
                is_blank: block.is_blank,
            },
        );
        true
    }

    /// Turns a content block into a blank placeholder covering the same breath range, leaving
    /// the track's coverage continuous. Returns `false` if no such block exists.
    pub fn blank_property_block(&mut self, layer_id: &str, property_id: &str, block_id: &str) -> bool {
        let Some(track) = self.ensure_property_track_mut(layer_id, property_id) else {
            return false;
        };
        let Some(block) = track.blocks.iter_mut().find(|block| block.id == block_id) else {
            return false;
        };
        block.is_blank = true;
        true
    }

    /// Merges a blank block into its content neighbor on `direction`, extending that neighbor
    /// to cover the blank's range and removing the blank block. Returns `false` if the block
    /// isn't blank or has no content neighbor on that side.
    pub fn merge_blank_property_block(
        &mut self,
        layer_id: &str,
        property_id: &str,
        block_id: &str,
        direction: PropertyBlockMergeDirection,
    ) -> bool {
        let Some(track) = self.ensure_property_track_mut(layer_id, property_id) else {
            return false;
        };
        let Some(index) = track.blocks.iter().position(|block| block.id == block_id) else {
            return false;
        };
        if !track.blocks[index].is_blank {
            return false;
        }
        let blank_start = track.blocks[index].start_breath;
        let blank_end = blank_start + track.blocks[index].length_breaths.max(1) - 1;
        match direction {
            PropertyBlockMergeDirection::Left => {
                let Some(previous) = index.checked_sub(1).map(|i| &mut track.blocks[i]) else {
                    return false;
                };
                if previous.is_blank {
                    return false;
                }
                let previous_start = previous.start_breath;
                previous.length_breaths = blank_end.max(previous_start) - previous_start + 1;
                track.blocks.remove(index);
            }
            PropertyBlockMergeDirection::Right => {
                let Some(next) = track.blocks.get_mut(index + 1) else {
                    return false;
                };
                if next.is_blank {
                    return false;
                }
                let next_end = next.start_breath + next.length_breaths.max(1) - 1;
                next.start_breath = blank_start.min(next.start_breath);
                next.length_breaths = next_end.max(blank_start) - next.start_breath + 1;
                track.blocks.remove(index);
            }
        }
        true
    }

    /// Swaps the breath range of two blocks in the same property track. Returns `false` if
    /// either block is missing or they are the same block.
    pub fn swap_property_blocks(
        &mut self,
        layer_id: &str,
        property_id: &str,
        source_block_id: &str,
        target_block_id: &str,
    ) -> bool {
        if source_block_id == target_block_id {
            return false;
        }
        let Some(track) = self.ensure_property_track_mut(layer_id, property_id) else {
            return false;
        };
        let Some(source_index) = track.blocks.iter().position(|block| block.id == source_block_id) else {
            return false;
        };
        let Some(target_index) = track.blocks.iter().position(|block| block.id == target_block_id) else {
            return false;
        };
        let source_span = (track.blocks[source_index].start_breath, track.blocks[source_index].length_breaths);
        let target_span = (track.blocks[target_index].start_breath, track.blocks[target_index].length_breaths);
        track.blocks[source_index].start_breath = target_span.0;
        track.blocks[source_index].length_breaths = target_span.1;
        track.blocks[target_index].start_breath = source_span.0;
        track.blocks[target_index].length_breaths = source_span.1;
        true
    }

    pub fn composited_canvas_in_layer_order(&self) -> Canvas {
        let mut canvas = Canvas::new();
        for layer in self.document.layers.iter().filter(|layer| layer.visible) {
            if let Some(layer_canvas) = self.layer_canvases.get(&layer.layer_id) {
                for (position, painted_cell) in layer_canvas {
                    canvas.insert(*position, painted_cell.clone());
                }
            }
        }
        canvas
    }

    pub fn apply_action_record(&mut self, record: SharedDocumentActionRecord) {
        let layer_id = record.layer_id.clone();
        match &record.action {
            SharedDocumentAction::CellPatchSet { patches } => {
                if !patches.is_empty() {
                    let canvas = self.layer_canvases.entry(layer_id.clone()).or_default();
                    apply_patches(canvas, patches, PatchDirection::After);
                    self.applied_action_ids_by_layer
                        .entry(layer_id.clone())
                        .or_default()
                        .push(record.action_id.clone());
                    self.undone_action_ids_by_layer
                        .entry(layer_id)
                        .or_default()
                        .clear();
                    self.patches_by_action_id
                        .insert(record.action_id.clone(), patches.clone());
                }
            }
            SharedDocumentAction::Undo => {
                if let Some(action_id) = self
                    .applied_action_ids_by_layer
                    .entry(layer_id.clone())
                    .or_default()
                    .pop()
                {
                    if let Some(patches) = self.patches_by_action_id.get(&action_id) {
                        let canvas = self.layer_canvases.entry(layer_id.clone()).or_default();
                        apply_patches(canvas, patches, PatchDirection::Before);
                        self.undone_action_ids_by_layer
                            .entry(layer_id)
                            .or_default()
                            .push(action_id);
                    }
                }
            }
            SharedDocumentAction::Redo => {
                if let Some(action_id) = self
                    .undone_action_ids_by_layer
                    .entry(layer_id.clone())
                    .or_default()
                    .pop()
                {
                    if let Some(patches) = self.patches_by_action_id.get(&action_id) {
                        let canvas = self.layer_canvases.entry(layer_id.clone()).or_default();
                        apply_patches(canvas, patches, PatchDirection::After);
                        self.applied_action_ids_by_layer
                            .entry(layer_id)
                            .or_default()
                            .push(action_id);
                    }
                }
            }
        }
        self.actions.push(record);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PatchDirection {
    Before,
    After,
}

fn apply_patches(canvas: &mut Canvas, patches: &[SharedCellPatch], direction: PatchDirection) {
    for patch in patches {
        let value = match direction {
            PatchDirection::Before => patch.before.as_ref(),
            PatchDirection::After => patch.after.as_ref(),
        };
        let position = patch.position.to_runtime();
        match value {
            Some(cell) => {
                canvas.insert(position, cell.to_runtime());
            }
            None => {
                canvas.remove(&position);
            }
        }
    }
}

pub fn load_or_create_shared_document(
    paths: &SharedDocumentPaths,
    default_document: SharedDocumentFile,
) -> Result<SharedDocumentRuntime> {
    fs::create_dir_all(&paths.root).with_context(|| {
        format!(
            "failed to create shared document directory {}",
            paths.root.display()
        )
    })?;
    if !paths.document_file_path.exists() {
        write_document_atomic(&paths.document_file_path, &default_document)?;
    }
    let document = load_document_file(&paths.document_file_path)?;
    let actions = load_action_records(&paths.actions_file_path)?;
    Ok(SharedDocumentRuntime::replay(document, actions))
}

pub fn load_document_file(path: &Path) -> Result<SharedDocumentFile> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("failed to read shared document file at {}", path.display()))?;
    serde_json::from_str(&text)
        .with_context(|| format!("failed to parse shared document JSON at {}", path.display()))
}

pub fn write_document_atomic(path: &Path, document: &SharedDocumentFile) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create shared document parent directory {}",
                parent.display()
            )
        })?;
    }
    let temp_path = path.with_extension("json.tmp");
    let text = serde_json::to_string_pretty(document)
        .context("failed to serialize shared document file")?;
    fs::write(&temp_path, text).with_context(|| {
        format!(
            "failed to write temporary shared document file at {}",
            temp_path.display()
        )
    })?;
    fs::rename(&temp_path, path).with_context(|| {
        format!(
            "failed to replace shared document file at {}",
            path.display()
        )
    })
}

pub fn load_action_records(path: &Path) -> Result<Vec<SharedDocumentActionRecord>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = File::open(path)
        .with_context(|| format!("failed to open shared actions file at {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut actions = Vec::new();
    for (index, line) in reader.lines().enumerate() {
        let line = line.with_context(|| {
            format!(
                "failed reading shared action line {} from {}",
                index + 1,
                path.display()
            )
        })?;
        if line.trim().is_empty() {
            continue;
        }
        let action = serde_json::from_str(&line).with_context(|| {
            format!(
                "failed parsing shared action line {} from {}",
                index + 1,
                path.display()
            )
        })?;
        actions.push(action);
    }
    Ok(actions)
}

pub fn append_action_record(path: &Path, action: &SharedDocumentActionRecord) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create shared action parent directory {}",
                parent.display()
            )
        })?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("failed to open shared actions file at {}", path.display()))?;
    let line = serde_json::to_string(action).context("failed to serialize shared action record")?;
    writeln!(file, "{line}")
        .with_context(|| format!("failed to append shared action to {}", path.display()))
}

pub fn write_action_records_atomic(
    path: &Path,
    actions: &[SharedDocumentActionRecord],
) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create shared action parent directory {}",
                parent.display()
            )
        })?;
    }
    let temp_path = path.with_extension("jsonl.tmp");
    let mut text = String::new();
    for action in actions {
        let line =
            serde_json::to_string(action).context("failed to serialize shared action record")?;
        text.push_str(&line);
        text.push('\n');
    }
    fs::write(&temp_path, text).with_context(|| {
        format!(
            "failed to write temporary shared actions file at {}",
            temp_path.display()
        )
    })?;
    fs::rename(&temp_path, path).with_context(|| {
        format!(
            "failed to replace shared actions file at {}",
            path.display()
        )
    })
}

pub fn save_shared_document_snapshot(
    paths: &SharedDocumentPaths,
    runtime: &SharedDocumentRuntime,
) -> Result<()> {
    fs::create_dir_all(&paths.root).with_context(|| {
        format!(
            "failed to create shared document directory {}",
            paths.root.display()
        )
    })?;
    write_document_atomic(&paths.document_file_path, &runtime.document)?;
    write_action_records_atomic(&paths.actions_file_path, &runtime.actions)
}

impl PersistedSharedPaintColor {
    fn from_runtime(color: PaintColor) -> Self {
        match color {
            PaintColor::FlatRgb(red, green, blue) => Self::FlatRgb { red, green, blue },
            PaintColor::Material(material) => Self::Material {
                material: material_name(material).to_string(),
            },
        }
    }

    fn to_runtime(&self) -> PaintColor {
        match self {
            Self::FlatRgb { red, green, blue } => PaintColor::flat_rgb(*red, *green, *blue),
            Self::Material { material } => material_from_name(material)
                .map(PaintColor::material)
                .unwrap_or_default(),
        }
    }
}

impl PersistedSharedGraphic {
    fn from_runtime(graphic: &CellGraphic) -> Self {
        match graphic {
            CellGraphic::None => Self::None,
            CellGraphic::Glyph(glyph) => Self::Glyph { glyph: *glyph },
            CellGraphic::Sprite(sprite) => Self::Sprite {
                atlas_relative_path: sprite.atlas_relative_path().to_string_lossy().into_owned(),
            },
        }
    }

    fn to_runtime(&self) -> CellGraphic {
        match self {
            Self::None => CellGraphic::None,
            Self::Glyph { glyph } => CellGraphic::Glyph(*glyph),
            Self::Sprite {
                atlas_relative_path,
            } => CellGraphic::Sprite(SpriteGraphic::new(atlas_relative_path)),
        }
    }
}

fn material_name(material: CellMaterialId) -> &'static str {
    match material {
        CellMaterialId::GrayScale => "gray-scale",
    }
}

fn material_from_name(name: &str) -> Option<CellMaterialId> {
    match name {
        "gray-scale" => Some(CellMaterialId::GrayScale),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use thaum_renderer_domain::CellGraphic;

    fn point(x: i32, y: i32) -> CellPoint {
        CellPoint { x, y, z: 0 }
    }

    fn cell(glyph: char) -> PaintedCell {
        PaintedCell {
            graphic: CellGraphic::Glyph(glyph),
            color: PaintColor::flat_rgb(255, 255, 255),
            weight_index: 1,
        }
    }

    #[test]
    fn patch_actions_replay_undo_and_redo_for_one_layer() {
        let document = SharedDocumentFile::single_layer("doc-1", "Doc", "layer-1", "Layer 1");
        let mut runtime = SharedDocumentRuntime::new(document.clone());
        runtime.apply_action_record(SharedDocumentActionRecord::cell_patch_set(
            "a1",
            "doc-1",
            "layer-1",
            "u1",
            "1",
            vec![SharedCellPatch::new(point(1, 1), None, Some(&cell('#')))],
        ));
        assert_eq!(runtime.canvas_for_layer("layer-1").unwrap().len(), 1);

        runtime.apply_action_record(SharedDocumentActionRecord::undo(
            "a2", "doc-1", "layer-1", "u2", "2",
        ));
        assert!(runtime.canvas_for_layer("layer-1").unwrap().is_empty());

        runtime.apply_action_record(SharedDocumentActionRecord::redo(
            "a3", "doc-1", "layer-1", "u1", "3",
        ));
        assert_eq!(
            runtime
                .canvas_for_layer("layer-1")
                .unwrap()
                .get(&point(1, 1)),
            Some(&cell('#'))
        );

        let replayed = SharedDocumentRuntime::replay(document, runtime.actions.clone());
        assert_eq!(
            replayed.canvas_for_layer("layer-1"),
            runtime.canvas_for_layer("layer-1")
        );
    }

    #[test]
    fn undo_is_isolated_per_layer() {
        let document = SharedDocumentFile {
            file_kind: SHARED_DOCUMENT_KIND.to_string(),
            schema_version: SHARED_DOCUMENT_SCHEMA_VERSION,
            document_id: "doc-1".to_string(),
            title: "Doc".to_string(),
            layers: vec![
                SharedDocumentLayer {
                    layer_id: "layer-1".to_string(),
                    name: "Layer 1".to_string(),
                    visible: true,
                    locked: false,
                    start_breath: 0,
                    length_breaths: default_layer_length_breaths(),
                    property_tracks: vec![default_raster_property_track(0, default_layer_length_breaths())],
                },
                SharedDocumentLayer {
                    layer_id: "layer-2".to_string(),
                    name: "Layer 2".to_string(),
                    visible: true,
                    locked: false,
                    start_breath: 0,
                    length_breaths: default_layer_length_breaths(),
                    property_tracks: vec![default_raster_property_track(0, default_layer_length_breaths())],
                },
            ],
        };
        let mut runtime = SharedDocumentRuntime::new(document);
        runtime.apply_action_record(SharedDocumentActionRecord::cell_patch_set(
            "a1",
            "doc-1",
            "layer-1",
            "u1",
            "1",
            vec![SharedCellPatch::new(point(0, 0), None, Some(&cell('A')))],
        ));
        runtime.apply_action_record(SharedDocumentActionRecord::cell_patch_set(
            "a2",
            "doc-1",
            "layer-2",
            "u1",
            "2",
            vec![SharedCellPatch::new(point(0, 0), None, Some(&cell('B')))],
        ));

        runtime.apply_action_record(SharedDocumentActionRecord::undo(
            "a3", "doc-1", "layer-1", "u2", "3",
        ));

        assert!(runtime.canvas_for_layer("layer-1").unwrap().is_empty());
        assert_eq!(
            runtime
                .canvas_for_layer("layer-2")
                .unwrap()
                .get(&point(0, 0)),
            Some(&cell('B'))
        );
    }

    #[test]
    fn invalid_undo_is_a_no_op() {
        let document = SharedDocumentFile::single_layer("doc-1", "Doc", "layer-1", "Layer 1");
        let mut runtime = SharedDocumentRuntime::new(document);
        runtime.apply_action_record(SharedDocumentActionRecord::undo(
            "a1", "doc-1", "layer-1", "u1", "1",
        ));
        assert!(runtime.canvas_for_layer("layer-1").unwrap().is_empty());
        assert_eq!(runtime.actions.len(), 1);
    }

    #[test]
    fn add_layer_initializes_runtime_state_and_preserves_order() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));

        let added = runtime.add_layer("layer-2", "Layer 2").unwrap();

        assert_eq!(added.layer_id, "layer-2");
        assert_eq!(runtime.layers().len(), 2);
        assert_eq!(runtime.layers()[1].name, "Layer 2");
        assert!(runtime.canvas_for_layer("layer-2").unwrap().is_empty());
    }

    #[test]
    fn composited_canvas_uses_document_layer_order() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));
        runtime.add_layer("layer-2", "Layer 2");
        runtime.apply_action_record(SharedDocumentActionRecord::cell_patch_set(
            "a1",
            "doc-1",
            "layer-1",
            "u1",
            "1",
            vec![SharedCellPatch::new(point(0, 0), None, Some(&cell('A')))],
        ));
        runtime.apply_action_record(SharedDocumentActionRecord::cell_patch_set(
            "a2",
            "doc-1",
            "layer-2",
            "u1",
            "2",
            vec![SharedCellPatch::new(point(0, 0), None, Some(&cell('B')))],
        ));

        let composited = runtime.composited_canvas_in_layer_order();

        assert_eq!(composited.get(&point(0, 0)), Some(&cell('B')));
    }

    #[test]
    fn save_shared_document_snapshot_creates_document_and_actions_files() {
        let unique = format!(
            "thaum-painter-storage-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        let paths = SharedDocumentPaths::new(root.clone());
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));
        runtime.apply_action_record(SharedDocumentActionRecord::cell_patch_set(
            "a1",
            "doc-1",
            "layer-1",
            "u1",
            "1",
            vec![SharedCellPatch::new(point(1, 1), None, Some(&cell('#')))],
        ));

        save_shared_document_snapshot(&paths, &runtime).unwrap();

        assert!(paths.document_file_path.exists());
        assert!(paths.actions_file_path.exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn set_layer_visible_excludes_a_hidden_layer_from_compositing() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));
        runtime.add_layer("layer-2", "Layer 2");
        runtime.apply_action_record(SharedDocumentActionRecord::cell_patch_set(
            "a1",
            "doc-1",
            "layer-1",
            "u1",
            "1",
            vec![SharedCellPatch::new(point(0, 0), None, Some(&cell('A')))],
        ));

        assert!(runtime.set_layer_visible("layer-1", false));
        assert!(runtime.composited_canvas_in_layer_order().is_empty());

        assert!(runtime.set_layer_visible("layer-1", true));
        assert_eq!(
            runtime.composited_canvas_in_layer_order().get(&point(0, 0)),
            Some(&cell('A'))
        );

        assert!(!runtime.set_layer_visible("missing-layer", false));
    }

    #[test]
    fn set_layer_locked_updates_the_layer_entry() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));

        assert!(runtime.set_layer_locked("layer-1", true));
        assert!(runtime.layers()[0].locked);
        assert!(!runtime.set_layer_locked("missing-layer", true));
    }

    #[test]
    fn rename_layer_updates_the_layer_entry() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));

        assert!(runtime.rename_layer("layer-1", "Renamed"));
        assert_eq!(runtime.layers()[0].name, "Renamed");
        assert!(!runtime.rename_layer("missing-layer", "Nope"));
    }

    #[test]
    fn remove_layer_drops_the_layer_and_its_canvas() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));
        runtime.add_layer("layer-2", "Layer 2");

        assert!(runtime.remove_layer("layer-1"));
        assert_eq!(runtime.layers().len(), 1);
        assert_eq!(runtime.layers()[0].layer_id, "layer-2");
        assert!(runtime.canvas_for_layer("layer-1").is_none());
        assert!(!runtime.remove_layer("layer-1"));
    }

    #[test]
    fn set_layer_timing_updates_start_and_length_and_enforces_a_minimum_length() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));
        assert_eq!(runtime.layers()[0].start_breath, 0);
        assert_eq!(runtime.layers()[0].length_breaths, 24);

        assert!(runtime.set_layer_timing("layer-1", 5, 10));
        assert_eq!(runtime.layers()[0].start_breath, 5);
        assert_eq!(runtime.layers()[0].length_breaths, 10);

        assert!(runtime.set_layer_timing("layer-1", 5, 0));
        assert_eq!(runtime.layers()[0].length_breaths, 1);

        assert!(!runtime.set_layer_timing("missing-layer", 0, 1));
    }

    #[test]
    fn split_property_block_creates_two_ordered_blocks() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));

        assert!(runtime.split_property_block("layer-1", "raster", "block-1", 8));
        let blocks = &runtime.property_track("layer-1", "raster").unwrap().blocks;

        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].id, "block-1");
        assert_eq!(blocks[0].start_breath, 0);
        assert_eq!(blocks[0].length_breaths, 8);
        assert_eq!(blocks[1].start_breath, 8);
        assert_eq!(blocks[1].length_breaths, 16);
    }

    #[test]
    fn set_property_block_timing_updates_one_block() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));

        assert!(runtime.set_property_block_timing("layer-1", "raster", "block-1", 3, 7));
        let block = &runtime.property_track("layer-1", "raster").unwrap().blocks[0];

        assert_eq!(block.start_breath, 3);
        assert_eq!(block.length_breaths, 7);
    }

    #[test]
    fn blank_property_block_marks_a_content_block_as_blank() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));

        assert!(runtime.blank_property_block("layer-1", "raster", "block-1"));
        let block = &runtime.property_track("layer-1", "raster").unwrap().blocks[0];
        assert!(block.is_blank);

        assert!(!runtime.blank_property_block("layer-1", "raster", "missing-block"));
    }

    #[test]
    fn merge_blank_property_block_left_extends_the_previous_content_block() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));
        runtime.split_property_block("layer-1", "raster", "block-1", 8);
        runtime.blank_property_block("layer-1", "raster", "block-2");

        assert!(runtime.merge_blank_property_block(
            "layer-1",
            "raster",
            "block-2",
            PropertyBlockMergeDirection::Left,
        ));
        let blocks = &runtime.property_track("layer-1", "raster").unwrap().blocks;
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].id, "block-1");
        assert_eq!(blocks[0].start_breath, 0);
        assert_eq!(blocks[0].length_breaths, 24);
        assert!(!blocks[0].is_blank);
    }

    #[test]
    fn merge_blank_property_block_right_extends_the_next_content_block() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));
        runtime.split_property_block("layer-1", "raster", "block-1", 8);
        runtime.blank_property_block("layer-1", "raster", "block-1");

        assert!(runtime.merge_blank_property_block(
            "layer-1",
            "raster",
            "block-1",
            PropertyBlockMergeDirection::Right,
        ));
        let blocks = &runtime.property_track("layer-1", "raster").unwrap().blocks;
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].id, "block-2");
        assert_eq!(blocks[0].start_breath, 0);
        assert_eq!(blocks[0].length_breaths, 24);
        assert!(!blocks[0].is_blank);
    }

    #[test]
    fn merge_blank_property_block_fails_without_a_content_neighbor_on_that_side() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));
        runtime.blank_property_block("layer-1", "raster", "block-1");

        assert!(!runtime.merge_blank_property_block(
            "layer-1",
            "raster",
            "block-1",
            PropertyBlockMergeDirection::Left,
        ));
        assert!(!runtime.merge_blank_property_block(
            "layer-1",
            "raster",
            "block-1",
            PropertyBlockMergeDirection::Right,
        ));
    }

    #[test]
    fn swap_property_blocks_exchanges_their_breath_ranges() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));
        runtime.split_property_block("layer-1", "raster", "block-1", 8);

        assert!(runtime.swap_property_blocks("layer-1", "raster", "block-1", "block-2"));
        let blocks = &runtime.property_track("layer-1", "raster").unwrap().blocks;
        assert_eq!(blocks[0].start_breath, 8);
        assert_eq!(blocks[0].length_breaths, 16);
        assert_eq!(blocks[1].start_breath, 0);
        assert_eq!(blocks[1].length_breaths, 8);

        assert!(!runtime.swap_property_blocks("layer-1", "raster", "block-1", "block-1"));
        assert!(!runtime.swap_property_blocks("layer-1", "raster", "block-1", "missing"));
    }
}
