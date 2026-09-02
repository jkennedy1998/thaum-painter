use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use thaum_renderer_domain::{CellGraphic, CellMaterialId, CellPoint, SpriteGraphic};

use crate::properties::{
    breath_in_span, clamped_breath_span, destructive_breath_span, pushed_breath_span,
};
use crate::{Canvas, PaintColor, PaintedCell};

pub const SHARED_DOCUMENT_KIND: &str = "thaum-painter-shared-document";
pub const SHARED_DOCUMENT_SCHEMA_VERSION: u32 = 1;

/// The one selection channel that exists until channel-picker UI arrives. Selection
/// channels are document-owned (per file, not per layer) so every user edits the same
/// 3D bitmap; later channels add private/shared selection layers on the same seam.
pub const DEFAULT_SELECTION_CHANNEL_ID: &str = "selection";

/// How many recent edit records stay individually undoable before older ones are
/// squashed into the document snapshot on the next save. A program-level constant,
/// not per-user config: in multiplayer every user shares one document history, so
/// the depth must be stable across sessions and machines.
pub const UNDO_HISTORY_DEPTH: usize = 20;

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
    /// Monotonic save counter. Every snapshot save verifies the on-disk revision
    /// matches the writer's loaded revision before overwriting, so a second writer
    /// (another user / another app instance) cannot silently clobber changes —
    /// the split-brain guard between document.json (structure) and actions.jsonl
    /// (content).
    #[serde(default)]
    pub revision: u64,
    pub title: String,
    pub layers: Vec<SharedDocumentLayer>,
    /// Document-owned selection bitmaps, keyed by channel. Selection is per file, not
    /// per layer: one 3D bitmap on the same coordinate system as the canvas, editable
    /// by any user. Not part of the per-layer action log — selection changes persist
    /// through the document snapshot, not undo records.
    #[serde(default)]
    pub selection: SharedDocumentSelection,
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
            revision: 0,
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
            selection: SharedDocumentSelection::default(),
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

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
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

/// One named selection bitmap: a set of 3D canvas points on the same coordinate
/// system as painted cells. Channels exist so a file can later carry several
/// independent selections (private/shared); today only the default channel exists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedDocumentSelectionChannel {
    pub channel_id: String,
    pub points: BTreeSet<PersistedCellPoint>,
}

/// Document-owned selection state. Lives on the file (not the action log) because
/// selection is per file and shared across users; changes persist through the
/// document snapshot on the next save.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedDocumentSelection {
    pub channels: Vec<SharedDocumentSelectionChannel>,
}

impl Default for SharedDocumentSelection {
    fn default() -> Self {
        Self {
            channels: vec![SharedDocumentSelectionChannel {
                channel_id: DEFAULT_SELECTION_CHANNEL_ID.to_string(),
                points: BTreeSet::new(),
            }],
        }
    }
}

/// How an incoming point batch mutates a selection channel. Mirrors the interaction
/// modes the selection tooling already exposes (replace/add/subtract/intersect);
/// Replace overwrites the whole channel, so it is only written from full-set commits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SharedSelectionWriteMode {
    Replace,
    Additive,
    Subtract,
    Intersect,
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
    CellPatchSet {
        patches: Vec<SharedCellPatch>,
        /// Raster block the patches landed on. `None` for legacy records, which replay
        /// into the layer's first non-blank raster block.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        block_id: Option<String>,
    },
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
        block_id: Option<String>,
    ) -> Self {
        Self {
            action_id: action_id.into(),
            document_id: document_id.into(),
            layer_id: layer_id.into(),
            user_id: user_id.into(),
            created_at: created_at.into(),
            action: SharedDocumentAction::CellPatchSet { patches, block_id },
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

/// Resolved canvas target for one applied cell-patch action: the layer and raster
/// block whose canvas the patches landed on.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AppliedCellPatches {
    layer_id: String,
    block_id: String,
    patches: Vec<SharedCellPatch>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SharedDocumentRuntime {
    pub document: SharedDocumentFile,
    pub actions: Vec<SharedDocumentActionRecord>,
    /// Painted content per raster block, keyed by `(layer_id, block_id)`. Block
    /// metadata lives in `document`; canvases are runtime state rebuilt by replay.
    block_canvases: BTreeMap<(String, String), Canvas>,
    applied_action_ids_by_layer: BTreeMap<String, Vec<String>>,
    undone_action_ids_by_layer: BTreeMap<String, Vec<String>>,
    patches_by_action_id: BTreeMap<String, AppliedCellPatches>,
    /// The revision this runtime loaded (and last saved). Snapshots verify the
    /// on-disk revision still matches before overwriting.
    revision: u64,
}

impl SharedDocumentRuntime {
    pub fn new(document: SharedDocumentFile) -> Self {
        let revision = document.revision;
        let mut block_canvases = BTreeMap::new();
        let mut applied_action_ids_by_layer = BTreeMap::new();
        let mut undone_action_ids_by_layer = BTreeMap::new();
        for layer in &document.layers {
            if let Some(track) = layer
                .property_tracks
                .iter()
                .find(|track| track.property_id == "raster")
            {
                for block in &track.blocks {
                    block_canvases.insert(
                        (layer.layer_id.clone(), block.id.clone()),
                        Canvas::new(),
                    );
                }
            }
            applied_action_ids_by_layer.insert(layer.layer_id.clone(), Vec::new());
            undone_action_ids_by_layer.insert(layer.layer_id.clone(), Vec::new());
        }
        Self {
            document,
            actions: Vec::new(),
            block_canvases,
            applied_action_ids_by_layer,
            undone_action_ids_by_layer,
            patches_by_action_id: BTreeMap::new(),
            revision,
        }
    }

    /// The revision this runtime loaded and last saved. Compare against
    /// `load_document_file(..).revision` to detect another writer's changes.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn replay(document: SharedDocumentFile, actions: Vec<SharedDocumentActionRecord>) -> Self {
        let mut runtime = Self::new(document);
        for action in actions {
            runtime.apply_action_record(action);
        }
        runtime
    }

    /// The layer's raster block covering `current_breath`, if any. Blank blocks are
    /// valid targets: their canvas is empty, and painting into one un-blanks it.
    pub fn active_raster_block_id(&self, layer_id: &str, current_breath: u32) -> Option<String> {
        let layer = self
            .document
            .layers
            .iter()
            .find(|layer| layer.layer_id == layer_id)?;
        let track = layer
            .property_tracks
            .iter()
            .find(|track| track.property_id == "raster")?;
        track
            .blocks
            .iter()
            .find(|block| breath_in_span(current_breath, block.start_breath, block.length_breaths))
            .map(|block| block.id.clone())
    }

    /// The selected points of one selection channel, as runtime cell points.
    pub fn selection_points(&self, channel_id: &str) -> Vec<CellPoint> {
        self.selection_channel(channel_id)
            .map(|channel| channel.points.iter().map(PersistedCellPoint::to_runtime).collect())
            .unwrap_or_default()
    }

    /// Whether one selection channel contains a point.
    pub fn selection_contains(&self, channel_id: &str, point: CellPoint) -> bool {
        self.selection_channel(channel_id)
            .is_some_and(|channel| channel.points.contains(&point.into()))
    }

    /// Applies one batch of points to a selection channel. Creates the channel if it
    /// does not exist yet. Returns whether the channel actually changed, so callers
    /// can skip snapshot saves for no-op strokes.
    pub fn apply_selection_points<I>(
        &mut self,
        channel_id: &str,
        points: I,
        mode: SharedSelectionWriteMode,
    ) -> bool
    where
        I: IntoIterator<Item = CellPoint>,
    {
        let incoming: BTreeSet<PersistedCellPoint> = points
            .into_iter()
            .map(PersistedCellPoint::from)
            .collect();
        let channel = self.ensure_selection_channel_mut(channel_id);
        let before = channel.points.clone();
        match mode {
            SharedSelectionWriteMode::Replace => channel.points = incoming,
            SharedSelectionWriteMode::Additive => channel.points.extend(incoming),
            SharedSelectionWriteMode::Subtract => {
                for point in incoming {
                    channel.points.remove(&point);
                }
            }
            SharedSelectionWriteMode::Intersect => {
                channel.points = channel
                    .points
                    .intersection(&incoming)
                    .cloned()
                    .collect();
            }
        }
        channel.points != before
    }

    fn ensure_selection_channel_mut(
        &mut self,
        channel_id: &str,
    ) -> &mut SharedDocumentSelectionChannel {
        let channels = &mut self.document.selection.channels;
        if let Some(position) = channels
            .iter()
            .position(|channel| channel.channel_id == channel_id)
        {
            return &mut channels[position];
        }
        channels.push(SharedDocumentSelectionChannel {
            channel_id: channel_id.to_string(),
            points: BTreeSet::new(),
        });
        let last = channels.len() - 1;
        &mut channels[last]
    }

    fn selection_channel(&self, channel_id: &str) -> Option<&SharedDocumentSelectionChannel> {
        self.document
            .selection
            .channels
            .iter()
            .find(|channel| channel.channel_id == channel_id)
    }

    /// Legacy records carry no block id; replay them into the layer's first
    /// non-blank raster block (falling back to the first block) to keep the old
    /// single-canvas-per-layer content reachable.
    fn legacy_raster_block_target(&self, layer_id: &str) -> Option<(String, String)> {
        let layer = self
            .document
            .layers
            .iter()
            .find(|layer| layer.layer_id == layer_id)?;
        let track = layer
            .property_tracks
            .iter()
            .find(|track| track.property_id == "raster")?;
        let block = track
            .blocks
            .iter()
            .find(|block| !block.is_blank)
            .or_else(|| track.blocks.first())?;
        Some((layer_id.to_string(), block.id.clone()))
    }

    /// The layer's canvas for the raster block covering `current_breath`. Returns
    /// `None` when the layer does not exist or the breath sits in a gap between
    /// blocks (a gap renders nothing for that layer).
    pub fn canvas_for_layer(&self, layer_id: &str, current_breath: u32) -> Option<&Canvas> {
        let block_id = self.active_raster_block_id(layer_id, current_breath)?;
        self.block_canvases.get(&(layer_id.to_string(), block_id))
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
        self.block_canvases.insert((layer_id.clone(), "block-1".to_string()), Canvas::new());
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
        self.block_canvases
            .retain(|(canvas_layer_id, _), _| canvas_layer_id != layer_id);
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

    /// Reshapes one property block's breath range. A block may never cross another
    /// block in its channel: the requested range is clamped into the free window
    /// between the block's neighbors, so no two blocks in a channel can share a breath.
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
        let Some(index) = track.blocks.iter().position(|block| block.id == block_id) else {
            return false;
        };
        let original = (
            track.blocks[index].start_breath,
            track.blocks[index].length_breaths,
        );
        let others: Vec<(u32, u32)> = track
            .blocks
            .iter()
            .enumerate()
            .filter(|(other_index, _)| *other_index != index)
            .map(|(_, block)| (block.start_breath, block.length_breaths))
            .collect();
        let (start, length) = clamped_breath_span(
            &others,
            original,
            (start_breath, length_breaths.max(1)),
        );
        let block = &mut track.blocks[index];
        block.start_breath = start;
        block.length_breaths = length;
        true
    }

    /// Reshapes one property block with a time-preserving ripple (`pushed_breath_span`):
    /// the blocks on the dragged side of the channel shift by the same delta, so
    /// relative spacing is preserved and no gap or overlap can appear.
    pub fn set_property_block_timing_pushed(
        &mut self,
        layer_id: &str,
        property_id: &str,
        block_id: &str,
        start_breath: u32,
        length_breaths: u32,
    ) -> bool {
        let Some((track, index)) = self.property_track_block_mut(layer_id, property_id, block_id)
        else {
            return false;
        };
        let original = (
            track.blocks[index].start_breath,
            track.blocks[index].length_breaths,
        );
        // The span helpers index their results by position in the `others` slice, so
        // carry each slice position's track index alongside it and map results back.
        let other_spans: Vec<(u32, u32)> = track
            .blocks
            .iter()
            .enumerate()
            .filter(|(other_index, _)| *other_index != index)
            .map(|(_, block)| (block.start_breath, block.length_breaths))
            .collect();
        let other_track_indices: Vec<usize> = track
            .blocks
            .iter()
            .enumerate()
            .filter(|(other_index, _)| *other_index != index)
            .map(|(other_index, _)| other_index)
            .collect();
        let pushed =
            pushed_breath_span(&other_spans, original, (start_breath, length_breaths.max(1)));
        track.blocks[index].start_breath = pushed.edited.0;
        track.blocks[index].length_breaths = pushed.edited.1;
        for (other_index, (start, length)) in pushed.shifted {
            let track_index = other_track_indices[other_index];
            track.blocks[track_index].start_breath = start;
            track.blocks[track_index].length_breaths = length;
        }
        true
    }

    /// Reshapes one property block destructively (`destructive_breath_span`): the block
    /// takes its full requested span and covered neighbors yield — truncated, removed,
    /// or split around it. A shrink overlaps nothing, so it acts as a plain trim.
    pub fn set_property_block_timing_destructive(
        &mut self,
        layer_id: &str,
        property_id: &str,
        block_id: &str,
        start_breath: u32,
        length_breaths: u32,
    ) -> bool {
        // Plan pass over an immutable track: compute the destructive resolution and
        // snapshot the victims' canvases (split right fragments inherit the victim's
        // content, matching `split_property_block`'s both-halves-identical semantics).
        // The borrow must end before the mutable apply pass below.
        let plan = {
            let Some(track) = self.property_track(layer_id, property_id) else {
                return false;
            };
            let Some(index) = track.blocks.iter().position(|block| block.id == block_id) else {
                return false;
            };
            // Destructive resolution needs no anchor — the requested span is taken
            // as-is — so the block's original timing is intentionally not read here.
            // The span helpers index their results by position in the `others` slice,
            // so carry each slice position's track index alongside it and map results
            // back.
            let other_spans: Vec<(u32, u32)> = track
                .blocks
                .iter()
                .enumerate()
                .filter(|(other_index, _)| *other_index != index)
                .map(|(_, block)| (block.start_breath, block.length_breaths))
                .collect();
            let other_track_indices: Vec<usize> = track
                .blocks
                .iter()
                .enumerate()
                .filter(|(other_index, _)| *other_index != index)
                .map(|(other_index, _)| other_index)
                .collect();
            let destructive =
                destructive_breath_span(&other_spans, (start_breath, length_breaths.max(1)));
            let split_canvases: Vec<(usize, Canvas)> = destructive
                .splits
                .iter()
                .map(|(slice_index, _)| {
                    let victim_track_index = other_track_indices[*slice_index];
                    let victim = &track.blocks[victim_track_index];
                    let canvas = self
                        .block_canvases
                        .get(&(layer_id.to_string(), victim.id.clone()))
                        .cloned()
                        .unwrap_or_default();
                    (victim_track_index, canvas)
                })
                .collect();
            Some((destructive, other_track_indices, split_canvases))
        };
        let Some((destructive, other_track_indices, split_canvases)) = plan else {
            return false;
        };
        let Some((track, index)) = self.property_track_block_mut(layer_id, property_id, block_id)
        else {
            return false;
        };
        track.blocks[index].start_breath = destructive.edited.0;
        track.blocks[index].length_breaths = destructive.edited.1;
        for (other_index, (start, length)) in destructive.truncated {
            let track_index = other_track_indices[other_index];
            track.blocks[track_index].start_breath = start;
            track.blocks[track_index].length_breaths = length;
        }
        // Removals and splits shift indices, so apply them highest-index-first. A split
        // keeps its left piece under the old id and inserts a fresh right piece after
        // the edited span that inherits the victim's canvas — both halves start as
        // identical copies and diverge as they are edited separately.
        let mut splits_and_removals: Vec<(usize, Option<u32>)> = destructive
            .splits
            .into_iter()
            .map(|(slice_index, split_breath)| {
                (other_track_indices[slice_index], Some(split_breath))
            })
            .chain(
                destructive
                    .removed
                    .into_iter()
                    .map(|slice_index| (other_track_indices[slice_index], None)),
            )
            .collect();
        splits_and_removals.sort_by_key(|(index, _)| std::cmp::Reverse(*index));
        let mut dropped_canvas_keys = Vec::new();
        let mut inserted_canvases: Vec<(String, Canvas)> = Vec::new();
        for (other_index, split_breath) in splits_and_removals {
            let other = &track.blocks[other_index];
            let other_canvas_key = (layer_id.to_string(), other.id.clone());
            let other_end = other.start_breath + other.length_breaths;
            let other_is_blank = other.is_blank;
            match split_breath {
                Some(split_breath) => {
                    track.blocks[other_index].length_breaths = split_breath - other.start_breath;
                    let next_id = next_property_block_id(&track.blocks);
                    let edited_end = destructive.edited.0 + destructive.edited.1;
                    track.blocks.insert(
                        other_index + 1,
                        SharedDocumentPropertyBlock {
                            id: next_id.clone(),
                            start_breath: edited_end,
                            length_breaths: other_end - edited_end,
                            is_blank: other_is_blank,
                        },
                    );
                    let victim_canvas = split_canvases
                        .iter()
                        .find(|(victim_track_index, _)| *victim_track_index == other_index)
                        .map(|(_, canvas)| canvas.clone())
                        .unwrap_or_default();
                    inserted_canvases.push((next_id, victim_canvas));
                }
                None => {
                    track.blocks.remove(other_index);
                    dropped_canvas_keys.push(other_canvas_key);
                }
            }
        }
        for key in dropped_canvas_keys {
            self.block_canvases.remove(&key);
        }
        for (new_block_id, canvas) in inserted_canvases {
            self.block_canvases
                .insert((layer_id.to_string(), new_block_id), canvas);
        }
        true
    }

    /// Resolves one property block to a mutable borrow of its track plus the block's
    /// index, creating the track when missing.
    fn property_track_block_mut(
        &mut self,
        layer_id: &str,
        property_id: &str,
        block_id: &str,
    ) -> Option<(&mut SharedDocumentPropertyTrack, usize)> {
        let track = self.ensure_property_track_mut(layer_id, property_id)?;
        let index = track.blocks.iter().position(|block| block.id == block_id)?;
        Some((track, index))
    }

    /// Splits one property block at `split_breath`: the left half keeps the id and the
    /// breaths before the split, the new right half gets the rest. Returns the new
    /// right block's id. The channel's data is propagated onto the new half through
    /// `split_data_propagation_record` (the split duplicates the block's content, so
    /// both halves start identical and diverge as they are edited separately).
    pub fn split_property_block(
        &mut self,
        layer_id: &str,
        property_id: &str,
        block_id: &str,
        split_breath: u32,
    ) -> Option<String> {
        let Some(track) = self.ensure_property_track_mut(layer_id, property_id) else {
            return None;
        };
        let Some(index) = track.blocks.iter().position(|block| block.id == block_id) else {
            return None;
        };
        let block = track.blocks[index].clone();
        if block.length_breaths <= 1 {
            return None;
        }
        let block_end = block.start_breath + block.length_breaths - 1;
        if split_breath <= block.start_breath || split_breath > block_end {
            return None;
        }

        let left_length = split_breath - block.start_breath;
        let right_start = split_breath;
        let right_length = block_end - split_breath + 1;
        if left_length == 0 || right_length == 0 {
            return None;
        }

        track.blocks[index].length_breaths = left_length;
        let next_id = next_property_block_id(&track.blocks);
        track.blocks.insert(
            index + 1,
            SharedDocumentPropertyBlock {
                id: next_id.clone(),
                start_breath: right_start,
                length_breaths: right_length,
                is_blank: block.is_blank,
            },
        );
        // The new right half starts with its own (empty) canvas; the caller propagates
        // the split block's data onto it as a recorded patch so replay rebuilds the copy.
        self.block_canvases
            .insert((layer_id.to_string(), next_id.clone()), Canvas::new());
        Some(next_id)
    }

    /// Builds the full-cell patch record that propagates a split block's channel data
    /// onto its new half — for the raster channel that is the block's whole canvas, so
    /// the two halves start as identical copies at different breaths. The copy is a
    /// normal recorded patch, so live edits and replay build the same canvas and
    /// save/load keeps the propagated data. Returns `None` when the channel carries no
    /// data (empty canvas), leaving the halves empty.
    pub fn split_data_propagation_record(
        &self,
        layer_id: &str,
        source_block_id: &str,
        new_block_id: &str,
        action_id: String,
        user_id: &str,
        timestamp: String,
    ) -> Option<SharedDocumentActionRecord> {
        let canvas = self
            .block_canvases
            .get(&(layer_id.to_string(), source_block_id.to_string()))?;
        if canvas.is_empty() {
            return None;
        }
        let patches: Vec<SharedCellPatch> = canvas
            .iter()
            .map(|(position, cell)| SharedCellPatch::new(*position, None, Some(cell)))
            .collect();
        Some(SharedDocumentActionRecord::cell_patch_set(
            action_id,
            self.document.document_id.clone(),
            layer_id,
            user_id,
            timestamp,
            patches,
            Some(new_block_id.to_string()),
        ))
    }

    /// Turns a content block into a blank placeholder covering the same breath range, leaving
    /// the track's coverage continuous. The block's painted content is discarded — a blank
    /// block renders empty. Returns `false` if no such block exists.
    pub fn blank_property_block(&mut self, layer_id: &str, property_id: &str, block_id: &str) -> bool {
        let Some(track) = self.ensure_property_track_mut(layer_id, property_id) else {
            return false;
        };
        let Some(block) = track.blocks.iter_mut().find(|block| block.id == block_id) else {
            return false;
        };
        block.is_blank = true;
        self.block_canvases
            .insert((layer_id.to_string(), block_id.to_string()), Canvas::new());
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
        // The blank's canvas is empty by definition; the content neighbor keeps its own.
        self.block_canvases
            .remove(&(layer_id.to_string(), block_id.to_string()));
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
        // Painted content belongs to the block, so it follows the swap: canvases stay
        // keyed to their own block ids while the breath spans exchange. Swapping the
        // canvases too would put each block's content back where it started — a
        // visual no-op — so only the timing moves here.
        true
    }

    /// Flattens every visible layer's canvas for the breath into one canvas, in document
    /// layer order (later layers win on overlap).
    pub fn composited_canvas_in_layer_order(&self, current_breath: u32) -> Canvas {
        let mut canvas = Canvas::new();
        for layer in self.document.layers.iter().filter(|layer| layer.visible) {
            if let Some(layer_canvas) = self.canvas_for_layer(&layer.layer_id, current_breath) {
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
            SharedDocumentAction::CellPatchSet { patches, block_id } => {
                if !patches.is_empty() {
                    let target = match block_id {
                        Some(block_id) => Some((layer_id.clone(), block_id.clone())),
                        None => self.legacy_raster_block_target(&layer_id),
                    };
                    if let Some((target_layer_id, target_block_id)) = target {
                        // Painting into a blank block turns it back into content.
                        if let Some(layer) = self
                            .document
                            .layers
                            .iter_mut()
                            .find(|layer| layer.layer_id == target_layer_id)
                        {
                            if let Some(track) = layer
                                .property_tracks
                                .iter_mut()
                                .find(|track| track.property_id == "raster")
                            {
                                if let Some(block) = track
                                    .blocks
                                    .iter_mut()
                                    .find(|block| block.id == target_block_id)
                                {
                                    block.is_blank = false;
                                }
                            }
                        }
                        let canvas = self
                            .block_canvases
                            .entry((target_layer_id.clone(), target_block_id.clone()))
                            .or_default();
                        apply_patches(canvas, patches, PatchDirection::After);
                        self.applied_action_ids_by_layer
                            .entry(layer_id.clone())
                            .or_default()
                            .push(record.action_id.clone());
                        self.undone_action_ids_by_layer
                            .entry(layer_id)
                            .or_default()
                            .clear();
                        self.patches_by_action_id.insert(
                            record.action_id.clone(),
                            AppliedCellPatches {
                                layer_id: target_layer_id,
                                block_id: target_block_id,
                                patches: patches.clone(),
                            },
                        );
                    }
                }
            }
            SharedDocumentAction::Undo => {
                if let Some(action_id) = self
                    .applied_action_ids_by_layer
                    .entry(layer_id.clone())
                    .or_default()
                    .pop()
                {
                    if let Some(applied) = self.patches_by_action_id.get(&action_id) {
                        let canvas = self
                            .block_canvases
                            .entry((applied.layer_id.clone(), applied.block_id.clone()))
                            .or_default();
                        apply_patches(canvas, &applied.patches, PatchDirection::Before);
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
                    if let Some(applied) = self.patches_by_action_id.get(&action_id) {
                        let canvas = self
                            .block_canvases
                            .entry((applied.layer_id.clone(), applied.block_id.clone()))
                            .or_default();
                        apply_patches(canvas, &applied.patches, PatchDirection::After);
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
    runtime: &mut SharedDocumentRuntime,
) -> Result<()> {
    fs::create_dir_all(&paths.root).with_context(|| {
        format!(
            "failed to create shared document directory {}",
            paths.root.display()
        )
    })?;

    // Optimistic concurrency: refuse to overwrite a document that changed on disk
    // since this runtime loaded it. Without this, a second writer's structure
    // edits (document.json) and this writer's content (actions.jsonl) would
    // silently split the document's truth across both files.
    if paths.document_file_path.exists() {
        let on_disk = load_document_file(&paths.document_file_path)?;
        if on_disk.revision != runtime.revision {
            return Err(anyhow::anyhow!(
                "shared document changed on disk (disk revision {}, local revision {}); \
                 refusing to overwrite — reload the document to pick up the other writer's changes",
                on_disk.revision,
                runtime.revision
            ));
        }
    }

    let next_revision = runtime.revision + 1;
    let mut document = runtime.document.clone();
    document.revision = next_revision;
    write_document_atomic(&paths.document_file_path, &document)?;
    write_action_records_atomic(&paths.actions_file_path, &runtime.actions)?;

    // Only commit the bump after both files are safely on disk, so a failed
    // write leaves the runtime retryable instead of permanently conflicting.
    runtime.document = document;
    runtime.revision = next_revision;
    Ok(())
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
            Some("block-1".to_string()),
        ));
        assert_eq!(runtime.canvas_for_layer("layer-1", 0).unwrap().len(), 1);

        runtime.apply_action_record(SharedDocumentActionRecord::undo(
            "a2", "doc-1", "layer-1", "u2", "2",
        ));
        assert!(runtime.canvas_for_layer("layer-1", 0).unwrap().is_empty());

        runtime.apply_action_record(SharedDocumentActionRecord::redo(
            "a3", "doc-1", "layer-1", "u1", "3",
        ));
        assert_eq!(
            runtime
                .canvas_for_layer("layer-1", 0)
                .unwrap()
                .get(&point(1, 1)),
            Some(&cell('#'))
        );

        let replayed = SharedDocumentRuntime::replay(runtime.document.clone(), runtime.actions.clone());
        assert_eq!(
            replayed.canvas_for_layer("layer-1", 0),
            runtime.canvas_for_layer("layer-1", 0)
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
            revision: 0,
            selection: SharedDocumentSelection::default(),
        };
        let mut runtime = SharedDocumentRuntime::new(document);
        runtime.apply_action_record(SharedDocumentActionRecord::cell_patch_set(
            "a1",
            "doc-1",
            "layer-1",
            "u1",
            "1",
            vec![SharedCellPatch::new(point(0, 0), None, Some(&cell('A')))],
            Some("block-1".to_string()),
        ));
        runtime.apply_action_record(SharedDocumentActionRecord::cell_patch_set(
            "a2",
            "doc-1",
            "layer-2",
            "u1",
            "2",
            vec![SharedCellPatch::new(point(0, 0), None, Some(&cell('B')))],
            Some("block-1".to_string()),
        ));

        runtime.apply_action_record(SharedDocumentActionRecord::undo(
            "a3", "doc-1", "layer-1", "u2", "3",
        ));

        assert!(runtime.canvas_for_layer("layer-1", 0).unwrap().is_empty());
        assert_eq!(
            runtime
                .canvas_for_layer("layer-2", 0)
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
        assert!(runtime.canvas_for_layer("layer-1", 0).unwrap().is_empty());
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
        assert!(runtime.canvas_for_layer("layer-2", 0).unwrap().is_empty());
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
            Some("block-1".to_string()),
        ));
        runtime.apply_action_record(SharedDocumentActionRecord::cell_patch_set(
            "a2",
            "doc-1",
            "layer-2",
            "u1",
            "2",
            vec![SharedCellPatch::new(point(0, 0), None, Some(&cell('B')))],
            Some("block-1".to_string()),
        ));

        let composited = runtime.composited_canvas_in_layer_order(0);

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
            Some("block-1".to_string()),
        ));

        save_shared_document_snapshot(&paths, &mut runtime).unwrap();

        assert!(paths.document_file_path.exists());
        assert!(paths.actions_file_path.exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn snapshot_saves_bump_the_document_revision_monotonically() {
        let unique = format!(
            "thaum-painter-revision-test-{}",
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
        assert_eq!(runtime.revision(), 0);

        save_shared_document_snapshot(&paths, &mut runtime).unwrap();
        save_shared_document_snapshot(&paths, &mut runtime).unwrap();

        assert_eq!(runtime.revision(), 2);
        let on_disk = load_document_file(&paths.document_file_path).unwrap();
        assert_eq!(on_disk.revision, 2);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn snapshot_save_refuses_to_clobber_a_document_changed_on_disk() {
        let unique = format!(
            "thaum-painter-conflict-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        let paths = SharedDocumentPaths::new(root.clone());
        let mut writer = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));
        save_shared_document_snapshot(&paths, &mut writer).unwrap();

        // A second writer loads the same file, saves first, and bumps the disk
        // revision — simulating another user (or app instance) winning the race.
        let mut rival = load_or_create_shared_document(&paths, {
            let mut fallback = SharedDocumentFile::single_layer("doc-1", "Doc", "layer-1", "Layer 1");
            fallback.revision = writer.revision();
            fallback
        })
        .unwrap();
        assert_eq!(rival.revision(), 1);
        save_shared_document_snapshot(&paths, &mut rival).unwrap();

        // The stale writer must be refused, and the on-disk document must still be
        // the rival's — no silent split-brain overwrite.
        let error = save_shared_document_snapshot(&paths, &mut writer)
            .expect_err("stale writer must not clobber a newer on-disk document");
        assert!(error.to_string().contains("changed on disk"));
        let survivor = load_document_file(&paths.document_file_path).unwrap();
        assert_eq!(survivor.revision, 2);

        // Reloading the document re-seeds the revision, so saving works again.
        let mut recovered = load_or_create_shared_document(&paths, {
            let mut fallback = SharedDocumentFile::single_layer("doc-1", "Doc", "layer-1", "Layer 1");
            fallback.revision = survivor.revision;
            fallback
        })
        .unwrap();
        assert_eq!(recovered.revision(), 2);
        save_shared_document_snapshot(&paths, &mut recovered).unwrap();
        assert_eq!(recovered.revision(), 3);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn selection_channels_apply_all_write_modes() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));
        let p = |x: i32, y: i32| CellPoint { x, y, z: 0 };

        // A fresh document carries the one default selection channel, empty.
        assert!(runtime
            .selection_points(DEFAULT_SELECTION_CHANNEL_ID)
            .is_empty());

        runtime.apply_selection_points(
            DEFAULT_SELECTION_CHANNEL_ID,
            [p(1, 1), p(2, 1)],
            SharedSelectionWriteMode::Additive,
        );
        runtime.apply_selection_points(
            DEFAULT_SELECTION_CHANNEL_ID,
            [p(3, 1)],
            SharedSelectionWriteMode::Additive,
        );
        assert_eq!(
            runtime.selection_points(DEFAULT_SELECTION_CHANNEL_ID),
            vec![p(1, 1), p(2, 1), p(3, 1)]
        );
        assert!(runtime.selection_contains(DEFAULT_SELECTION_CHANNEL_ID, p(2, 1)));

        runtime.apply_selection_points(
            DEFAULT_SELECTION_CHANNEL_ID,
            [p(2, 1)],
            SharedSelectionWriteMode::Subtract,
        );
        assert!(!runtime.selection_contains(DEFAULT_SELECTION_CHANNEL_ID, p(2, 1)));

        runtime.apply_selection_points(
            DEFAULT_SELECTION_CHANNEL_ID,
            [p(1, 1), p(2, 1)],
            SharedSelectionWriteMode::Intersect,
        );
        assert_eq!(
            runtime.selection_points(DEFAULT_SELECTION_CHANNEL_ID),
            vec![p(1, 1)]
        );

        runtime.apply_selection_points(
            DEFAULT_SELECTION_CHANNEL_ID,
            [p(4, 4), p(5, 4)],
            SharedSelectionWriteMode::Replace,
        );
        assert_eq!(
            runtime.selection_points(DEFAULT_SELECTION_CHANNEL_ID),
            vec![p(4, 4), p(5, 4)]
        );

        // An unknown channel id is created on demand rather than rejected.
        assert!(runtime.apply_selection_points(
            "private", [p(0, 0)], SharedSelectionWriteMode::Additive
        ));
        assert!(runtime.selection_contains("private", p(0, 0)));
    }

    #[test]
    fn selection_channels_persist_through_snapshot_and_round_trip() {
        let unique = format!(
            "thaum-painter-selection-test-{}",
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
        runtime.apply_selection_points(
            DEFAULT_SELECTION_CHANNEL_ID,
            [
                CellPoint { x: 1, y: 2, z: 3 },
                CellPoint { x: -4, y: 5, z: 0 },
            ],
            SharedSelectionWriteMode::Additive,
        );

        save_shared_document_snapshot(&paths, &mut runtime).unwrap();
        let reloaded = load_or_create_shared_document(&paths, {
            let mut fallback = SharedDocumentFile::single_layer(
                "doc-1", "Doc", "layer-1", "Layer 1",
            );
            fallback.selection = runtime.document.selection.clone();
            fallback
        })
        .unwrap();

        // Points survive the file round trip exactly, including negative coords.
        assert_eq!(
            reloaded.selection_points(DEFAULT_SELECTION_CHANNEL_ID),
            vec![
                CellPoint { x: -4, y: 5, z: 0 },
                CellPoint { x: 1, y: 2, z: 3 },
            ]
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn documents_saved_without_selection_state_load_with_a_default_channel() {
        // Simulate an older document.json without the selection field: serde's
        // default must materialize the one default channel instead of failing.
        let legacy_json = r#"{
            "file_kind": "thaum-painter-shared-document",
            "schema_version": 1,
            "document_id": "doc-1",
            "title": "Doc",
            "layers": []
        }"#;
        let document: SharedDocumentFile = serde_json::from_str(legacy_json).unwrap();

        assert_eq!(document.revision, 0);
        assert_eq!(document.selection.channels.len(), 1);
        assert_eq!(
            document.selection.channels[0].channel_id,
            DEFAULT_SELECTION_CHANNEL_ID
        );
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
            Some("block-1".to_string()),
        ));

        assert!(runtime.set_layer_visible("layer-1", false));
        assert!(runtime.composited_canvas_in_layer_order(0).is_empty());

        assert!(runtime.set_layer_visible("layer-1", true));
        assert_eq!(
            runtime.composited_canvas_in_layer_order(0).get(&point(0, 0)),
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
        assert!(runtime.canvas_for_layer("layer-1", 0).is_none());
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

        assert!(runtime
            .split_property_block("layer-1", "raster", "block-1", 8)
            .is_some());
        let blocks = &runtime.property_track("layer-1", "raster").unwrap().blocks;

        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].id, "block-1");
        assert_eq!(blocks[0].start_breath, 0);
        assert_eq!(blocks[0].length_breaths, 8);
        assert_eq!(blocks[1].start_breath, 8);
        assert_eq!(blocks[1].length_breaths, 16);
    }

    #[test]
    fn pushed_timing_shifts_following_blocks_to_preserve_spacing() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));
        runtime.split_property_block("layer-1", "raster", "block-1", 8);

        // Contiguous channel: block-1 (0..8), block-2 (8..24).

        // Shrinking block-1 to 0..4 pulls block-2 left by the same delta, so the
        // channel stays gap-free and relative spacing is preserved.
        assert!(runtime.set_property_block_timing_pushed("layer-1", "raster", "block-1", 0, 4));
        let blocks = &runtime.property_track("layer-1", "raster").unwrap().blocks;
        assert_eq!((blocks[0].start_breath, blocks[0].length_breaths), (0, 4));
        assert_eq!((blocks[1].start_breath, blocks[1].length_breaths), (4, 16));

        // Growing block-1 back pushes block-2 right by the same delta.
        assert!(runtime.set_property_block_timing_pushed("layer-1", "raster", "block-1", 0, 8));
        let blocks = &runtime.property_track("layer-1", "raster").unwrap().blocks;
        assert_eq!((blocks[1].start_breath, blocks[1].length_breaths), (8, 16));

        // The edited block itself is never clobbered by a neighbor's shift.
        assert_eq!((blocks[0].start_breath, blocks[0].length_breaths), (0, 8));
    }

    #[test]
    fn destructive_timing_split_right_fragment_inherits_the_victim_canvas() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));
        runtime.split_property_block("layer-1", "raster", "block-1", 8);
        runtime.set_property_block_timing("layer-1", "raster", "block-2", 16, 8);
        // block-2 (16..24) carries content; block-1 (0..8) is empty.
        runtime.apply_action_record(SharedDocumentActionRecord::cell_patch_set(
            "a1",
            "doc-1",
            "layer-1",
            "u1",
            "1",
            vec![SharedCellPatch::new(point(0, 0), None, Some(&cell('B')))],
            Some("block-2".to_string()),
        ));

        // block-1 is destructively moved into the MIDDLE of block-2 (16..24):
        // edited span 18..22 straddles block-2's interior, so block-2 splits at 18 —
        // a left fragment keeps 16..18 under the old id, and a fresh right fragment
        // covers 22..24 carrying block-2's content.
        assert!(
            runtime.set_property_block_timing_destructive("layer-1", "raster", "block-1", 18, 4)
        );
        let blocks = &runtime.property_track("layer-1", "raster").unwrap().blocks;
        assert_eq!(blocks.len(), 3);
        let right = blocks
            .iter()
            .find(|block| block.start_breath == 22)
            .expect("split right fragment should exist");
        assert_eq!(right.length_breaths, 2);
        assert_eq!(
            runtime
                .canvas_for_layer("layer-1", 23)
                .unwrap()
                .get(&point(0, 0)),
            Some(&cell('B')),
            "the right fragment must inherit the victim's content, not start empty"
        );
        // The left fragment keeps the victim's canvas too (both halves start identical).
        assert_eq!(
            runtime
                .canvas_for_layer("layer-1", 17)
                .unwrap()
                .get(&point(0, 0)),
            Some(&cell('B'))
        );
    }

    #[test]
    fn destructive_timing_truncates_removes_and_splits_covered_neighbors() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));
        runtime.split_property_block("layer-1", "raster", "block-1", 8);
        runtime.set_property_block_timing("layer-1", "raster", "block-2", 16, 8);

        // block-1 (0..8) grows destructively to 0..20: block-2 (16..24) truncates to
        // start at the edited block's end.
        assert!(
            runtime.set_property_block_timing_destructive("layer-1", "raster", "block-1", 0, 20)
        );
        let blocks = &runtime.property_track("layer-1", "raster").unwrap().blocks;
        assert_eq!((blocks[0].start_breath, blocks[0].length_breaths), (0, 20));
        assert_eq!((blocks[1].start_breath, blocks[1].length_breaths), (20, 4));

        // A fully covered neighbor disappears entirely.
        runtime.set_property_block_timing("layer-1", "raster", "block-2", 24, 8);
        assert!(
            runtime.set_property_block_timing_destructive("layer-1", "raster", "block-1", 0, 40)
        );
        let blocks = &runtime.property_track("layer-1", "raster").unwrap().blocks;
        assert_eq!(blocks.len(), 1);
        assert_eq!((blocks[0].start_breath, blocks[0].length_breaths), (0, 40));
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
    fn set_property_block_timing_cannot_cross_a_neighbor_block() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));
        runtime.split_property_block("layer-1", "raster", "block-1", 8);
        runtime.set_property_block_timing("layer-1", "raster", "block-2", 16, 8);

        // Dragging block-2's start to 4 would land it on block-1; it clamps to block-1's end.
        assert!(runtime.set_property_block_timing("layer-1", "raster", "block-2", 4, 8));
        let blocks = &runtime.property_track("layer-1", "raster").unwrap().blocks;
        assert_eq!(blocks[1].start_breath, 8);

        // And a block cannot grow across the other one either.
        assert!(runtime.set_property_block_timing("layer-1", "raster", "block-1", 0, 100));
        let blocks = &runtime.property_track("layer-1", "raster").unwrap().blocks;
        assert_eq!(blocks[0].start_breath, 0);
        assert_eq!(blocks[0].length_breaths, 8);

        // No two blocks in the channel share a breath.
        let first = (blocks[0].start_breath, blocks[0].length_breaths);
        let second = (blocks[1].start_breath, blocks[1].length_breaths);
        assert!(!breath_in_span(
            second.0,
            first.0,
            first.1
        ));
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

    #[test]
    fn canvas_for_layer_resolves_the_block_covering_the_breath() {
        let document = SharedDocumentFile::single_layer("doc-1", "Doc", "layer-1", "Layer 1");
        let mut runtime = SharedDocumentRuntime::new(document);
        runtime.split_property_block("layer-1", "raster", "block-1", 8);
        runtime.apply_action_record(SharedDocumentActionRecord::cell_patch_set(
            "a1",
            "doc-1",
            "layer-1",
            "u1",
            "1",
            vec![SharedCellPatch::new(point(0, 0), None, Some(&cell('A')))],
            Some("block-1".to_string()),
        ));

        assert_eq!(
            runtime.canvas_for_layer("layer-1", 5).unwrap().get(&point(0, 0)),
            Some(&cell('A'))
        );
        // block-2 has no content yet, so later breaths render empty.
        assert!(runtime.canvas_for_layer("layer-1", 10).unwrap().is_empty());
        // A breath in a gap between blocks has no canvas at all.
        assert!(runtime.canvas_for_layer("layer-1", 99).is_none());

        let replayed = SharedDocumentRuntime::replay(runtime.document.clone(), runtime.actions.clone());
        assert_eq!(
            replayed.canvas_for_layer("layer-1", 5),
            runtime.canvas_for_layer("layer-1", 5)
        );
        assert_eq!(
            replayed.canvas_for_layer("layer-1", 10),
            runtime.canvas_for_layer("layer-1", 10)
        );
    }

    #[test]
    fn painting_into_a_blank_block_unblanks_it_and_lands_on_its_canvas() {
        let document = SharedDocumentFile::single_layer("doc-1", "Doc", "layer-1", "Layer 1");
        let mut runtime = SharedDocumentRuntime::new(document);
        runtime.split_property_block("layer-1", "raster", "block-1", 8);
        runtime.apply_action_record(SharedDocumentActionRecord::cell_patch_set(
            "a1",
            "doc-1",
            "layer-1",
            "u1",
            "1",
            vec![SharedCellPatch::new(point(0, 0), None, Some(&cell('A')))],
            Some("block-1".to_string()),
        ));
        assert!(runtime.blank_property_block("layer-1", "raster", "block-2"));
        assert!(runtime.canvas_for_layer("layer-1", 10).unwrap().is_empty());

        runtime.apply_action_record(SharedDocumentActionRecord::cell_patch_set(
            "a2",
            "doc-1",
            "layer-1",
            "u1",
            "2",
            vec![SharedCellPatch::new(point(0, 0), None, Some(&cell('B')))],
            Some("block-2".to_string()),
        ));

        let blocks = &runtime.property_track("layer-1", "raster").unwrap().blocks;
        assert!(!blocks.iter().find(|block| block.id == "block-2").unwrap().is_blank);
        assert_eq!(
            runtime.canvas_for_layer("layer-1", 10).unwrap().get(&point(0, 0)),
            Some(&cell('B'))
        );
        // The first block's content is untouched.
        assert_eq!(
            runtime.canvas_for_layer("layer-1", 0).unwrap().get(&point(0, 0)),
            Some(&cell('A'))
        );
    }

    #[test]
    fn legacy_records_without_a_block_id_replay_into_the_first_non_blank_block() {
        let document = SharedDocumentFile::single_layer("doc-1", "Doc", "layer-1", "Layer 1");
        let mut runtime = SharedDocumentRuntime::new(document);
        runtime.split_property_block("layer-1", "raster", "block-1", 8);
        runtime.blank_property_block("layer-1", "raster", "block-1");

        runtime.apply_action_record(SharedDocumentActionRecord::cell_patch_set(
            "a1",
            "doc-1",
            "layer-1",
            "u1",
            "1",
            vec![SharedCellPatch::new(point(0, 0), None, Some(&cell('A')))],
            None,
        ));

        assert_eq!(
            runtime.canvas_for_layer("layer-1", 10).unwrap().get(&point(0, 0)),
            Some(&cell('A'))
        );
        assert!(runtime.canvas_for_layer("layer-1", 0).unwrap().is_empty());

        let replayed = SharedDocumentRuntime::replay(runtime.document.clone(), runtime.actions.clone());
        assert_eq!(
            replayed.canvas_for_layer("layer-1", 10),
            runtime.canvas_for_layer("layer-1", 10)
        );
    }

    #[test]
    fn swap_property_blocks_moves_the_content_with_the_block() {
        let document = SharedDocumentFile::single_layer("doc-1", "Doc", "layer-1", "Layer 1");
        let mut runtime = SharedDocumentRuntime::new(document);
        runtime.split_property_block("layer-1", "raster", "block-1", 8);
        runtime.apply_action_record(SharedDocumentActionRecord::cell_patch_set(
            "a1",
            "doc-1",
            "layer-1",
            "u1",
            "1",
            vec![SharedCellPatch::new(point(0, 0), None, Some(&cell('A')))],
            Some("block-1".to_string()),
        ));

        assert!(runtime.swap_property_blocks("layer-1", "raster", "block-1", "block-2"));

        // block-1 now covers breaths 8..23 and keeps its own canvas, so its content
        // visibly moves to the exchanged span; block-2 covers breaths 0..7 and shows
        // its own (empty) content there.
        assert_eq!(
            runtime.canvas_for_layer("layer-1", 10).unwrap().get(&point(0, 0)),
            Some(&cell('A'))
        );
        assert!(runtime.canvas_for_layer("layer-1", 0).unwrap().is_empty());
    }

    #[test]
    fn split_data_propagation_record_copies_the_source_canvas_onto_the_new_half() {
        let document = SharedDocumentFile::single_layer("doc-1", "Doc", "layer-1", "Layer 1");
        let mut runtime = SharedDocumentRuntime::new(document);
        runtime.apply_action_record(SharedDocumentActionRecord::cell_patch_set(
            "a1",
            "doc-1",
            "layer-1",
            "u1",
            "1",
            vec![SharedCellPatch::new(point(0, 0), None, Some(&cell('A')))],
            Some("block-1".to_string()),
        ));
        let new_block_id = runtime
            .split_property_block("layer-1", "raster", "block-1", 8)
            .unwrap();

        let record = runtime
            .split_data_propagation_record(
                "layer-1",
                "block-1",
                &new_block_id,
                "copy-1".to_string(),
                "u1",
                "t".to_string(),
            )
            .expect("raster split should carry data");
        runtime.apply_action_record(record);

        // Both halves start as identical copies at different breaths.
        assert_eq!(
            runtime.canvas_for_layer("layer-1", 0),
            runtime.canvas_for_layer("layer-1", 10)
        );
        assert_eq!(
            runtime
                .canvas_for_layer("layer-1", 10)
                .unwrap()
                .get(&point(0, 0)),
            Some(&cell('A'))
        );

        // Replay from the saved document reproduces the same copy.
        let replayed =
            SharedDocumentRuntime::replay(runtime.document.clone(), runtime.actions.clone());
        assert_eq!(
            replayed.canvas_for_layer("layer-1", 10),
            runtime.canvas_for_layer("layer-1", 10)
        );
    }

    #[test]
    fn split_data_propagation_record_is_none_for_an_empty_channel() {
        let document = SharedDocumentFile::single_layer("doc-1", "Doc", "layer-1", "Layer 1");
        let mut runtime = SharedDocumentRuntime::new(document);
        let new_block_id = runtime
            .split_property_block("layer-1", "raster", "block-1", 8)
            .unwrap();

        assert!(runtime
            .split_data_propagation_record(
                "layer-1",
                "block-1",
                &new_block_id,
                "copy-1".to_string(),
                "u1",
                "t".to_string(),
            )
            .is_none());
    }
}
