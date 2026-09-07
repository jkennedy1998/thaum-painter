use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thaum_renderer_domain::{CellGraphic, CellMaterialId, CellPoint, SpriteGraphic, WorldPoint};

use crate::properties::{breath_in_span, destructive_breath_span, pushed_breath_span};
use crate::{Canvas, PaintColor, PaintedCell};

pub const SHARED_DOCUMENT_KIND: &str = "thaum-painter-shared-document";
pub const SHARED_DOCUMENT_SCHEMA_VERSION: u32 = 2;

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
    /// Per-kind payload for value-carrying property kinds. Move blocks hold
    /// an `{x,y,z}` render offset; timing-only kinds (raster) leave it `None`.
    /// `serde(default)` keeps existing files loading unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
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

/// Pushes one re-tiled block, merging it into the previous block when both are blank.
/// Adjacent empties merge into one — the no-adjacent-empties invariant (J, 2026-09-07):
/// the user only ever sees empties and bars, never two empties touching. Only blanks
/// merge; two solid bars stay distinct keyframes even when adjacent.
fn push_tiled_block(
    tiled: &mut Vec<SharedDocumentPropertyBlock>,
    block: SharedDocumentPropertyBlock,
) {
    if let Some(last) = tiled.last_mut() {
        if last.is_blank && block.is_blank {
            last.length_breaths += block.length_breaths;
            return;
        }
    }
    tiled.push(block);
}

/// Re-tiles one property track's blocks over the layer's breath span so the track
/// stays fully covered: blocks are clipped into the span (fully-outside ones dropped),
/// every uncovered window becomes a blank block, and adjacent blanks merge into one.
/// Binary channels — every breath is empty or solid, never a void, never two touching
/// empties.
fn retiled_property_track(
    blocks: Vec<SharedDocumentPropertyBlock>,
    span_start: u32,
    span_length: u32,
) -> Vec<SharedDocumentPropertyBlock> {
    let span_end = span_start.saturating_add(span_length.max(1));
    let mut clipped: Vec<SharedDocumentPropertyBlock> = Vec::new();
    for mut block in blocks {
        let block_end = block.start_breath + block.length_breaths.max(1);
        let new_start = block.start_breath.max(span_start);
        let new_end = block_end.min(span_end);
        if new_end <= new_start {
            // Fully outside the span — nothing left to cover.
            continue;
        }
        block.start_breath = new_start;
        block.length_breaths = new_end - new_start;
        clipped.push(block);
    }
    clipped.sort_by_key(|block| block.start_breath);
    let mut tiled: Vec<SharedDocumentPropertyBlock> = Vec::with_capacity(clipped.len() + 2);
    let mut cursor = span_start;
    for mut block in clipped {
        let block_end = block.start_breath + block.length_breaths;
        if block_end <= cursor {
            // Fully covered by an earlier block (only possible in broken data); drop it.
            continue;
        }
        if block.start_breath > cursor {
            let gap_id = next_property_block_id(&tiled);
            push_tiled_block(
                &mut tiled,
                SharedDocumentPropertyBlock {
                    id: gap_id,
                    start_breath: cursor,
                    length_breaths: block.start_breath - cursor,
                    is_blank: true,
                    value: None,
                },
            );
        } else {
            // Overlapping broken data: trim to the first uncovered breath.
            block.start_breath = cursor;
            block.length_breaths = block_end - cursor;
        }
        cursor = block.start_breath + block.length_breaths;
        push_tiled_block(&mut tiled, block);
    }
    if cursor < span_end {
        let tail_id = next_property_block_id(&tiled);
        push_tiled_block(
            &mut tiled,
            SharedDocumentPropertyBlock {
                id: tail_id,
                start_breath: cursor,
                length_breaths: span_end - cursor,
                is_blank: true,
                value: None,
            },
        );
    }
    tiled
}

fn default_raster_property_track(
    start_breath: u32,
    length_breaths: u32,
) -> SharedDocumentPropertyTrack {
    SharedDocumentPropertyTrack {
        property_id: "raster".to_string(),
        blocks: vec![SharedDocumentPropertyBlock {
            id: "block-1".to_string(),
            start_breath,
            length_breaths: length_breaths.max(1),
            is_blank: false,
            value: None,
        }],
    }
}

/// Parses a move block's `{x,y,z}` offset value — the same shape the old
/// system's move properties carried and the file-schema import path parses.
fn parse_move_offset(value: &Value) -> Option<[i32; 3]> {
    let object = value.as_object()?;
    let axis = |key: &str| object.get(key).and_then(Value::as_i64).map(|v| v as i32);
    Some([axis("x")?, axis("y")?, axis("z")?])
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

/// The document's active timeline span: the loop window the playhead and
/// (future) playback live inside. One bar on the layers-panel timeline edits
/// it; it is document state, not per-layer timing, so it cannot be split or
/// deleted the way property blocks can.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentWindow {
    pub start_breath: u32,
    pub end_breath: u32,
}

impl Default for DocumentWindow {
    fn default() -> Self {
        // Defaults to the panel's visible timeline span (24 breaths) so a new
        // document's window bar covers exactly the ruler it is drawn on.
        Self {
            start_breath: 0,
            end_breath: 23,
        }
    }
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
    /// The document's active timeline span, editable through the layers-panel
    /// loop bar. `serde(default)` keeps older files (no window field) loading
    /// at the 24-breath default span.
    #[serde(default)]
    pub document_window: DocumentWindow,
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
                property_tracks: vec![default_raster_property_track(
                    0,
                    default_layer_length_breaths(),
                )],
            }],
            selection: SharedDocumentSelection::default(),
            document_window: DocumentWindow::default(),
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

    /// Swaps the before/after values so applying this patch undoes the original.
    pub fn inverse(&self) -> Self {
        Self {
            position: self.position.clone(),
            before: self.after.clone(),
            after: self.before.clone(),
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
        /// Passive records paint content but are invisible to the undo/redo stacks:
        /// history-squash baselines and undo/redo revert records. Keeping the log
        /// all-forward-edits makes squash trivially safe (replay never references a
        /// folded action) and matches the undo-as-operation multiplayer model.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        passive: bool,
        /// For passive revert records: the action id this record undoes (undo) or
        /// re-applies (redo). Replay uses it to move the action between the applied
        /// and undone stacks so reload preserves undo depth.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reverts: Option<String>,
    },
    /// Legacy undo/redo records from before undo became a revert record. Kept
    /// replayable so existing action logs keep loading; new code writes only
    /// `CellPatchSet` records.
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
            action: SharedDocumentAction::CellPatchSet {
                patches,
                block_id,
                passive: false,
                reverts: None,
            },
        }
    }

    /// Builds a passive undo/redo revert record whose replay moves `reverts`
    /// between the applied and undone stacks (pop for undo, push-back for redo).
    #[allow(clippy::too_many_arguments)] // record shape mirrors the on-disk kebab-case JSON; fields stay flat
    pub fn revert_patch_set(
        action_id: impl Into<String>,
        document_id: impl Into<String>,
        layer_id: impl Into<String>,
        user_id: impl Into<String>,
        created_at: impl Into<String>,
        patches: Vec<SharedCellPatch>,
        block_id: Option<String>,
        reverts: impl Into<String>,
    ) -> Self {
        Self {
            action_id: action_id.into(),
            document_id: document_id.into(),
            layer_id: layer_id.into(),
            user_id: user_id.into(),
            created_at: created_at.into(),
            action: SharedDocumentAction::CellPatchSet {
                patches,
                block_id,
                passive: true,
                reverts: Some(reverts.into()),
            },
        }
    }

    /// Marks this record passive: it paints content on replay but never enters the
    /// undo/redo stacks. Used for undo/redo revert records and squash baselines.
    pub fn as_passive(mut self) -> Self {
        if let SharedDocumentAction::CellPatchSet { passive, .. } = &mut self.action {
            *passive = true;
        }
        self
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

/// The inverse (undo) or re-application (redo) patches for one history action,
/// resolved to the raster block the action landed on. The caller persists these
/// as a passive `CellPatchSet` revert record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryRevertPatches {
    pub block_id: String,
    pub patches: Vec<SharedCellPatch>,
    /// The history action this revert undoes (undo) or re-applies (redo).
    pub action_id: String,
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
    /// Passive baseline records covering every action folded out of the saved log.
    baseline: Vec<SharedDocumentActionRecord>,
    /// How many entries of `actions` (the oldest ones) the baseline already covers.
    squashed_through: usize,
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
                    block_canvases
                        .insert((layer.layer_id.clone(), block.id.clone()), Canvas::new());
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
            baseline: Vec::new(),
            squashed_through: 0,
        }
    }

    /// The revision this runtime loaded and last saved. Compare against
    /// `load_document_file(..).revision` to detect another writer's changes.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Direct read access to one raster block's live canvas (test and debug use).
    pub fn block_canvas(&self, layer_id: &str, block_id: &str) -> Option<&Canvas> {
        self.block_canvases
            .get(&(layer_id.to_string(), block_id.to_string()))
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
            .map(|channel| {
                channel
                    .points
                    .iter()
                    .map(PersistedCellPoint::to_runtime)
                    .collect()
            })
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
        let incoming: BTreeSet<PersistedCellPoint> =
            points.into_iter().map(PersistedCellPoint::from).collect();
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
                channel.points = channel.points.intersection(&incoming).cloned().collect();
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

    /// The first breath the layer's raster track covers, preferring the earliest
    /// block with content. Boot seeks the playhead here so it never rests in a gap
    /// where the layer renders nothing and strokes are silently rejected.
    pub fn first_breath_with_raster_block(&self, layer_id: &str) -> Option<u32> {
        let layer = self
            .document
            .layers
            .iter()
            .find(|layer| layer.layer_id == layer_id)?;
        let track = layer
            .property_tracks
            .iter()
            .find(|track| track.property_id == "raster")?;
        let first = track.blocks.iter().min_by_key(|block| block.start_breath)?;
        let first_with_content = track
            .blocks
            .iter()
            .filter(|block| !block.is_blank)
            .min_by_key(|block| block.start_breath);
        Some(first_with_content.unwrap_or(first).start_breath)
    }

    pub fn layers(&self) -> &[SharedDocumentLayer] {
        &self.document.layers
    }

    /// The active move property block's `{x,y,z}` render offset at `breath`,
    /// or the zero offset when the layer has no move track, no block covers
    /// the breath, or the value is missing/malformed. Move offsets are
    /// optional metadata: absence renders the layer unshifted.
    pub fn move_offset_for_layer(&self, layer_id: &str, breath: u32) -> WorldPoint {
        let Some(track) = self.property_track(layer_id, "move") else {
            return WorldPoint::origin();
        };
        let Some(block) = track
            .blocks
            .iter()
            .find(|block| breath_in_span(breath, block.start_breath, block.length_breaths))
        else {
            return WorldPoint::origin();
        };
        block
            .value
            .as_ref()
            .and_then(parse_move_offset)
            .map(|offset| WorldPoint {
                x: offset[0],
                y: offset[1],
                z: offset[2],
            })
            .unwrap_or_else(WorldPoint::origin)
    }

    /// Adds `delta` to the move block covering `breath` on `layer_id`,
    /// creating the move track and a block spanning the layer's own timing
    /// window when either is missing. Returns whether the document changed.
    pub fn add_move_offset(&mut self, layer_id: &str, breath: u32, delta: WorldPoint) -> bool {
        if delta == WorldPoint::origin() {
            return false;
        }
        let (start_breath, length_breaths) = {
            let Some(layer) = self
                .document
                .layers
                .iter()
                .find(|layer| layer.layer_id == layer_id)
            else {
                return false;
            };
            (layer.start_breath, layer.length_breaths)
        };
        let Some(track) = self.ensure_property_track_mut(layer_id, "move") else {
            return false;
        };
        let block = if let Some(position) = track
            .blocks
            .iter()
            .position(|block| breath_in_span(breath, block.start_breath, block.length_breaths))
        {
            &mut track.blocks[position]
        } else {
            track.blocks.push(SharedDocumentPropertyBlock {
                id: next_property_block_id(&track.blocks),
                start_breath,
                length_breaths: length_breaths.max(1),
                is_blank: false,
                value: None,
            });
            track.blocks.last_mut().expect("just pushed")
        };
        let current = block
            .value
            .as_ref()
            .and_then(parse_move_offset)
            .unwrap_or([0, 0, 0]);
        block.value = Some(
            serde_json::json!({
                "x": current[0] + delta.x,
                "y": current[1] + delta.y,
                "z": current[2] + delta.z,
            }),
        );
        block.is_blank = false;
        true
    }

    pub fn property_track(
        &self,
        layer_id: &str,
        property_id: &str,
    ) -> Option<&SharedDocumentPropertyTrack> {
        self.document
            .layers
            .iter()
            .find(|layer| layer.layer_id == layer_id)?
            .property_tracks
            .iter()
            .find(|track| track.property_id == property_id)
    }

    /// One-line shape of a property track for the interaction log artifact:
    /// each block as `start..end` with `/e` marking an empty — `0..5 5..10/e`.
    pub fn property_track_shape(&self, layer_id: &str, property_id: &str) -> String {
        let Some(track) = self.property_track(layer_id, property_id) else {
            return "[]".to_string();
        };
        let spans: Vec<String> = track
            .blocks
            .iter()
            .map(|block| {
                format!(
                    "{}..{}{}",
                    block.start_breath,
                    block.start_breath + block.length_breaths,
                    if block.is_blank { "/e" } else { "" }
                )
            })
            .collect();
        format!("[{}]", spans.join(" "))
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
            property_tracks: vec![default_raster_property_track(
                0,
                default_layer_length_breaths(),
            )],
        });
        self.block_canvases
            .insert((layer_id.clone(), "block-1".to_string()), Canvas::new());
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
        self.document
            .layers
            .retain(|layer| layer.layer_id != layer_id);
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
    /// The document's active timeline span (loop window).
    pub fn document_window(&self) -> DocumentWindow {
        self.document.document_window
    }

    /// Reshapes the document's active timeline span. The end never drops below
    /// the start, so the window stays a valid span (possibly one breath wide).
    pub fn set_document_window(&mut self, start_breath: u32, end_breath: u32) {
        self.document.document_window = DocumentWindow {
            start_breath,
            end_breath: end_breath.max(start_breath),
        };
    }

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
    pub fn set_layer_timing(
        &mut self,
        layer_id: &str,
        start_breath: u32,
        length_breaths: u32,
    ) -> bool {
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
        // Binary tiling: the layer's span changed, so every property track re-tiles
        // over the new span — uncovered windows become blank blocks.
        let (span_start, span_length) = (layer.start_breath, layer.length_breaths);
        for track in &mut layer.property_tracks {
            track.blocks = retiled_property_track(
                std::mem::take(&mut track.blocks),
                span_start,
                span_length,
            );
        }
        true
    }

    /// Reshapes one property block with a time-preserving ripple (`pushed_breath_span`):
    /// the blocks on the dragged side of the channel shift by the same delta, so
    /// relative spacing is preserved. The seam then re-tiles the track: a ripple
    /// shrink with nothing left to pull cannot fill the tail (bare void), and a
    /// shift clamped at breath zero can strand an overlap — re-tiling turns any
    /// uncovered window into a blank block and trims overlapped data, so the
    /// track always leaves this seam fully tiled.
    pub fn set_property_block_timing_pushed(
        &mut self,
        layer_id: &str,
        property_id: &str,
        block_id: &str,
        start_breath: u32,
        length_breaths: u32,
    ) -> bool {
        let (span_start, span_length) = {
            let Some(layer) = self
                .document
                .layers
                .iter()
                .find(|layer| layer.layer_id == layer_id)
            else {
                return false;
            };
            (layer.start_breath, layer.length_breaths)
        };
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
        let pushed = pushed_breath_span(
            &other_spans,
            original,
            (start_breath, length_breaths.max(1)),
        );
        track.blocks[index].start_breath = pushed.edited.0;
        track.blocks[index].length_breaths = pushed.edited.1;
        for (other_index, (start, length)) in pushed.shifted {
            let track_index = other_track_indices[other_index];
            track.blocks[track_index].start_breath = start;
            track.blocks[track_index].length_breaths = length;
        }
        // Binary tiling: the ripple alone cannot guarantee coverage — see the doc
        // comment above — so this seam re-tiles like every other timing seam.
        track.blocks = retiled_property_track(
            std::mem::take(&mut track.blocks),
            span_start,
            span_length,
        );
        true
    }

    /// Reshapes one property block destructively (`destructive_breath_span`): the block
    /// takes its full requested span and covered neighbors yield into empty cell types —
    /// partially overlapped neighbors shrink to their remainder and turn blank (content
    /// discarded), fully covered ones are removed outright. The edited block's vacated
    /// range and every other uncovered window re-tile into blank blocks, so the track
    /// never leaves a void. A shrink overlaps nothing, so it acts as a plain trim.
    pub fn set_property_block_timing_destructive(
        &mut self,
        layer_id: &str,
        property_id: &str,
        block_id: &str,
        start_breath: u32,
        length_breaths: u32,
    ) -> bool {
        let Some(layer) = self
            .document
            .layers
            .iter()
            .find(|layer| layer.layer_id == layer_id)
        else {
            return false;
        };
        let (span_start, span_length) = (layer.start_breath, layer.length_breaths);
        // Plan pass over an immutable track: compute the destructive resolution. The
        // borrow must end before the mutable apply pass below.
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
            Some((
                destructive_breath_span(&other_spans, (start_breath, length_breaths.max(1))),
                other_track_indices,
            ))
        };
        let Some((destructive, other_track_indices)) = plan else {
            return false;
        };
        let Some((track, index)) = self.property_track_block_mut(layer_id, property_id, block_id)
        else {
            return false;
        };
        track.blocks[index].start_breath = destructive.edited.0;
        track.blocks[index].length_breaths = destructive.edited.1;
        // Victims yield into empty cell types; their canvases are discarded with them.
        let mut dropped_canvas_keys = Vec::new();
        for (other_index, (start, length)) in destructive.blanked {
            let track_index = other_track_indices[other_index];
            let victim = &mut track.blocks[track_index];
            victim.start_breath = start;
            victim.length_breaths = length;
            victim.is_blank = true;
            victim.value = None;
            dropped_canvas_keys.push((layer_id.to_string(), victim.id.clone()));
        }
        // Removals shift indices, so apply them highest-index-first.
        let mut removed_track_indices: Vec<usize> = destructive
            .removed
            .iter()
            .map(|slice_index| other_track_indices[*slice_index])
            .collect();
        removed_track_indices.sort_by_key(|&track_index| std::cmp::Reverse(track_index));
        for track_index in removed_track_indices {
            let removed_id = track.blocks[track_index].id.clone();
            track.blocks.remove(track_index);
            dropped_canvas_keys.push((layer_id.to_string(), removed_id));
        }
        for key in dropped_canvas_keys {
            self.block_canvases.remove(&key);
        }
        // Binary tiling: the edited block's vacated range becomes blank and the track
        // stays fully covered — no void ever appears.
        if let Some(track) = self
            .document
            .layers
            .iter_mut()
            .find(|layer| layer.layer_id == layer_id)
            .and_then(|layer| {
                layer
                    .property_tracks
                    .iter_mut()
                    .find(|track| track.property_id == property_id)
            })
        {
            track.blocks = retiled_property_track(
                std::mem::take(&mut track.blocks),
                span_start,
                span_length,
            );
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
        let track = self.ensure_property_track_mut(layer_id, property_id)?;
        let index = track.blocks.iter().position(|block| block.id == block_id)?;
        let block = track.blocks[index].clone();
        if block.length_breaths <= 1 {
            return None;
        }
        // Splitting an empty is meaningless — the halves would be adjacent empties,
        // which the no-adjacent-empties invariant forbids. Blanks have no split
        // branch in the interaction matrix.
        if block.is_blank {
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
                value: block.value.clone(),
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
    /// block renders empty. The track re-tiles afterwards so the new empty merges with any
    /// adjacent empty (no-adjacent-empties invariant). Returns `false` if no such block exists.
    pub fn blank_property_block(
        &mut self,
        layer_id: &str,
        property_id: &str,
        block_id: &str,
    ) -> bool {
        let (span_start, span_length) = {
            let Some(layer) = self
                .document
                .layers
                .iter()
                .find(|layer| layer.layer_id == layer_id)
            else {
                return false;
            };
            (layer.start_breath, layer.length_breaths)
        };
        let Some(track) = self.ensure_property_track_mut(layer_id, property_id) else {
            return false;
        };
        let Some(block) = track.blocks.iter_mut().find(|block| block.id == block_id) else {
            return false;
        };
        block.is_blank = true;
        track.blocks = retiled_property_track(
            std::mem::take(&mut track.blocks),
            span_start,
            span_length,
        );
        self.block_canvases
            .insert((layer_id.to_string(), block_id.to_string()), Canvas::new());
        true
    }

    /// Merges one empty (blank) block into the adjacent content block, preferring the
    /// left side and falling back to the right (J 2026-09-07: one seam for single and
    /// center empties; right-head empties pass `prefer_left: false` for the mirror).
    /// The content block expands destructively over the empty's span and keeps its
    /// content; the empty disappears. Returns `false` — rejecting the interaction —
    /// when the block is not blank or the track has no content at all (fully-empty
    /// track: nothing to merge into).
    pub fn merge_empty_property_block(
        &mut self,
        layer_id: &str,
        property_id: &str,
        block_id: &str,
        prefer_left: bool,
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
        // The no-adjacent-empties invariant guarantees the empty's neighbors are
        // content blocks or the span edge — never another empty.
        let left = if index > 0 { Some(index - 1) } else { None };
        let right = if index + 1 < track.blocks.len() {
            Some(index + 1)
        } else {
            None
        };
        let target = if prefer_left {
            left.filter(|&i| !track.blocks[i].is_blank)
                .or(right.filter(|&i| !track.blocks[i].is_blank))
        } else {
            right.filter(|&i| !track.blocks[i].is_blank)
                .or(left.filter(|&i| !track.blocks[i].is_blank))
        };
        let Some(target_index) = target else {
            return false; // fully-empty track — no content anywhere to merge into
        };
        let empty_span = (
            track.blocks[index].start_breath,
            track.blocks[index].length_breaths,
        );
        if target_index < index {
            // Content on the left expands right over the empty.
            track.blocks[target_index].length_breaths += empty_span.1;
        } else {
            // Content on the right expands left over the empty.
            track.blocks[target_index].start_breath = empty_span.0;
            track.blocks[target_index].length_breaths += empty_span.1;
        }
        track.blocks.remove(index);
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
        let Some(source_index) = track
            .blocks
            .iter()
            .position(|block| block.id == source_block_id)
        else {
            return false;
        };
        let Some(target_index) = track
            .blocks
            .iter()
            .position(|block| block.id == target_block_id)
        else {
            return false;
        };
        let source_span = (
            track.blocks[source_index].start_breath,
            track.blocks[source_index].length_breaths,
        );
        let target_span = (
            track.blocks[target_index].start_breath,
            track.blocks[target_index].length_breaths,
        );
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

    /// Duplicates one block's full span and content immediately to its right, pushing
    /// every later block right by the duplicated length (non-destructive, duplicates
    /// always land on the right — J 2026-09-07). If the duplicate would pass the layer
    /// span end, the layer span grows by exactly the overflow — bounded growth, one
    /// interaction at a time, never auto-filling. The new block starts with an empty
    /// canvas; the caller copies the source's content onto it through
    /// `duplicate_data_propagation_record` so live edits and replay build the same
    /// copy. Returns the new block's id.
    pub fn duplicate_property_block(
        &mut self,
        layer_id: &str,
        property_id: &str,
        block_id: &str,
    ) -> Option<String> {
        let (span_start, span_length) = {
            let layer = self
                .document
                .layers
                .iter()
                .find(|layer| layer.layer_id == layer_id)?;
            (layer.start_breath, layer.length_breaths)
        };
        let source = {
            let track = self.property_track(layer_id, property_id)?;
            let index = track
                .blocks
                .iter()
                .position(|block| block.id == block_id)?;
            track.blocks[index].clone()
        };
        let new_start = source.start_breath + source.length_breaths;
        let span_end = span_start + span_length.max(1);
        let overflow = (new_start + source.length_breaths).saturating_sub(span_end);
        if overflow > 0 {
            // The duplicate needs room past the span end: grow the layer by exactly
            // the overflow. `set_layer_timing` re-tiles every track of the layer, so
            // the new tail is blank and coverage stays exact.
            self.set_layer_timing(layer_id, span_start, span_length + overflow);
        }
        let Some(track) = self.ensure_property_track_mut(layer_id, property_id) else {
            return None;
        };
        let source_index = track
            .blocks
            .iter()
            .position(|block| block.id == block_id)?;
        // Non-destructive push: every block starting at/after the source's end shifts
        // right by the duplicated length. In a tiled track nothing else overlaps the
        // source's span, so this is the complete ripple.
        for block in track.blocks.iter_mut() {
            if block.start_breath >= source.start_breath + source.length_breaths {
                block.start_breath += source.length_breaths;
            }
        }
        let new_id = next_property_block_id(&track.blocks);
        track.blocks.insert(
            source_index + 1,
            SharedDocumentPropertyBlock {
                id: new_id.clone(),
                start_breath: new_start,
                length_breaths: source.length_breaths,
                is_blank: source.is_blank,
                value: source.value.clone(),
            },
        );
        self.block_canvases
            .insert((layer_id.to_string(), new_id.clone()), Canvas::new());
        // When the layer grew, `set_layer_timing` re-tiled this track and left a tail
        // blank inside the new span — and the push above then shifted that tail blank
        // past the span end. One more re-tile drops the out-of-span tail and restores
        // exact coverage of the grown span.
        if overflow > 0 {
            if let Some(track) = self
                .document
                .layers
                .iter_mut()
                .find(|layer| layer.layer_id == layer_id)
                .and_then(|layer| {
                    layer
                        .property_tracks
                        .iter_mut()
                        .find(|track| track.property_id == property_id)
                })
            {
                track.blocks = retiled_property_track(
                    std::mem::take(&mut track.blocks),
                    span_start,
                    span_length + overflow,
                );
            }
        }
        Some(new_id)
    }

    /// Builds the full-cell patch record that propagates a duplicated block's channel
    /// data onto its copy — the duplicate starts as an identical copy at a later span
    /// and diverges as it is edited separately. Returns `None` when the source carries
    /// no data (empty canvas), leaving the copy empty.
    pub fn duplicate_data_propagation_record(
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
            SharedDocumentAction::CellPatchSet {
                patches,
                block_id,
                passive,
                reverts,
            } => {
                if !patches.is_empty() {
                    let target = match block_id {
                        Some(block_id) => Some((layer_id.clone(), block_id.clone())),
                        None => self.legacy_raster_block_target(&layer_id),
                    };
                    if let Some((target_layer_id, target_block_id)) = target {
                        self.unblank_raster_block(&target_layer_id, &target_block_id);
                        let canvas = self
                            .block_canvases
                            .entry((target_layer_id.clone(), target_block_id.clone()))
                            .or_default();
                        apply_patches(canvas, patches, PatchDirection::After);
                        if !*passive {
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
                        } else if let Some(reverted_id) = reverts {
                            // Revert record on replay: undo reverts pop the action out
                            // of the applied stack; redo reverts push it back in.
                            let applied_stack = self
                                .applied_action_ids_by_layer
                                .entry(layer_id.clone())
                                .or_default();
                            if let Some(position) =
                                applied_stack.iter().position(|id| id == reverted_id)
                            {
                                applied_stack.remove(position);
                                self.undone_action_ids_by_layer
                                    .entry(layer_id)
                                    .or_default()
                                    .push(reverted_id.clone());
                            } else {
                                let undone_stack = self
                                    .undone_action_ids_by_layer
                                    .entry(layer_id.clone())
                                    .or_default();
                                if let Some(position) =
                                    undone_stack.iter().position(|id| id == reverted_id)
                                {
                                    undone_stack.remove(position);
                                    applied_stack.push(reverted_id.clone());
                                }
                            }
                        }
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

    /// Paints patches onto a block canvas for live stroke preview WITHOUT creating
    /// an action record or touching undo stacks. The committing record is appended
    /// at stroke release; because patches are absolute (after-value sets), applying
    /// the record again then is idempotent on already-staged content.
    pub fn stage_canvas_patches(
        &mut self,
        layer_id: &str,
        block_id: &str,
        patches: &[SharedCellPatch],
    ) {
        if patches.is_empty() {
            return;
        }
        self.unblank_raster_block(layer_id, block_id);
        let canvas = self
            .block_canvases
            .entry((layer_id.to_string(), block_id.to_string()))
            .or_default();
        apply_patches(canvas, patches, PatchDirection::After);
    }

    fn unblank_raster_block(&mut self, layer_id: &str, block_id: &str) {
        // Painting into a blank block turns it back into content.
        if let Some(layer) = self
            .document
            .layers
            .iter_mut()
            .find(|layer| layer.layer_id == layer_id)
        {
            if let Some(track) = layer
                .property_tracks
                .iter_mut()
                .find(|track| track.property_id == "raster")
            {
                if let Some(block) = track.blocks.iter_mut().find(|block| block.id == block_id) {
                    block.is_blank = false;
                }
            }
        }
    }

    /// Pops the layer's last applied action and reverts it on the canvas, returning
    /// the inverse patches so the caller can persist the undo as a passive
    /// `CellPatchSet` revert record. One undo = one whole committed stroke.
    pub fn undo_top_action(&mut self, layer_id: &str) -> Option<HistoryRevertPatches> {
        let action_id = self.applied_action_ids_by_layer.get_mut(layer_id)?.pop()?;
        let applied = self.patches_by_action_id.get(&action_id)?;
        let canvas = self
            .block_canvases
            .entry((applied.layer_id.clone(), applied.block_id.clone()))
            .or_default();
        apply_patches(canvas, &applied.patches, PatchDirection::Before);
        let revert = HistoryRevertPatches {
            block_id: applied.block_id.clone(),
            patches: applied
                .patches
                .iter()
                .map(SharedCellPatch::inverse)
                .collect(),
            action_id: action_id.clone(),
        };
        self.undone_action_ids_by_layer
            .entry(layer_id.to_string())
            .or_default()
            .push(action_id);
        Some(revert)
    }

    /// Pops the layer's last undone action and re-applies it, returning the
    /// re-application patches for a passive revert record.
    pub fn redo_top_action(&mut self, layer_id: &str) -> Option<HistoryRevertPatches> {
        let action_id = self.undone_action_ids_by_layer.get_mut(layer_id)?.pop()?;
        let applied = self.patches_by_action_id.get(&action_id)?;
        let canvas = self
            .block_canvases
            .entry((applied.layer_id.clone(), applied.block_id.clone()))
            .or_default();
        apply_patches(canvas, &applied.patches, PatchDirection::After);
        self.applied_action_ids_by_layer
            .entry(layer_id.to_string())
            .or_default()
            .push(action_id.clone());
        Some(HistoryRevertPatches {
            block_id: applied.block_id.clone(),
            patches: applied.patches.clone(),
            action_id,
        })
    }

    /// Appends an already-applied record to the in-memory history without applying
    /// it again. Used for passive revert records produced by undo/redo.
    pub fn push_history_record(&mut self, record: SharedDocumentActionRecord) {
        self.actions.push(record);
    }

    /// Folds history older than the undo-depth window into passive baseline
    /// records, so the saved action log stays bounded by content + `UNDO_HISTORY_DEPTH`
    /// records. In-memory undo stacks are untouched, so the session can still undo
    /// past the fold until the document is reloaded.
    pub fn fold_history(&mut self) {
        let len = self.actions.len();
        if len - self.squashed_through <= UNDO_HISTORY_DEPTH {
            return;
        }
        let new_cut = len - UNDO_HISTORY_DEPTH;
        let head = self.actions[..new_cut].to_vec();
        let replay = SharedDocumentRuntime::replay(self.document.clone(), head);
        let mut baseline = Vec::new();
        for ((layer_id, block_id), canvas) in &replay.block_canvases {
            if canvas.is_empty() {
                continue;
            }
            let patches: Vec<SharedCellPatch> = canvas
                .iter()
                .map(|(position, cell)| SharedCellPatch::new(*position, None, Some(cell)))
                .collect();
            baseline.push(
                SharedDocumentActionRecord::cell_patch_set(
                    format!("baseline-{layer_id}-{block_id}"),
                    self.document.document_id.clone(),
                    layer_id.clone(),
                    "history-squash",
                    "baseline",
                    patches,
                    Some(block_id.clone()),
                )
                .as_passive(),
            );
        }
        self.baseline = baseline;
        self.squashed_through = new_cut;
    }

    /// Reload this runtime from disk, adopting the on-disk truth wholesale and
    /// discarding local in-memory state (undo/redo stacks, staged patches, and any
    /// local edits not yet saved). This is the recovery path for a `changed on
    /// disk` revision conflict: the other writer's version wins, this session
    /// re-seeds its revision so the next save works again.
    pub fn reload_from_disk(&mut self, paths: &SharedDocumentPaths) -> Result<()> {
        let document = load_document_file(&paths.document_file_path)?;
        let actions = load_action_records(&paths.actions_file_path)?;
        *self = SharedDocumentRuntime::replay(document, actions);
        Ok(())
    }

    /// The records that belong in the saved log: squash baselines followed by every
    /// record not yet folded.
    pub fn actions_for_file(&self) -> Vec<SharedDocumentActionRecord> {
        let mut records = self.baseline.clone();
        records.extend(self.actions[self.squashed_through..].iter().cloned());
        records
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
    let value: Value = serde_json::from_str(&text)
        .with_context(|| format!("failed to parse shared document JSON at {}", path.display()))?;
    ensure_supported_file_schema(path, &value)?;
    serde_json::from_value(value)
        .with_context(|| format!("failed to parse shared document JSON at {}", path.display()))
}

/// A saved file exists but this build cannot open it: its kind or schema version
/// does not match what this code reads. Distinct from parse errors so the open
/// flow can reject cleanly — no partial state, no crash — with an explicit
/// version-difference message. The schema version only changes when the shape
/// breaks, so the version number alone describes the file's generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsupportedFileError {
    pub path: PathBuf,
    pub reason: UnsupportedFileReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnsupportedFileReason {
    KindMismatch { found: String, supported: String },
    VersionMismatch { found: u32, supported: u32 },
}

impl fmt::Display for UnsupportedFileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unsupported file at {} — ", self.path.display())?;
        match &self.reason {
            UnsupportedFileReason::KindMismatch { found, supported } => {
                write!(f, "file kind '{found}' is not '{supported}'")
            }
            UnsupportedFileReason::VersionMismatch { found, supported } => {
                if *found == 0 {
                    write!(f, "file carries no schema version; this app reads v{supported}")
                } else {
                    write!(f, "file is schema v{found}, this app reads v{supported}")
                }
            }
        }
    }
}

impl std::error::Error for UnsupportedFileError {}

/// The load-time schema gate: kind and version are checked BEFORE the body is
/// deserialized, so an old- or new-schema file is rejected as unsupported up
/// front instead of failing field-by-field (or worse, loading with silent
/// serde defaults). A missing kind or version field is treated as unsupported,
/// not malformed: the file predates or postdates this reader either way.
fn ensure_supported_file_schema(path: &Path, value: &Value) -> Result<()> {
    let unsupported = |reason| UnsupportedFileError {
        path: path.to_path_buf(),
        reason,
    };
    match value.get("file_kind").and_then(Value::as_str) {
        Some(kind) if kind == SHARED_DOCUMENT_KIND => {}
        found => {
            return Err(unsupported(UnsupportedFileReason::KindMismatch {
                found: found.unwrap_or("<missing>").to_string(),
                supported: SHARED_DOCUMENT_KIND.to_string(),
            })
            .into());
        }
    }
    let found = value.get("schema_version").and_then(Value::as_u64);
    if found != Some(SHARED_DOCUMENT_SCHEMA_VERSION as u64) {
        return Err(unsupported(UnsupportedFileReason::VersionMismatch {
            found: found.unwrap_or(0) as u32,
            supported: SHARED_DOCUMENT_SCHEMA_VERSION,
        })
        .into());
    }
    Ok(())
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

/// Counts non-empty action records currently on disk. Cheap divergence check for
/// `save_shared_document_snapshot`: another writer appends stroke/undo records
/// without snapshot saves, so the on-disk log can grow behind this runtime's back.
pub fn count_action_records(path: &Path) -> Result<usize> {
    if !path.exists() {
        return Ok(0);
    }
    let file = File::open(path)
        .with_context(|| format!("failed to open shared actions file at {}", path.display()))?;
    Ok(BufReader::new(file)
        .lines()
        .filter(|line| {
            line.as_ref()
                .map(|line| !line.trim().is_empty())
                .unwrap_or(false)
        })
        .count())
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
    // O_APPEND only makes the offset update atomic per write(2) syscall, and
    // `writeln!` on an unbuffered File issues one syscall per format piece (the
    // record bytes, then the newline). A rival writer appending inside that window
    // splices both records onto one line and corrupts the JSONL log. Serialize the
    // full line first so one `write_all` issues a single append syscall; local
    // filesystems serialize that write against other writers via the inode lock.
    let mut bytes = line.into_bytes();
    bytes.push(b'\n');
    file.write_all(&bytes)
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

    // The action log needs its own guard: strokes/undos are appended to
    // actions.jsonl WITHOUT a snapshot save, so another writer can add records
    // without touching document.json — and this function rewrites the whole log
    // from this writer's view. If the on-disk log holds records this runtime never
    // replayed, rewriting it here would silently drop the other writer's edits.
    // A revision bump from the other writer is caught above; this catches the
    // append-only case where the revision did not move. Only applies when the
    // target log already exists — a fresh root (save-as) has nothing to diverge
    // from, and comparing it against this runtime's history would always fail.
    if paths.actions_file_path.exists() {
        let on_disk_record_count = count_action_records(&paths.actions_file_path)?;
        let expected_record_count = runtime.actions_for_file().len();
        if on_disk_record_count != expected_record_count {
            return Err(anyhow::anyhow!(
                "shared action log changed on disk (disk has {} records, this session accounts for {}); \
                 refusing to overwrite — reload the document to pick up the other writer's changes",
                on_disk_record_count,
                expected_record_count
            ));
        }
    }

    let next_revision = runtime.revision + 1;
    let mut document = runtime.document.clone();
    document.revision = next_revision;
    write_document_atomic(&paths.document_file_path, &document)?;
    // Squash first so the saved log stays bounded: baselines + the last
    // UNDO_HISTORY_DEPTH records, never the full session history.
    runtime.fold_history();
    let records = runtime.actions_for_file();
    write_action_records_atomic(&paths.actions_file_path, &records)?;

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

pub(crate) fn material_name(material: CellMaterialId) -> &'static str {
    match material {
        CellMaterialId::GrayScale => "gray-scale",
    }
}

pub(crate) fn material_from_name(name: &str) -> Option<CellMaterialId> {
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

        let replayed =
            SharedDocumentRuntime::replay(runtime.document.clone(), runtime.actions.clone());
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
                    property_tracks: vec![default_raster_property_track(
                        0,
                        default_layer_length_breaths(),
                    )],
                },
                SharedDocumentLayer {
                    layer_id: "layer-2".to_string(),
                    name: "Layer 2".to_string(),
                    visible: true,
                    locked: false,
                    start_breath: 0,
                    length_breaths: default_layer_length_breaths(),
                    property_tracks: vec![default_raster_property_track(
                        0,
                        default_layer_length_breaths(),
                    )],
                },
            ],
            revision: 0,
            selection: SharedDocumentSelection::default(),
            document_window: DocumentWindow::default(),
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
        let stroke = SharedDocumentActionRecord::cell_patch_set(
            "a1",
            "doc-1",
            "layer-1",
            "u1",
            "1",
            vec![SharedCellPatch::new(point(1, 1), None, Some(&cell('#')))],
            Some("block-1".to_string()),
        );
        append_action_record(&paths.actions_file_path, &stroke).unwrap();
        runtime.apply_action_record(stroke);

        save_shared_document_snapshot(&paths, &mut runtime).unwrap();

        assert!(paths.document_file_path.exists());
        assert!(paths.actions_file_path.exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn save_as_to_a_fresh_root_accepts_this_runtime_action_history() {
        // Regression: save-as used to compare the fresh target's (missing)
        // actions log against this runtime's loaded history and always refuse,
        // which crashed the entrypoint. A fresh root has nothing to diverge
        // from — the snapshot carries baseline + recent records wholesale.
        let unique = format!(
            "thaum-painter-save-as-test-{}",
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
        let stroke = SharedDocumentActionRecord::cell_patch_set(
            "a1",
            "doc-1",
            "layer-1",
            "u1",
            "1",
            vec![SharedCellPatch::new(point(2, 2), None, Some(&cell('#')))],
            Some("block-1".to_string()),
        );
        runtime.push_history_record(stroke);

        save_shared_document_snapshot(&paths, &mut runtime).unwrap();

        assert!(paths.document_file_path.exists());
        assert_eq!(count_action_records(&paths.actions_file_path).unwrap(), 1);

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
    fn staged_stroke_patches_commit_idempotently_and_undo_as_one_stroke() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));
        let p = |x: i32, y: i32| CellPoint { x, y, z: 0 };
        let cell = |c: char| PaintedCell {
            graphic: thaum_renderer_domain::CellGraphic::Glyph(c),
            color: PaintColor::flat_rgb(1, 2, 3),
            weight_index: 0,
        };

        // Simulate a drag: chunks staged live, no records yet.
        runtime.stage_canvas_patches(
            "layer-1",
            "block-1",
            &[SharedCellPatch::new(p(1, 1), None, Some(&cell('A')))],
        );
        runtime.stage_canvas_patches(
            "layer-1",
            "block-1",
            &[SharedCellPatch::new(p(2, 1), None, Some(&cell('B')))],
        );
        assert!(runtime.actions.is_empty());
        assert_eq!(runtime.block_canvas("layer-1", "block-1").unwrap().len(), 2);

        // Release: one record covering the whole stroke; applying it over the
        // already-staged canvas must be idempotent.
        let patches = vec![
            SharedCellPatch::new(p(1, 1), None, Some(&cell('A'))),
            SharedCellPatch::new(p(2, 1), None, Some(&cell('B'))),
        ];
        let record = SharedDocumentActionRecord::cell_patch_set(
            "a1",
            "doc-1",
            "layer-1",
            "u1",
            "1",
            patches,
            Some("block-1".to_string()),
        );
        runtime.apply_action_record(record);
        assert_eq!(runtime.block_canvas("layer-1", "block-1").unwrap().len(), 2);

        // One undo reverts the entire stroke.
        let revert = runtime.undo_top_action("layer-1").unwrap();
        assert_eq!(revert.block_id, "block-1");
        assert_eq!(revert.patches.len(), 2);
        assert!(runtime
            .block_canvas("layer-1", "block-1")
            .unwrap()
            .is_empty());
        // Redo re-applies the same patches.
        let redo = runtime.redo_top_action("layer-1").unwrap();
        assert_eq!(redo.patches.len(), 2);
        assert_eq!(runtime.block_canvas("layer-1", "block-1").unwrap().len(), 2);
    }

    #[test]
    fn passive_records_paint_content_without_entering_undo_stacks() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));
        let cell = PaintedCell {
            graphic: thaum_renderer_domain::CellGraphic::Glyph('A'),
            color: PaintColor::flat_rgb(1, 1, 1),
            weight_index: 0,
        };
        runtime.apply_action_record(SharedDocumentActionRecord::cell_patch_set(
            "a1",
            "doc-1",
            "layer-1",
            "u1",
            "1",
            vec![SharedCellPatch::new(point(1, 1), None, Some(&cell))],
            Some("block-1".to_string()),
        ));
        // A passive record (squash baseline or undo revert) paints but must not
        // become an undo target.
        runtime.apply_action_record(
            SharedDocumentActionRecord::cell_patch_set(
                "baseline-1",
                "doc-1",
                "layer-1",
                "history-squash",
                "baseline",
                vec![SharedCellPatch::new(point(2, 1), None, Some(&cell))],
                Some("block-1".to_string()),
            )
            .as_passive(),
        );

        assert_eq!(runtime.block_canvas("layer-1", "block-1").unwrap().len(), 2);
        // Only the active record is undoable.
        assert!(runtime.undo_top_action("layer-1").is_some());
        assert!(runtime.undo_top_action("layer-1").is_none());
    }

    #[test]
    fn undo_revert_records_replay_to_the_same_state_without_undo_depth_growth() {
        let cell = |c: char| PaintedCell {
            graphic: thaum_renderer_domain::CellGraphic::Glyph(c),
            color: PaintColor::flat_rgb(1, 1, 1),
            weight_index: 0,
        };
        let paint = |id: &str, pos: (i32, i32), c: char| {
            SharedDocumentActionRecord::cell_patch_set(
                id,
                "doc-1",
                "layer-1",
                "u1",
                "1",
                vec![SharedCellPatch::new(
                    point(pos.0, pos.1),
                    None,
                    Some(&cell(c)),
                )],
                Some("block-1".to_string()),
            )
        };

        // In-session: paint A, paint B, undo B (revert record), undo A.
        let mut session = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));
        session.apply_action_record(paint("a1", (1, 1), 'A'));
        session.apply_action_record(paint("a2", (2, 1), 'B'));
        let revert_b = session.undo_top_action("layer-1").unwrap();
        session.push_history_record(SharedDocumentActionRecord::revert_patch_set(
            "r1",
            "doc-1",
            "layer-1",
            "u1",
            "2",
            revert_b.patches,
            Some("block-1".to_string()),
            revert_b.action_id,
        ));
        let revert_a = session.undo_top_action("layer-1").unwrap();
        session.push_history_record(SharedDocumentActionRecord::revert_patch_set(
            "r2",
            "doc-1",
            "layer-1",
            "u1",
            "3",
            revert_a.patches,
            Some("block-1".to_string()),
            revert_a.action_id,
        ));
        assert!(session
            .block_canvas("layer-1", "block-1")
            .unwrap()
            .is_empty());

        // Reload: replaying the log must produce the identical state, and undo
        // depth must not grow (passive records never enter the stacks).
        let mut reloaded = SharedDocumentRuntime::replay(
            SharedDocumentFile::single_layer("doc-1", "Doc", "layer-1", "Layer 1"),
            session.actions.clone(),
        );
        assert!(reloaded
            .block_canvas("layer-1", "block-1")
            .unwrap()
            .is_empty());
        assert!(reloaded.undo_top_action("layer-1").is_none());

        // Redoing both strokes still works in-session (undone stack intact).
        assert!(session.redo_top_action("layer-1").is_some());
        assert!(session.redo_top_action("layer-1").is_some());
        assert_eq!(session.block_canvas("layer-1", "block-1").unwrap().len(), 2);
    }

    #[test]
    fn fold_history_bounds_the_saved_log_and_replay_matches_the_final_state() {
        let cell = |c: char| PaintedCell {
            graphic: thaum_renderer_domain::CellGraphic::Glyph(c),
            color: PaintColor::flat_rgb(1, 1, 1),
            weight_index: 0,
        };
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));
        for index in 0..30 {
            runtime.apply_action_record(SharedDocumentActionRecord::cell_patch_set(
                format!("a{index}"),
                "doc-1",
                "layer-1",
                "u1",
                "1",
                vec![SharedCellPatch::new(
                    point(index, 1),
                    None,
                    Some(&cell('A')),
                )],
                Some("block-1".to_string()),
            ));
        }

        runtime.fold_history();
        let records = runtime.actions_for_file();
        assert_eq!(records.len(), UNDO_HISTORY_DEPTH + 1); // 1 baseline + last 20
        assert!(records[0].action_id.starts_with("baseline-"));

        // Replaying the folded log reproduces the exact final state.
        let mut reloaded = SharedDocumentRuntime::replay(
            SharedDocumentFile::single_layer("doc-1", "Doc", "layer-1", "Layer 1"),
            records,
        );
        assert_eq!(
            reloaded.block_canvas("layer-1", "block-1").unwrap(),
            runtime.block_canvas("layer-1", "block-1").unwrap()
        );
        // Undo depth after reload is bounded by the window.
        let mut undo_count = 0;
        while reloaded.undo_top_action("layer-1").is_some() {
            undo_count += 1;
        }
        assert_eq!(undo_count, UNDO_HISTORY_DEPTH);
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
            let mut fallback =
                SharedDocumentFile::single_layer("doc-1", "Doc", "layer-1", "Layer 1");
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
            let mut fallback =
                SharedDocumentFile::single_layer("doc-1", "Doc", "layer-1", "Layer 1");
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
    fn reload_from_disk_adopts_the_other_writer_and_clears_local_history() {
        let unique = format!(
            "thaum-painter-reload-test-{}",
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

        // Local session state that must not survive a reload: an applied stroke
        // plus its undo/redo bookkeeping.
        let stroke = SharedDocumentActionRecord::cell_patch_set(
            "local-stroke",
            "doc-1",
            "layer-1",
            "user-1",
            "stroke",
            vec![SharedCellPatch::new(point(1, 1), None, Some(&cell('X')))],
            None,
        );
        writer.apply_action_record(stroke);
        assert!(writer.undo_top_action("layer-1").is_some());

        // A rival writer appends its record to the shared log, applies it, and
        // snapshot-saves — the real writer flow.
        let mut rival = SharedDocumentRuntime::replay(
            load_document_file(&paths.document_file_path).unwrap(),
            load_action_records(&paths.actions_file_path).unwrap(),
        );
        let rival_stroke = SharedDocumentActionRecord::cell_patch_set(
            "rival-stroke",
            "doc-1",
            "layer-1",
            "user-2",
            "stroke",
            vec![SharedCellPatch::new(point(2, 2), None, Some(&cell('Y')))],
            None,
        );
        append_action_record(&paths.actions_file_path, &rival_stroke).unwrap();
        rival.apply_action_record(rival_stroke);
        save_shared_document_snapshot(&paths, &mut rival).unwrap();

        // The local save hits the conflict, then reload adopts the rival's state.
        assert!(save_shared_document_snapshot(&paths, &mut writer).is_err());
        writer.reload_from_disk(&paths).unwrap();
        assert_eq!(writer.revision(), rival.revision());
        assert_eq!(writer.actions.len(), 1);
        assert_eq!(writer.actions[0].action_id, "rival-stroke");

        // Local undo history is gone: the rival's content is live, and the next
        // save succeeds from the re-seeded revision.
        assert_eq!(
            writer
                .canvas_for_layer("layer-1", 0)
                .unwrap()
                .get(&point(2, 2))
                .map(|c| c.graphic.clone()),
            Some(cell('Y').graphic)
        );
        assert!(writer
            .canvas_for_layer("layer-1", 0)
            .unwrap()
            .get(&point(1, 1))
            .is_none());
        save_shared_document_snapshot(&paths, &mut writer).unwrap();
        assert_eq!(writer.revision(), rival.revision() + 1);

        // One undo reverts the rival's stroke; a second undo finds nothing, so the
        // local session's pre-reload history did not survive the reload.
        assert!(writer.undo_top_action("layer-1").is_some());
        assert!(writer
            .canvas_for_layer("layer-1", 0)
            .unwrap()
            .get(&point(2, 2))
            .is_none());
        assert!(writer.undo_top_action("layer-1").is_none());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn snapshot_save_refuses_to_clobber_records_appended_without_a_snapshot() {
        let unique = format!(
            "thaum-painter-append-conflict-test-{}",
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

        // A rival appends a stroke to the shared log without snapshot-saving, so
        // document.json's revision does not move.
        let rival_stroke = SharedDocumentActionRecord::cell_patch_set(
            "rival-stroke",
            "doc-1",
            "layer-1",
            "user-2",
            "stroke",
            vec![SharedCellPatch::new(point(2, 2), None, Some(&cell('Y')))],
            None,
        );
        append_action_record(&paths.actions_file_path, &rival_stroke).unwrap();

        // The stale writer's snapshot save must refuse — rewriting the log from its
        // stale view would silently drop the rival's appended record.
        let error = save_shared_document_snapshot(&paths, &mut writer)
            .expect_err("stale writer must not rewrite a log holding unknown records");
        assert!(error.to_string().contains("changed on disk"));

        // Reload adopts the rival's record and the next save succeeds.
        writer.reload_from_disk(&paths).unwrap();
        assert_eq!(writer.actions.len(), 1);
        assert_eq!(writer.actions[0].action_id, "rival-stroke");
        save_shared_document_snapshot(&paths, &mut writer).unwrap();

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn large_record_appends_stay_whole_records() {
        // The append seam must never tear a record across write syscalls: a record
        // far past the 4KB folklore-atomicity threshold is serialized into one
        // buffer, so two writers appending to the same log still leave every line
        // a whole JSON object.
        let unique = format!(
            "append-atomicity-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        let paths = SharedDocumentPaths::new(root.join("doc"));
        let patch_count = 2000;
        let patches: Vec<SharedCellPatch> = (0..patch_count)
            .map(|i| {
                SharedCellPatch::new(point(i as i32 % 64, i as i32 / 64), None, Some(&cell('X')))
            })
            .collect();
        let record_for = |writer: &str| {
            SharedDocumentActionRecord::cell_patch_set(
                format!("stroke-{writer}"),
                "doc-1",
                "layer-1",
                writer,
                "stroke",
                patches.clone(),
                None,
            )
        };

        append_action_record(&paths.actions_file_path, &record_for("writer-1")).unwrap();
        append_action_record(&paths.actions_file_path, &record_for("writer-2")).unwrap();

        let actions = load_action_records(&paths.actions_file_path).unwrap();
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].action_id, "stroke-writer-1");
        assert_eq!(actions[1].action_id, "stroke-writer-2");
        for action in &actions {
            let SharedDocumentAction::CellPatchSet { patches, .. } = &action.action else {
                panic!("unexpected record shape {:?}", action.action_id);
            };
            assert_eq!(patches.len(), patch_count);
        }

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
            "private",
            [p(0, 0)],
            SharedSelectionWriteMode::Additive
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
            let mut fallback =
                SharedDocumentFile::single_layer("doc-1", "Doc", "layer-1", "Layer 1");
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
    fn load_rejects_a_newer_schema_version_with_an_explicit_message() {
        let unique = format!(
            "thaum-painter-storage-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        let paths = SharedDocumentPaths::new(root.clone());
        let mut document = SharedDocumentFile::single_layer("doc-1", "Doc", "layer-1", "Layer 1");
        document.schema_version = SHARED_DOCUMENT_SCHEMA_VERSION + 1;
        write_document_atomic(&paths.document_file_path, &document).unwrap();

        let error = load_document_file(&paths.document_file_path).unwrap_err();
        let unsupported = error
            .downcast_ref::<UnsupportedFileError>()
            .expect("load failure must be the typed unsupported-file error");
        assert_eq!(
            unsupported.reason,
            UnsupportedFileReason::VersionMismatch {
                found: SHARED_DOCUMENT_SCHEMA_VERSION + 1,
                supported: SHARED_DOCUMENT_SCHEMA_VERSION,
            }
        );
        assert!(error
            .to_string()
            .contains(&format!("file is schema v{}, this app reads v{}", SHARED_DOCUMENT_SCHEMA_VERSION + 1, SHARED_DOCUMENT_SCHEMA_VERSION)));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn load_rejects_a_wrong_kind_file_without_deserializing_its_body() {
        let unique = format!(
            "thaum-painter-storage-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        fs::create_dir_all(&root).unwrap();
        let path = root.join("foreign.json");
        fs::write(
            &path,
            r#"{ "file_kind": "not-thaum-painter", "schema_version": 1, "document_id": "x" }"#,
        )
        .unwrap();

        let error = load_document_file(&path).unwrap_err();
        let unsupported = error
            .downcast_ref::<UnsupportedFileError>()
            .expect("load failure must be the typed unsupported-file error");
        assert_eq!(
            unsupported.reason,
            UnsupportedFileReason::KindMismatch {
                found: "not-thaum-painter".to_string(),
                supported: SHARED_DOCUMENT_KIND.to_string(),
            }
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn load_rejects_a_file_missing_kind_or_version_as_unsupported() {
        let unique = format!(
            "thaum-painter-storage-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        fs::create_dir_all(&root).unwrap();
        // Right kind but no version field: the version gate must fire with the
        // explicit "carries no schema version" message.
        let path = root.join("unversioned.json");
        fs::write(
            &path,
            format!(
                r#"{{ "file_kind": "{}", "document_id": "doc-1", "title": "Doc", "layers": [] }}"#,
                SHARED_DOCUMENT_KIND
            ),
        )
        .unwrap();
        let error = load_document_file(&path).unwrap_err();
        assert!(error.downcast_ref::<UnsupportedFileError>().is_some());
        assert!(error.to_string().contains("carries no schema version"));

        // No kind field at all: the kind gate must fire with the explicit
        // "<missing>" message.
        let path = root.join("kindless.json");
        fs::write(
            &path,
            r#"{ "document_id": "doc-1", "title": "Doc", "layers": [] }"#,
        )
        .unwrap();
        let error = load_document_file(&path).unwrap_err();
        assert!(error.downcast_ref::<UnsupportedFileError>().is_some());
        assert!(error.to_string().contains("file kind '<missing>'"));

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
            Some("block-1".to_string()),
        ));

        assert!(runtime.set_layer_visible("layer-1", false));
        assert!(runtime.composited_canvas_in_layer_order(0).is_empty());

        assert!(runtime.set_layer_visible("layer-1", true));
        assert_eq!(
            runtime
                .composited_canvas_in_layer_order(0)
                .get(&point(0, 0)),
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
    fn pushed_timing_shrink_of_the_last_block_fills_the_tail_blank() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));

        // block-1 (0..24) is the only block, so a pushed shrink has nothing to
        // pull left: the vacated tail re-tiles into a blank block — a bare void
        // would leave the channel with two kinds of no-content.
        assert!(
            runtime.set_property_block_timing_pushed("layer-1", "raster", "block-1", 0, 8)
        );
        let track = runtime.property_track("layer-1", "raster").unwrap();
        let covered: Vec<(u32, u32, bool)> = track
            .blocks
            .iter()
            .map(|b| (b.start_breath, b.start_breath + b.length_breaths, b.is_blank))
            .collect();
        assert_eq!(covered, vec![(0, 8, false), (8, 24, true)]);
    }

    #[test]
    fn pushed_timing_clamped_at_breath_zero_never_leaves_an_overlap() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));
        runtime.split_property_block("layer-1", "raster", "block-1", 8);

        // Drag block-2's start edge (8..24) left to 2: block-1 must shift left
        // but is already at breath zero, so the shift clamps. The seam re-tiles,
        // trimming the overlapped range instead of leaving bars on both sides
        // of the same breath.
        assert!(
            runtime.set_property_block_timing_pushed("layer-1", "raster", "block-2", 2, 16)
        );
        let track = runtime.property_track("layer-1", "raster").unwrap();
        let covered: Vec<(u32, u32, bool)> = track
            .blocks
            .iter()
            .map(|b| (b.start_breath, b.start_breath + b.length_breaths, b.is_blank))
            .collect();
        assert_eq!(
            covered,
            vec![(0, 8, false), (8, 18, false), (18, 24, true)]
        );
    }

    #[test]
    fn adjacent_empties_merge_into_one_on_retile() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));

        // Blanking block-1 (0..24) leaves one empty; shrinking the layer span and
        // growing it back must not produce two touching empties — the invariant is
        // that the user only ever sees empties and bars, never two empties touching.
        assert!(runtime.blank_property_block("layer-1", "raster", "block-1"));
        assert!(runtime.set_layer_timing("layer-1", 0, 12));
        assert!(runtime.set_layer_timing("layer-1", 0, 24));
        let track = runtime.property_track("layer-1", "raster").unwrap();
        assert_eq!(track.blocks.len(), 1);
        assert!(track.blocks[0].is_blank);
        assert_eq!((track.blocks[0].start_breath, track.blocks[0].length_breaths), (0, 24));
    }

    #[test]
    fn blanking_next_to_an_existing_empty_merges_them() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));
        runtime.split_property_block("layer-1", "raster", "block-1", 8);

        // block-1 (0..8) and block-2 (8..24) both blanked: one empty, not two.
        assert!(runtime.blank_property_block("layer-1", "raster", "block-1"));
        assert!(runtime.blank_property_block("layer-1", "raster", "block-2"));
        let track = runtime.property_track("layer-1", "raster").unwrap();
        assert_eq!(track.blocks.len(), 1);
        assert!(track.blocks[0].is_blank);
    }

    #[test]
    fn merge_empty_prefers_the_left_content_block() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));
        runtime.split_property_block("layer-1", "raster", "block-1", 8);
        assert!(runtime.blank_property_block("layer-1", "raster", "block-1"));
        assert!(runtime.split_property_block("layer-1", "raster", "block-2", 16).is_some());

        // Track: empty (0..8), content (8..16), content (16..24). Merging the empty
        // prefers the left... there is no content on the left, so it falls back to
        // the right: content (8..16) expands left over the empty.
        assert!(runtime.merge_empty_property_block("layer-1", "raster", "block-1", true));
        let track = runtime.property_track("layer-1", "raster").unwrap();
        let covered: Vec<(u32, u32, bool)> = track
            .blocks
            .iter()
            .map(|b| (b.start_breath, b.start_breath + b.length_breaths, b.is_blank))
            .collect();
        assert_eq!(covered, vec![(0, 16, false), (16, 24, false)]);
    }

    #[test]
    fn merge_empty_prefers_left_when_both_sides_have_content() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));
        runtime.split_property_block("layer-1", "raster", "block-1", 8);
        runtime.split_property_block("layer-1", "raster", "block-2", 16);

        // Track: content (0..8), content (8..16), content (16..24). Blank the middle
        // and merge it: the left neighbor (0..8) expands over it, keeping its content.
        assert!(runtime.blank_property_block("layer-1", "raster", "block-2"));
        assert!(runtime.merge_empty_property_block("layer-1", "raster", "block-2", true));
        let track = runtime.property_track("layer-1", "raster").unwrap();
        let covered: Vec<(u32, u32, bool)> = track
            .blocks
            .iter()
            .map(|b| (b.start_breath, b.start_breath + b.length_breaths, b.is_blank))
            .collect();
        assert_eq!(covered, vec![(0, 16, false), (16, 24, false)]);
    }

    #[test]
    fn merge_empty_on_a_fully_empty_track_rejects() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));

        // The fresh track is one empty covering the whole span: no content anywhere,
        // so the merge interaction rejects.
        assert!(!runtime.merge_empty_property_block("layer-1", "raster", "block-1", true));
    }

    #[test]
    fn merge_empty_rejects_content_blocks() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));
        assert!(!runtime.merge_empty_property_block("layer-1", "raster", "block-1", true));
    }

    #[test]
    fn duplicate_lands_on_the_right_pushing_later_blocks() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));
        runtime.split_property_block("layer-1", "raster", "block-1", 8);

        // Track: content (0..8), content (8..24). Duplicating block-1 lands a copy
        // at (8..16) and pushes block-2 right by 8 — non-destructive.
        let new_id = runtime
            .duplicate_property_block("layer-1", "raster", "block-1")
            .expect("duplicate");
        let track = runtime.property_track("layer-1", "raster").unwrap();
        let covered: Vec<(u32, u32, bool)> = track
            .blocks
            .iter()
            .map(|b| (b.start_breath, b.start_breath + b.length_breaths, b.is_blank))
            .collect();
        assert_eq!(
            covered,
            vec![(0, 8, false), (8, 16, false), (16, 32, false)]
        );
        assert_ne!(new_id, "block-1");
    }

    #[test]
    fn duplicate_at_the_span_end_grows_the_layer_by_exactly_the_duplicate() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));

        // block-1 (0..24) ends at the span end: the duplicate lands at (24..48) and
        // the layer grows by exactly 24 — bounded, one interaction at a time.
        let _new_id = runtime
            .duplicate_property_block("layer-1", "raster", "block-1")
            .expect("duplicate");
        let layer = runtime
            .layers()
            .iter()
            .find(|layer| layer.layer_id == "layer-1")
            .unwrap();
        assert_eq!((layer.start_breath, layer.length_breaths), (0, 48));
        let track = runtime.property_track("layer-1", "raster").unwrap();
        let covered: Vec<(u32, u32, bool)> = track
            .blocks
            .iter()
            .map(|b| (b.start_breath, b.start_breath + b.length_breaths, b.is_blank))
            .collect();
        assert_eq!(covered, vec![(0, 24, false), (24, 48, false)]);
    }

    #[test]
    fn duplicate_does_not_disturb_earlier_blocks() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));
        runtime.split_property_block("layer-1", "raster", "block-1", 8);
        runtime.split_property_block("layer-1", "raster", "block-2", 16);

        // Track: (0..8), (8..16), (16..24). Duplicating block-2 shifts only block-3.
        let new_id = runtime
            .duplicate_property_block("layer-1", "raster", "block-2")
            .expect("duplicate");
        let track = runtime.property_track("layer-1", "raster").unwrap();
        let ids: Vec<&str> = track.blocks.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(ids, vec!["block-1", "block-2", new_id.as_str(), "block-3"]);
        assert_eq!(track.blocks[0].start_breath, 0);
        assert_eq!(track.blocks[1].start_breath, 8);
        assert_eq!(track.blocks[2].start_breath, 16);
        assert_eq!(track.blocks[3].start_breath, 24);
    }

    #[test]
    fn split_rejects_blank_blocks() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));
        assert!(runtime.blank_property_block("layer-1", "raster", "block-1"));
        assert!(runtime.split_property_block("layer-1", "raster", "block-1", 8).is_none());
    }

    #[test]
    fn destructive_timing_victims_become_blanks_and_the_track_stays_tiled() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));
        runtime.split_property_block("layer-1", "raster", "block-1", 8);
        runtime.set_property_block_timing_destructive("layer-1", "raster", "block-2", 16, 8);

        // block-1 (0..8, empty) slides destructively into the middle of block-2
        // (16..24): the victim keeps only its pre-edited remainder and turns blank,
        // and every vacated window re-tiles into blank blocks — no void anywhere.
        // The two adjacent victim blanks merge into one (no-adjacent-empties).
        assert!(
            runtime.set_property_block_timing_destructive("layer-1", "raster", "block-1", 18, 4)
        );
        let track = runtime.property_track("layer-1", "raster").unwrap();
        let covered: Vec<(u32, u32, bool)> = track
            .blocks
            .iter()
            .map(|b| (b.start_breath, b.start_breath + b.length_breaths, b.is_blank))
            .collect();
        assert_eq!(
            covered,
            vec![(0, 18, true), (18, 22, false), (22, 24, true)]
        );
    }

    #[test]
    fn destructive_timing_shrink_leaves_the_vacated_range_blank() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));

        // block-1 (0..24) shrinks to 0..8: breaths 8..24 re-tile into blanks.
        assert!(
            runtime.set_property_block_timing_destructive("layer-1", "raster", "block-1", 0, 8)
        );
        let track = runtime.property_track("layer-1", "raster").unwrap();
        let covered: Vec<(u32, u32, bool)> = track
            .blocks
            .iter()
            .map(|b| (b.start_breath, b.start_breath + b.length_breaths, b.is_blank))
            .collect();
        assert_eq!(covered, vec![(0, 8, false), (8, 24, true)]);
    }

    #[test]
    fn destructive_timing_removes_fully_covered_neighbors() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));
        runtime.split_property_block("layer-1", "raster", "block-1", 8);

        // block-1 (0..8) grows destructively to 0..24: block-2 (8..24) is fully
        // covered, so it is removed outright — the edited block covers its range.
        assert!(
            runtime.set_property_block_timing_destructive("layer-1", "raster", "block-1", 0, 24)
        );
        let blocks = &runtime.property_track("layer-1", "raster").unwrap().blocks;
        assert_eq!(blocks.len(), 1);
        assert_eq!((blocks[0].start_breath, blocks[0].length_breaths), (0, 24));
        assert!(!blocks[0].is_blank);
    }

    #[test]
    fn set_layer_timing_retiles_tracks_to_the_new_span() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));
        runtime.split_property_block("layer-1", "raster", "block-1", 8);

        // Grow the layer span to 32: the uncovered tail re-tiles into a blank block.
        assert!(runtime.set_layer_timing("layer-1", 0, 32));
        let track = runtime.property_track("layer-1", "raster").unwrap();
        let covered: Vec<(u32, u32, bool)> = track
            .blocks
            .iter()
            .map(|b| (b.start_breath, b.start_breath + b.length_breaths, b.is_blank))
            .collect();
        assert_eq!(
            covered,
            vec![(0, 8, false), (8, 24, false), (24, 32, true)]
        );

        // Shrink the span to 12: the block past the end clips into it.
        assert!(runtime.set_layer_timing("layer-1", 0, 12));
        let track = runtime.property_track("layer-1", "raster").unwrap();
        let covered: Vec<(u32, u32, bool)> = track
            .blocks
            .iter()
            .map(|b| (b.start_breath, b.start_breath + b.length_breaths, b.is_blank))
            .collect();
        assert_eq!(covered, vec![(0, 8, false), (8, 12, false)]);
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
    fn first_breath_with_raster_block_seeks_content_and_skips_leading_gaps() {
        let document = SharedDocumentFile::single_layer("doc-1", "Doc", "layer-1", "Layer 1");
        let mut runtime = SharedDocumentRuntime::new(document);
        // Default: one block starting at breath 0.
        assert_eq!(runtime.first_breath_with_raster_block("layer-1"), Some(0));

        // Split at 8, blank the leading block so breaths 0..8 are a real gap
        // (split alone preserves is_blank), then paint only into the later block:
        // the seek lands on the first block with content, not the gap.
        runtime.split_property_block("layer-1", "raster", "block-1", 8);
        runtime.split_property_block("layer-1", "raster", "block-2", 16);
        assert!(runtime.blank_property_block("layer-1", "raster", "block-1"));
        runtime.apply_action_record(SharedDocumentActionRecord::cell_patch_set(
            "a1",
            "doc-1",
            "layer-1",
            "u1",
            "1",
            vec![SharedCellPatch::new(point(0, 0), None, Some(&cell('A')))],
            Some("block-2".to_string()),
        ));
        assert_eq!(runtime.first_breath_with_raster_block("layer-1"), Some(8));

        // Unknown layers have no raster track to seek.
        assert_eq!(runtime.first_breath_with_raster_block("missing"), None);
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
            runtime
                .canvas_for_layer("layer-1", 5)
                .unwrap()
                .get(&point(0, 0)),
            Some(&cell('A'))
        );
        // block-2 has no content yet, so later breaths render empty.
        assert!(runtime.canvas_for_layer("layer-1", 10).unwrap().is_empty());
        // A breath in a gap between blocks has no canvas at all.
        assert!(runtime.canvas_for_layer("layer-1", 99).is_none());

        let replayed =
            SharedDocumentRuntime::replay(runtime.document.clone(), runtime.actions.clone());
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
        assert!(
            !blocks
                .iter()
                .find(|block| block.id == "block-2")
                .unwrap()
                .is_blank
        );
        assert_eq!(
            runtime
                .canvas_for_layer("layer-1", 10)
                .unwrap()
                .get(&point(0, 0)),
            Some(&cell('B'))
        );
        // The first block's content is untouched.
        assert_eq!(
            runtime
                .canvas_for_layer("layer-1", 0)
                .unwrap()
                .get(&point(0, 0)),
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
            runtime
                .canvas_for_layer("layer-1", 10)
                .unwrap()
                .get(&point(0, 0)),
            Some(&cell('A'))
        );
        assert!(runtime.canvas_for_layer("layer-1", 0).unwrap().is_empty());

        let replayed =
            SharedDocumentRuntime::replay(runtime.document.clone(), runtime.actions.clone());
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
            runtime
                .canvas_for_layer("layer-1", 10)
                .unwrap()
                .get(&point(0, 0)),
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
