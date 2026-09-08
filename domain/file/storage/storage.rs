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

use crate::interp_mode;
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

/// Which ease end a cycle targets — the left end eases out, the right end in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EaseEnd {
    Out,
    In,
}

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
    /// Interpretation-mode slot for the END blank keyframes (J 2026-09-07): the
    /// trailing blank's mode is what happens at infinity (e.g. loop out), the
    /// leading blank's is the mirror for negative time. Vocabulary and cycling
    /// live in `properties/interp_mode.rs` (interpolate / hold / loop_out /
    /// loop_in). `serde(default)` keeps existing files loading unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interpretation: Option<String>,
    /// Ease-out strength on the empty's left end (J 2026-09-07), percent of the
    /// `interp_mode::EASE_STRENGTHS` steps (unset = 0% linear). Only meaningful
    /// while the mode is ease-adjustable; cycling onto hold / loop modes clears
    /// it. `serde(default)` keeps existing files loading unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ease_out_percent: Option<u8>,
    /// Ease-in strength on the empty's right end — mirror of `ease_out_percent`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ease_in_percent: Option<u8>,
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

/// Re-tiles one property track's blocks so the track covers from the span start out
/// to INFINITY (J 2026-09-07): blocks clip on the left edge only, every uncovered
/// window becomes a blank block, adjacent blanks merge into one, and the track always
/// ends with a blank — the trailing blank, the finite representative of the infinite
/// empty region. Everything past its stored extent resolves as that same blank at
/// render and hit-test time; no infinite block is ever stored.
fn retiled_property_track(
    blocks: Vec<SharedDocumentPropertyBlock>,
    span_start: u32,
    span_length: u32,
) -> Vec<SharedDocumentPropertyBlock> {
    let span_end = span_start.saturating_add(span_length.max(1));
    let mut clipped: Vec<SharedDocumentPropertyBlock> = Vec::new();
    for mut block in blocks {
        // Left clip only: the span end is a viewport boundary, not a data wall.
        let new_start = block.start_breath.max(span_start);
        if block.start_breath + block.length_breaths.max(1) <= new_start {
            // Fully left of the span — nothing left to cover.
            continue;
        }
        if block.start_breath < new_start {
            let trimmed = block.length_breaths.max(1) - (new_start - block.start_breath);
            block.start_breath = new_start;
            block.length_breaths = trimmed;
        }
        clipped.push(block);
    }
    clipped.sort_by_key(|block| block.start_breath);
    // Gap and tail blanks must not reuse an id any clipped block still holds: ids are
    // looked up across the whole track (highlight, swap, duplicate, merge), so a
    // duplicate id makes two blocks light up as one and routes edits to the wrong bar.
    let mut used_ids: Vec<String> = clipped.iter().map(|b| b.id.clone()).collect();
    let next_free_block_id =
        |used_ids: &mut Vec<String>, tiled: &[SharedDocumentPropertyBlock]| -> String {
            let mut id = next_property_block_id(tiled);
            while used_ids.contains(&id) {
                id = format!("{id}x");
            }
            used_ids.push(id.clone());
            id
        };
    let mut tiled: Vec<SharedDocumentPropertyBlock> = Vec::with_capacity(clipped.len() + 2);
    let mut cursor = span_start;
    for mut block in clipped {
        let block_end = block.start_breath + block.length_breaths;
        if block_end <= cursor {
            // Fully covered by an earlier block (only possible in broken data); drop it.
            continue;
        }
        if block.start_breath > cursor {
            let gap_id = next_free_block_id(&mut used_ids, &tiled);
            push_tiled_block(
                &mut tiled,
                SharedDocumentPropertyBlock {
                    id: gap_id,
                    start_breath: cursor,
                    length_breaths: block.start_breath - cursor,
                    is_blank: true,
                    value: None,
                    interpretation: None,
                    ease_out_percent: None,
                    ease_in_percent: None,
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
        let tail_id = next_free_block_id(&mut used_ids, &tiled);
        push_tiled_block(
            &mut tiled,
            SharedDocumentPropertyBlock {
                id: tail_id,
                start_breath: cursor,
                length_breaths: span_end - cursor,
                is_blank: true,
                value: None,
                interpretation: None,
                ease_out_percent: None,
                ease_in_percent: None,
            },
        );
    }
    // The trailing blank: the last block must be blank so the infinite empty region
    // always has a stored representative to carry its interpretation and receive
    // its interactions. If the last block is solid (possibly past the viewport end),
    // append the minimal representative right after it. The representative carries
    // the semantic id "tail" (dedup'd) — it is THE end-blank keyframe, and it must
    // not consume numeric ids future content blocks expect to grow into.
    if let Some(last) = tiled.last() {
        if !last.is_blank {
            let tail_start = last.start_breath + last.length_breaths;
            let tail_id = if used_ids.iter().any(|used| used == "tail") {
                next_free_block_id(&mut used_ids, &tiled)
            } else {
                "tail".to_string()
            };
            used_ids.push(tail_id.clone());
            let tail_length = if tail_start < span_end {
                span_end - tail_start
            } else {
                1
            };
            tiled.push(SharedDocumentPropertyBlock {
                id: tail_id,
                start_breath: tail_start,
                length_breaths: tail_length,
                is_blank: true,
                value: None,
                interpretation: None,
                ease_out_percent: None,
                ease_in_percent: None,
            });
        }
    }
    enforce_loop_mode_edges(&mut tiled);
    tiled
}

/// Loop-mode edge enforcement (J 2026-09-07): `loop_out` only survives on the
/// track's LAST blank — the trailing blank, the right edge of time — and
/// `loop_in` only on the FIRST blank — the leading blank, the left edge. Any
/// loop mode stranded elsewhere by a swap, drag, split, duplicate, or track
/// growth drops back to the default interpolate mode with cleared ease ends.
/// Interpretation data on a solid block is dead (solids hold their value
/// outright) and is stripped for hygiene.
fn enforce_loop_mode_edges(blocks: &mut [SharedDocumentPropertyBlock]) {
    let last_index = blocks.len().saturating_sub(1);
    for (index, block) in blocks.iter_mut().enumerate() {
        let stranded = match interp_mode::resolve_mode(block.interpretation.as_deref()) {
            "loop_out" => !(block.is_blank && index == last_index),
            "loop_in" => !(block.is_blank && index == 0),
            _ => false,
        };
        if stranded || !block.is_blank {
            block.interpretation = None;
            block.ease_out_percent = None;
            block.ease_in_percent = None;
        }
    }
}

fn default_property_track(
    property_id: &str,
    start_breath: u32,
    length_breaths: u32,
) -> SharedDocumentPropertyTrack {
    // Every property kind is born on the same binary tiling as raster (J
    // 2026-09-07): one solid block over the viewport plus the trailing blank
    // representative — a user never sees an empty property row.
    SharedDocumentPropertyTrack {
        property_id: property_id.to_string(),
        blocks: retiled_property_track(
            vec![SharedDocumentPropertyBlock {
                id: "block-1".to_string(),
                start_breath,
                length_breaths: length_breaths.max(1),
                is_blank: false,
                value: None,
                interpretation: None,
                ease_out_percent: None,
                ease_in_percent: None,
            }],
            start_breath,
            length_breaths,
        ),
    }
}

/// Parses a move block's `{x,y,z}` offset value — the same shape the old
/// system's move properties carried and the file-schema import path parses.
pub(crate) fn parse_move_offset(value: &Value) -> Option<[i32; 3]> {
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
                property_tracks: vec![
                    default_property_track("raster", 0, default_layer_length_breaths()),
                    default_property_track("move", 0, default_layer_length_breaths()),
                ],
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
        // Normalize every property track into the current binary tiling on the
        // way in (J 2026-09-07): documents saved before the binary-bars work (or
        // by an older build) load with solid+tail tiling, merged blanks, and
        // loop modes re-locked to their edges — the new-layer rows show their
        // empties. In-memory normalization, not a file migration pass.
        let mut document = document;
        for layer in &mut document.layers {
            for track in &mut layer.property_tracks {
                track.blocks = retiled_property_track(
                    std::mem::take(&mut track.blocks),
                    layer.start_breath,
                    layer.length_breaths,
                );
            }
        }
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

    /// Ensures every block on the layer's raster track has a canvas entry (empty
    /// for new blanks) and that blank blocks' canvases are actually empty. Only
    /// raster blocks carry canvases; the move and other value channels live
    /// entirely in block values. Re-tiles mint fresh ids for gap/tail blanks,
    /// so every structural seam calls this instead of hand-inserting entries —
    /// a missing entry would make a solid render nothing silently, and a stale
    /// entry (removed blocks leave residue; id reuse can resurrect it onto a
    /// fresh blank) would render ghost content through the edit-surface seam.
    fn ensure_block_canvas_coverage(&mut self, layer_id: &str) {
        let Some(track) = self
            .document
            .layers
            .iter()
            .find(|layer| layer.layer_id == layer_id)
            .and_then(|layer| {
                layer
                    .property_tracks
                    .iter()
                    .find(|track| track.property_id == "raster")
            })
            .map(|track| {
                track
                    .blocks
                    .iter()
                    .map(|block| (block.id.clone(), block.is_blank))
                    .collect::<Vec<_>>()
            })
        else {
            return;
        };
        for (block_id, is_blank) in track {
            let entry = self
                .block_canvases
                .entry((layer_id.to_string(), block_id))
                .or_default();
            if is_blank && !entry.is_empty() {
                entry.clear();
            }
        }
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

    /// The layer's RESOLVED canvas at `current_breath`: a solid raster block
    /// renders its own canvas, but an interpolating empty blends the surrounding
    /// keyframes' canvases (color/weight lerp, discrete graphic cutoff — see
    /// `interp_raster`). This is the render/compositing read seam; the raw block
    /// canvas behind `canvas_for_layer` stays the edit-surface seam, so strokes
    /// still author real keyframes instead of painting into a blend.
    pub fn resolved_canvas_for_layer(
        &self,
        layer_id: &str,
        current_breath: u32,
        graphic_fade: Option<&thaum_renderer_domain::shape_fade::fade::ShapeFade>,
    ) -> Option<Canvas> {
        let track = self
            .document
            .layers
            .iter()
            .find(|layer| layer.layer_id == layer_id)?
            .property_tracks
            .iter()
            .find(|track| track.property_id == "raster")?;
        let canvases = &self.block_canvases;
        crate::interp_raster::resolve_raster_canvas(
            &track.blocks,
            current_breath,
            |block| {
                canvases
                    .get(&(layer_id.to_string(), block.id.clone()))
                    .cloned()
            },
            graphic_fade,
        )
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

    /// The layer's move offset at `breath`, resolved through the move channel's
    /// own interpolation (`interp_move`): solids hold their offset, empties
    /// resolve per their authored mode (hold / interpolate with eases /
    /// edge-locked loop repeats). Zero offset when the layer has no move track
    /// or nothing resolves — move offsets are optional metadata.
    pub fn move_offset_for_layer(&self, layer_id: &str, breath: u32) -> WorldPoint {
        let Some(track) = self.property_track(layer_id, "move") else {
            return WorldPoint::origin();
        };
        crate::interp_move::resolve_move_offset(&track.blocks, breath)
            .map(|offset| WorldPoint {
                x: offset[0],
                y: offset[1],
                z: offset[2],
            })
            .unwrap_or_else(WorldPoint::origin)
    }

    /// Adds `delta` to the move track at `breath` on `layer_id`, creating the
    /// move track when missing. This is the canvas drag commit seam, so it
    /// authors KEYFRAMES the way J drags (J 2026-09-07):
    ///
    /// - drag at a solid keyframe's own start breath → the delta accumulates on
    ///   that keyframe (repeated drags at one breath stay one keyframe);
    /// - drag strictly inside a solid → the drag authors a NEW keyframe at the
    ///   drag breath (old offset + delta), the covering bar keeps the region
    ///   before the split, and an interpolating empty opens between the two
    ///   bars so consecutive drags at different breaths produce visible motion
    ///   instead of a position-to-position clip;
    /// - drag inside an empty → the empty's left part stays empty (the
    ///   interpolation region survives) and a new keyframe lands at the drag
    ///   breath carrying the resolved offset + delta.
    ///
    /// With no block covering the breath at all, a keyframe spanning the
    /// layer's own timing window is created. Returns whether the document
    /// changed.
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
        let covered = track
            .blocks
            .iter()
            .position(|block| breath_in_span(breath, block.start_breath, block.length_breaths));
        // The offset already resolved at `breath` before this drag: a solid
        // holds its value across its whole span; an empty resolves through the
        // move interp (hold / interpolate / loop).
        let (covered, current) = match covered {
            Some(index) => {
                let block = &track.blocks[index];
                let current = if block.is_blank {
                    crate::interp_move::resolve_move_offset(&track.blocks, breath)
                        .unwrap_or([0, 0, 0])
                } else {
                    block
                        .value
                        .as_ref()
                        .and_then(parse_move_offset)
                        .unwrap_or([0, 0, 0])
                };
                (Some(index), current)
            }
            None => (None, [0, 0, 0]),
        };
        let next_value = serde_json::json!({
            "x": current[0] + delta.x,
            "y": current[1] + delta.y,
            "z": current[2] + delta.z,
        });
        match covered {
            Some(index) => {
                let block_start = track.blocks[index].start_breath;
                let block_length = track.blocks[index].length_breaths;
                let block_is_blank = track.blocks[index].is_blank;
                let strictly_inside =
                    breath > block_start && breath < block_start + block_length.max(1);
                let carries_value = track.blocks[index]
                    .value
                    .as_ref()
                    .and_then(parse_move_offset)
                    .is_some();
                // Split cases: a drag strictly inside a real keyframe, or
                // strictly inside an empty. A valueless solid (the born-tiled
                // placeholder) and a drag at a block's own start breath edit in
                // place instead.
                let splits = if block_is_blank {
                    strictly_inside
                } else {
                    carries_value && strictly_inside
                };
                if !splits {
                    // Drag at the keyframe's own start breath: edit in place.
                    let block = &mut track.blocks[index];
                    block.value = Some(next_value);
                    block.is_blank = false;
                } else {
                    // Keyframe split at the drag breath. A valueless solid (the
                    // born-tiled placeholder) just takes the value in place. For a
                    // solid keyframe the empty is carved out of the left keyframe's
                    // right end (keeping at least one breath of it) so the retile
                    // below fills the gap with a default interpolating empty; for
                    // an empty covering block the left part simply stays empty.
                    if block_is_blank {
                        track.blocks[index].length_breaths = breath - block_start;
                    } else {
                        let left_length = breath - block_start;
                        let empty_length = if left_length >= 2 {
                            (left_length / 2).max(1).min(left_length - 1)
                        } else {
                            0
                        };
                        track.blocks[index].length_breaths = left_length - empty_length;
                    }
                    track.blocks.insert(
                        index + 1,
                        SharedDocumentPropertyBlock {
                            id: next_property_block_id(&track.blocks),
                            start_breath: breath,
                            length_breaths: block_start + block_length - breath,
                            is_blank: false,
                            value: Some(next_value),
                            interpretation: None,
                            ease_out_percent: None,
                            ease_in_percent: None,
                        },
                    );
                }
            }
            None => {
                // A breath past the stored extent maps onto the trailing blank
                // (the infinite region resolves AS it) — so the drag lands a
                // one-breath keyframe AT the drag breath carrying the resolved
                // offset + delta. Pushing a span solid here would overlap the
                // whole track and the retile would vaporize later keyframes.
                let past_extent = track
                    .blocks
                    .last()
                    .is_some_and(|last| last.is_blank && breath >= last.start_breath);
                if past_extent {
                    let resolved = crate::interp_move::resolve_move_offset(&track.blocks, breath)
                        .unwrap_or([0, 0, 0]);
                    track.blocks.push(SharedDocumentPropertyBlock {
                        id: next_property_block_id(&track.blocks),
                        start_breath: breath,
                        length_breaths: 1,
                        is_blank: false,
                        value: Some(serde_json::json!({
                            "x": resolved[0] + delta.x,
                            "y": resolved[1] + delta.y,
                            "z": resolved[2] + delta.z,
                        })),
                        interpretation: None,
                        ease_out_percent: None,
                        ease_in_percent: None,
                    });
                } else {
                    track.blocks.push(SharedDocumentPropertyBlock {
                        id: next_property_block_id(&track.blocks),
                        start_breath,
                        length_breaths: length_breaths.max(1),
                        is_blank: false,
                        value: Some(next_value),
                        interpretation: None,
                        ease_out_percent: None,
                        ease_in_percent: None,
                    });
                }
            }
        }
        // Re-tile like every other mutating seam: painting into a blank can
        // consume the trailing blank (or strand a loop mode's edge), and the
        // normalization runs here too (J 2026-09-07).
        track.blocks = retiled_property_track(
            std::mem::take(&mut track.blocks),
            start_breath,
            length_breaths,
        );
        self.ensure_block_canvas_coverage(layer_id);
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
            let track =
                default_property_track(property_id, layer.start_breath, layer.length_breaths);
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
            property_tracks: vec![
                default_property_track("raster", 0, default_layer_length_breaths()),
                default_property_track("move", 0, default_layer_length_breaths()),
            ],
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
            track.blocks =
                retiled_property_track(std::mem::take(&mut track.blocks), span_start, span_length);
        }
        drop(layer);
        self.ensure_block_canvas_coverage(layer_id);
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
        track.blocks =
            retiled_property_track(std::mem::take(&mut track.blocks), span_start, span_length);
        self.ensure_block_canvas_coverage(layer_id);
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
        // Partially overlapped neighbors are CROPPED to their remainder, keeping
        // their content (a destructive drag is a timing adjustment — J 2026-09-07).
        // Only fully encapsulated neighbors die, and that is the only content loss.
        // A neighbor the edit straddles crops on BOTH sides: the first remainder
        // stays in place, the second splits off as a new block (same is_blank/value,
        // fresh id, own empty canvas — per-block canvases cannot be split).
        let mut dropped_canvas_keys = Vec::new();
        let mut new_canvas_keys: Vec<(String, String)> = Vec::new();
        let mut trimmed_once: Vec<usize> = Vec::new();
        for (other_index, (start, length)) in destructive.trimmed {
            let track_index = other_track_indices[other_index];
            if trimmed_once.contains(&track_index) {
                let victim = &track.blocks[track_index];
                let split_id = next_property_block_id(&track.blocks);
                let split = SharedDocumentPropertyBlock {
                    id: split_id.clone(),
                    start_breath: start,
                    length_breaths: length,
                    is_blank: victim.is_blank,
                    value: victim.value.clone(),
                    interpretation: None,
                    ease_out_percent: None,
                    ease_in_percent: None,
                };
                let insert_at = track
                    .blocks
                    .iter()
                    .position(|block| block.start_breath > start)
                    .unwrap_or(track.blocks.len());
                track.blocks.insert(insert_at, split);
                new_canvas_keys.push((layer_id.to_string(), split_id));
                continue;
            }
            trimmed_once.push(track_index);
            let victim = &mut track.blocks[track_index];
            victim.start_breath = start;
            victim.length_breaths = length;
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
        for key in new_canvas_keys {
            self.block_canvases.insert(key, Canvas::new());
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
            track.blocks =
                retiled_property_track(std::mem::take(&mut track.blocks), span_start, span_length);
        }
        self.ensure_block_canvas_coverage(layer_id);
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
                interpretation: None,
                ease_out_percent: None,
                ease_in_percent: None,
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
        track.blocks =
            retiled_property_track(std::mem::take(&mut track.blocks), span_start, span_length);
        // The block's painted content is discarded: a stale canvas would keep
        // rendering through the raw edit-surface seam (which reads by block id,
        // not by is_blank) as ghost content over a blank span.
        self.block_canvases
            .insert((layer_id.to_string(), block_id.to_string()), Canvas::new());
        self.ensure_block_canvas_coverage(layer_id);
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
            right
                .filter(|&i| !track.blocks[i].is_blank)
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
        // Re-tile like every other mutating seam: merging the trailing blank
        // into a content block must still leave a trailing blank representative
        // (and any stranded edge-locked mode must normalize away).
        let (start_breath, length_breaths) = {
            let Some(layer) = self
                .document
                .layers
                .iter()
                .find(|layer| layer.layer_id == layer_id)
            else {
                return true;
            };
            (layer.start_breath, layer.length_breaths)
        };
        let track = self
            .ensure_property_track_mut(layer_id, property_id)
            .expect("track existed above");
        track.blocks = retiled_property_track(
            std::mem::take(&mut track.blocks),
            start_breath,
            length_breaths,
        );
        self.ensure_block_canvas_coverage(layer_id);
        true
    }

    /// Cycles an empty (blank) block's interpolation mode through the
    /// `interp_mode::INTERP_MODES` order (J 2026-09-07). The loop modes are
    /// edge-locked: `loop_out` is only reachable on the track's last blank (the
    /// trailing blank — the right edge of time) and `loop_in` only on the first
    /// blank (the leading blank — the left edge); the cycle SKIPS a locked mode
    /// the empty is not allowed to carry rather than rejecting, so a middle
    /// empty just toggles interpolate ↔ hold. Cycling onto a mode that does not
    /// use the ease ends clears both stored ease strengths — the ends are not
    /// utilizable there. Returns `false` when the block is missing or not blank.
    pub fn cycle_property_block_interp_mode(
        &mut self,
        layer_id: &str,
        property_id: &str,
        block_id: &str,
    ) -> bool {
        let Some(track) = self.ensure_property_track_mut(layer_id, property_id) else {
            return false;
        };
        let Some(index) = track.blocks.iter().position(|block| block.id == block_id) else {
            return false;
        };
        let block = &track.blocks[index];
        if !block.is_blank {
            return false;
        }
        let is_first = index == 0;
        let is_last = index + 1 == track.blocks.len();
        let mut next = interp_mode::next_mode(block.interpretation.as_deref());
        for _ in 0..interp_mode::INTERP_MODES.len() {
            if interp_mode::mode_allowed_at(next, is_first, is_last) {
                break;
            }
            next = interp_mode::next_mode(Some(next));
        }
        let block = &mut track.blocks[index];
        block.interpretation = Some(next.to_string());
        if !interp_mode::mode_is_ease_adjustable(next) {
            block.ease_out_percent = None;
            block.ease_in_percent = None;
        }
        true
    }

    /// Cycles an empty block's ease-out strength (left end) through the
    /// `interp_mode::EASE_STRENGTHS` steps. Rejected when the block is missing,
    /// not blank, or its mode does not use the ease ends. Returns `false` on any
    /// rejection.
    pub fn cycle_property_block_ease_out(
        &mut self,
        layer_id: &str,
        property_id: &str,
        block_id: &str,
    ) -> bool {
        self.cycle_property_block_ease(layer_id, property_id, block_id, EaseEnd::Out)
    }

    /// Cycles an empty block's ease-in strength (right end) — the ease-out mirror.
    pub fn cycle_property_block_ease_in(
        &mut self,
        layer_id: &str,
        property_id: &str,
        block_id: &str,
    ) -> bool {
        self.cycle_property_block_ease(layer_id, property_id, block_id, EaseEnd::In)
    }

    fn cycle_property_block_ease(
        &mut self,
        layer_id: &str,
        property_id: &str,
        block_id: &str,
        end: EaseEnd,
    ) -> bool {
        let Some(track) = self.ensure_property_track_mut(layer_id, property_id) else {
            return false;
        };
        let Some(block) = track.blocks.iter_mut().find(|block| block.id == block_id) else {
            return false;
        };
        if !block.is_blank {
            return false;
        }
        let mode = interp_mode::resolve_mode(block.interpretation.as_deref());
        if !interp_mode::mode_is_ease_adjustable(mode) {
            return false;
        }
        let next = interp_mode::next_ease_percent(match end {
            EaseEnd::Out => block.ease_out_percent,
            EaseEnd::In => block.ease_in_percent,
        });
        match end {
            EaseEnd::Out => block.ease_out_percent = Some(next),
            EaseEnd::In => block.ease_in_percent = Some(next),
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
        let (span_start, span_length) = self
            .document
            .layers
            .iter()
            .find(|layer| layer.layer_id == layer_id)
            .map(|layer| (layer.start_breath, layer.length_breaths))
            .unwrap_or((0, 1));
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
        // Re-tile aggressively: a swap can butt two empties together, and the
        // no-adjacent-empties invariant must hold after EVERY mutation (J
        // 2026-09-07). It also restores the trailing blank representative.
        track.blocks =
            retiled_property_track(std::mem::take(&mut track.blocks), span_start, span_length);
        self.ensure_block_canvas_coverage(layer_id);
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
        let source = {
            let track = self.property_track(layer_id, property_id)?;
            let index = track.blocks.iter().position(|block| block.id == block_id)?;
            track.blocks[index].clone()
        };
        let new_start = source.start_breath + source.length_breaths;
        // The viewport (layer span) is never coupled to editing (J 2026-09-07): a
        // duplicate landing past the span end just lands there — the trailing blank
        // and the infinite empty region absorb the room. No span growth.
        let Some(track) = self.ensure_property_track_mut(layer_id, property_id) else {
            return None;
        };
        let source_index = track.blocks.iter().position(|block| block.id == block_id)?;
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
                interpretation: None,
                ease_out_percent: None,
                ease_in_percent: None,
            },
        );
        self.block_canvases
            .insert((layer_id.to_string(), new_id.clone()), Canvas::new());
        let (span_start, span_length) = self
            .document
            .layers
            .iter()
            .find(|layer| layer.layer_id == layer_id)
            .map(|layer| (layer.start_breath, layer.length_breaths))
            .unwrap_or((0, 1));
        // The push may leave the track's tail blank stranded (shifted right, now
        // starting at the duplicate's old spot) — one re-tile re-opens the gap and
        // restores the trailing-blank representative.
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
            track.blocks =
                retiled_property_track(std::mem::take(&mut track.blocks), span_start, span_length);
        }
        self.ensure_block_canvas_coverage(layer_id);
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

    /// Flattens every visible layer's RESOLVED canvas for the breath into one canvas, in document
    /// layer order (later layers win on overlap). Interpolating raster empties resolve their
    /// blend (see `resolved_canvas_for_layer`), not the raw block canvas.
    pub fn composited_canvas_in_layer_order(
        &self,
        current_breath: u32,
        graphic_fade: Option<&thaum_renderer_domain::shape_fade::fade::ShapeFade>,
    ) -> Canvas {
        let mut canvas = Canvas::new();
        for layer in self.document.layers.iter().filter(|layer| layer.visible) {
            if let Some(layer_canvas) =
                self.resolved_canvas_for_layer(&layer.layer_id, current_breath, graphic_fade)
            {
                for (position, painted_cell) in &layer_canvas {
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
        let (span_start, span_length) = self
            .document
            .layers
            .iter()
            .find(|layer| layer.layer_id == layer_id)
            .map(|layer| (layer.start_breath, layer.length_breaths))
            .unwrap_or((0, 1));
        if let Some(track) = self
            .document
            .layers
            .iter_mut()
            .find(|layer| layer.layer_id == layer_id)
            .and_then(|layer| {
                layer
                    .property_tracks
                    .iter_mut()
                    .find(|track| track.property_id == "raster")
            })
        {
            if let Some(block) = track.blocks.iter_mut().find(|block| block.id == block_id) {
                block.is_blank = false;
            }
            // Unblanking can consume the trailing blank (or strand a loop
            // mode's edge) — the same re-tile the live paint path runs via
            // `add_move_offset`, here on the replay/apply path (J 2026-09-07:
            // the fuzzer caught painting into a past-extent blank leaving a
            // track with no trailing blank representative).
            track.blocks =
                retiled_property_track(std::mem::take(&mut track.blocks), span_start, span_length);
        }
        self.ensure_block_canvas_coverage(layer_id);
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
                    write!(
                        f,
                        "file carries no schema version; this app reads v{supported}"
                    )
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
                    property_tracks: vec![
                        default_property_track("raster", 0, default_layer_length_breaths()),
                        default_property_track("move", 0, default_layer_length_breaths()),
                    ],
                },
                SharedDocumentLayer {
                    layer_id: "layer-2".to_string(),
                    name: "Layer 2".to_string(),
                    visible: true,
                    locked: false,
                    start_breath: 0,
                    length_breaths: default_layer_length_breaths(),
                    property_tracks: vec![
                        default_property_track("raster", 0, default_layer_length_breaths()),
                        default_property_track("move", 0, default_layer_length_breaths()),
                    ],
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

        let composited = runtime.composited_canvas_in_layer_order(0, None);

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
        assert!(error.to_string().contains(&format!(
            "file is schema v{}, this app reads v{}",
            SHARED_DOCUMENT_SCHEMA_VERSION + 1,
            SHARED_DOCUMENT_SCHEMA_VERSION
        )));

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
        assert!(runtime.composited_canvas_in_layer_order(0, None).is_empty());

        assert!(runtime.set_layer_visible("layer-1", true));
        assert_eq!(
            runtime
                .composited_canvas_in_layer_order(0, None)
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

        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0].id, "block-1");
        assert_eq!(blocks[0].start_breath, 0);
        assert_eq!(blocks[0].length_breaths, 8);
        assert_eq!(blocks[1].start_breath, 8);
        assert_eq!(blocks[1].length_breaths, 16);
        assert_eq!(blocks[2].id, "tail");
        assert!(blocks[2].is_blank);
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
        // would leave the channel with two kinds of no-content. The pushed ripple
        // pulls the old tail blank left; the re-tile re-opens the infinite tail.
        assert!(runtime.set_property_block_timing_pushed("layer-1", "raster", "block-1", 0, 8));
        let track = runtime.property_track("layer-1", "raster").unwrap();
        let covered: Vec<(u32, u32, bool)> = track
            .blocks
            .iter()
            .map(|b| {
                (
                    b.start_breath,
                    b.start_breath + b.length_breaths,
                    b.is_blank,
                )
            })
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
        assert!(runtime.set_property_block_timing_pushed("layer-1", "raster", "block-2", 2, 16));
        let track = runtime.property_track("layer-1", "raster").unwrap();
        let covered: Vec<(u32, u32, bool)> = track
            .blocks
            .iter()
            .map(|b| {
                (
                    b.start_breath,
                    b.start_breath + b.length_breaths,
                    b.is_blank,
                )
            })
            .collect();
        assert_eq!(covered, vec![(0, 8, false), (8, 18, false), (18, 24, true)]);
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
        // The merged empty absorbs the trailing blank representative (infinite).
        assert_eq!(
            (track.blocks[0].start_breath, track.blocks[0].length_breaths),
            (0, 25)
        );
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
        assert!(runtime
            .split_property_block("layer-1", "raster", "block-2", 16)
            .is_some());

        // Track: empty (0..8), content (8..16), content (16..24). Merging the empty
        // prefers the left... there is no content on the left, so it falls back to
        // the right: content (8..16) expands left over the empty. The re-tile then
        // appends the trailing blank representative past the last content block.
        assert!(runtime.merge_empty_property_block("layer-1", "raster", "block-1", true));
        let track = runtime.property_track("layer-1", "raster").unwrap();
        let covered: Vec<(u32, u32, bool)> = track
            .blocks
            .iter()
            .map(|b| {
                (
                    b.start_breath,
                    b.start_breath + b.length_breaths,
                    b.is_blank,
                )
            })
            .collect();
        assert_eq!(
            covered,
            vec![(0, 16, false), (16, 24, false), (24, 25, true)]
        );
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
        // The re-tile then appends the trailing blank representative.
        assert!(runtime.blank_property_block("layer-1", "raster", "block-2"));
        assert!(runtime.merge_empty_property_block("layer-1", "raster", "block-2", true));
        let track = runtime.property_track("layer-1", "raster").unwrap();
        let covered: Vec<(u32, u32, bool)> = track
            .blocks
            .iter()
            .map(|b| {
                (
                    b.start_breath,
                    b.start_breath + b.length_breaths,
                    b.is_blank,
                )
            })
            .collect();
        assert_eq!(
            covered,
            vec![(0, 16, false), (16, 24, false), (24, 25, true)]
        );
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
            .map(|b| {
                (
                    b.start_breath,
                    b.start_breath + b.length_breaths,
                    b.is_blank,
                )
            })
            .collect();
        assert_eq!(
            covered,
            vec![
                (0, 8, false),
                (8, 16, false),
                (16, 32, false),
                (32, 33, true)
            ]
        );
        assert_ne!(new_id, "block-1");
    }

    #[test]
    fn duplicate_past_the_viewport_end_leaves_the_layer_span_alone() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));

        // block-1 (0..24) ends at the viewport end: the duplicate lands at (24..48)
        // and the layer span does NOT grow — the viewport is never coupled to editing
        // (J 2026-09-07). The infinite trailing blank absorbs the room.
        let _new_id = runtime
            .duplicate_property_block("layer-1", "raster", "block-1")
            .expect("duplicate");
        let layer = runtime
            .layers()
            .iter()
            .find(|layer| layer.layer_id == "layer-1")
            .unwrap();
        assert_eq!((layer.start_breath, layer.length_breaths), (0, 24));
        let track = runtime.property_track("layer-1", "raster").unwrap();
        let covered: Vec<(u32, u32, bool)> = track
            .blocks
            .iter()
            .map(|b| {
                (
                    b.start_breath,
                    b.start_breath + b.length_breaths,
                    b.is_blank,
                )
            })
            .collect();
        assert_eq!(
            covered,
            vec![(0, 24, false), (24, 48, false), (48, 49, true)]
        );
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
        assert_eq!(ids.len(), 5);
        assert_eq!(
            &ids[..4],
            &["block-1", "block-2", new_id.as_str(), "block-3"]
        );
        assert_eq!(track.blocks[0].start_breath, 0);
        assert_eq!(track.blocks[1].start_breath, 8);
        assert_eq!(track.blocks[2].start_breath, 16);
        assert_eq!(track.blocks[3].start_breath, 24);
        // The trailing blank representative: fresh id, blank, right after the tail.
        assert!(track.blocks[4].is_blank);
        assert_ne!(track.blocks[4].id, track.blocks[3].id);
    }

    #[test]
    fn split_rejects_blank_blocks() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));
        assert!(runtime.blank_property_block("layer-1", "raster", "block-1"));
        assert!(runtime
            .split_property_block("layer-1", "raster", "block-1", 8)
            .is_none());
    }

    #[test]
    fn destructive_timing_partial_overlap_crops_the_victim_keeping_its_content() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));
        runtime.split_property_block("layer-1", "raster", "block-1", 8);
        runtime.set_property_block_timing_destructive("layer-1", "raster", "block-2", 16, 8);

        // block-1 (0..8, empty) slides destructively into the middle of block-2
        // (16..24): the victim is CROPPED to its un-covered remainder (8..18) and
        // KEEPS its content — a destructive drag is a timing adjustment, not a
        // deletion (J 2026-09-07). The vacated window re-tiles into a blank and
        // the trailing blank representative is appended after the cropped solid.
        assert!(
            runtime.set_property_block_timing_destructive("layer-1", "raster", "block-1", 18, 4)
        );
        let track = runtime.property_track("layer-1", "raster").unwrap();
        let covered: Vec<(u32, u32, bool)> = track
            .blocks
            .iter()
            .map(|b| {
                (
                    b.start_breath,
                    b.start_breath + b.length_breaths,
                    b.is_blank,
                )
            })
            .collect();
        assert_eq!(
            covered,
            vec![
                (0, 16, true),
                (16, 18, false),
                (18, 22, false),
                (22, 24, false),
                (24, 25, true)
            ]
        );
    }

    #[test]
    fn destructive_timing_fully_encapsulated_victim_is_the_only_content_deletion() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));
        runtime.split_property_block("layer-1", "raster", "block-1", 8);
        runtime.split_property_block("layer-1", "raster", "block-2", 16);

        // block-2 (8..16) slides destructively to 4..20: block-1 (0..8) is cropped
        // to 0..4 and block-3 (16..24) is cropped to 20..24 — both keep their
        // content. Only a victim fully encapsulated by the moved bar would be
        // deleted outright; partial overlap never deletes (J 2026-09-07).
        assert!(
            runtime.set_property_block_timing_destructive("layer-1", "raster", "block-2", 4, 16)
        );
        let track = runtime.property_track("layer-1", "raster").unwrap();
        let covered: Vec<(u32, u32, bool)> = track
            .blocks
            .iter()
            .map(|b| {
                (
                    b.start_breath,
                    b.start_breath + b.length_breaths,
                    b.is_blank,
                )
            })
            .collect();
        assert_eq!(
            covered,
            vec![
                (0, 4, false),
                (4, 20, false),
                (20, 24, false),
                (24, 25, true)
            ]
        );
    }

    #[test]
    fn destructive_timing_shrink_leaves_the_vacated_range_blank() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));

        // block-1 (0..24) shrinks to 0..8: breaths 8..24 re-tile into blanks and
        // merge with the trailing blank representative.
        assert!(runtime.set_property_block_timing_destructive("layer-1", "raster", "block-1", 0, 8));
        let track = runtime.property_track("layer-1", "raster").unwrap();
        let covered: Vec<(u32, u32, bool)> = track
            .blocks
            .iter()
            .map(|b| {
                (
                    b.start_breath,
                    b.start_breath + b.length_breaths,
                    b.is_blank,
                )
            })
            .collect();
        assert_eq!(covered, vec![(0, 8, false), (8, 25, true)]);
    }

    #[test]
    fn destructive_timing_removes_fully_covered_neighbors() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));
        runtime.split_property_block("layer-1", "raster", "block-1", 8);

        // block-1 (0..8) grows destructively to 0..24: block-2 (8..24) is fully
        // covered, so it is removed outright — the edited block covers its range.
        // The trailing blank representative is appended after the solid tail.
        assert!(
            runtime.set_property_block_timing_destructive("layer-1", "raster", "block-1", 0, 24)
        );
        let blocks = &runtime.property_track("layer-1", "raster").unwrap().blocks;
        assert_eq!(blocks.len(), 2);
        assert_eq!((blocks[0].start_breath, blocks[0].length_breaths), (0, 24));
        assert!(!blocks[0].is_blank);
        assert!(blocks[1].is_blank);
    }

    #[test]
    fn destructive_timing_gap_blanks_never_reuse_a_later_blocks_id() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));
        runtime.split_property_block("layer-1", "raster", "block-1", 8);
        runtime.split_property_block("layer-1", "raster", "block-2", 16);

        // block-1 (0..8) jumps destructively to 8..16, eating block-2's front half:
        // the vacated 0..8 becomes a gap blank, and that blank must not reuse "block-1"
        // or "block-2" — duplicate ids made highlight and id-keyed edits hit two bars.
        assert!(runtime.set_property_block_timing_destructive("layer-1", "raster", "block-1", 8, 8));
        let blocks = &runtime.property_track("layer-1", "raster").unwrap().blocks;
        let mut ids: Vec<&str> = blocks.iter().map(|b| b.id.as_str()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), blocks.len(), "duplicate block ids: {ids:?}");
    }

    #[test]
    fn set_layer_timing_resizes_the_viewport_without_clipping_tracks() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));
        runtime.split_property_block("layer-1", "raster", "block-1", 8);

        // The viewport (layer span) is never coupled to track data (J 2026-09-07):
        // growing it just extends the trailing blank representative.
        assert!(runtime.set_layer_timing("layer-1", 0, 32));
        let track = runtime.property_track("layer-1", "raster").unwrap();
        let covered: Vec<(u32, u32, bool)> = track
            .blocks
            .iter()
            .map(|b| {
                (
                    b.start_breath,
                    b.start_breath + b.length_breaths,
                    b.is_blank,
                )
            })
            .collect();
        assert_eq!(covered, vec![(0, 8, false), (8, 24, false), (24, 32, true)]);

        // Shrinking it never clips content — blocks past the viewport survive, and
        // the trailing blank's stored extent just stays (it is infinite either way).
        assert!(runtime.set_layer_timing("layer-1", 0, 12));
        let track = runtime.property_track("layer-1", "raster").unwrap();
        let covered: Vec<(u32, u32, bool)> = track
            .blocks
            .iter()
            .map(|b| {
                (
                    b.start_breath,
                    b.start_breath + b.length_breaths,
                    b.is_blank,
                )
            })
            .collect();
        assert_eq!(covered, vec![(0, 8, false), (8, 24, false), (24, 32, true)]);
    }

    #[test]
    fn swap_property_blocks_exchanges_their_breath_ranges() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));
        runtime.split_property_block("layer-1", "raster", "block-1", 8);

        assert!(runtime.swap_property_blocks("layer-1", "raster", "block-1", "block-2"));
        let blocks = &runtime.property_track("layer-1", "raster").unwrap().blocks;
        // Re-tile sorts by start: block-2's swapped span lands first.
        assert_eq!(blocks[0].start_breath, 0);
        assert_eq!(blocks[0].length_breaths, 8);
        assert_eq!(blocks[1].start_breath, 8);
        assert_eq!(blocks[1].length_breaths, 16);

        assert!(!runtime.swap_property_blocks("layer-1", "raster", "block-1", "block-1"));
        assert!(!runtime.swap_property_blocks("layer-1", "raster", "block-1", "missing"));
    }

    #[test]
    fn swap_butting_two_empties_together_merges_them() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));
        runtime.split_property_block("layer-1", "raster", "block-1", 8);
        runtime.split_property_block("layer-1", "raster", "block-2", 16);
        // Track: block-1 (0..8), block-2 (8..16), block-3 (16..24), tail (24..25).
        assert!(runtime.blank_property_block("layer-1", "raster", "block-1"));

        // Track: empty (0..8), content (8..16), content (16..24), tail. Swapping
        // the empty with the far content bar lands the empty right next to the
        // tail — the aggressive re-tile must merge them (no-adjacent-empties,
        // J 2026-09-07).
        assert!(runtime.swap_property_blocks("layer-1", "raster", "block-1", "block-3"));
        let track = runtime.property_track("layer-1", "raster").unwrap();
        let covered: Vec<(u32, u32, bool)> = track
            .blocks
            .iter()
            .map(|b| {
                (
                    b.start_breath,
                    b.start_breath + b.length_breaths,
                    b.is_blank,
                )
            })
            .collect();
        assert_eq!(covered, vec![(0, 8, false), (8, 16, false), (16, 25, true)]);
        let blanks: Vec<_> = track.blocks.iter().filter(|b| b.is_blank).collect();
        assert_eq!(blanks.len(), 1, "adjacent empties must merge into one");
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
    fn painting_into_a_past_extent_blank_keeps_the_trailing_blank_representative() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));
        // Push the sole block far past the window: gap blank 0..50, solid
        // 50..56, minimal trailing blank 56..57.
        assert!(
            runtime.set_property_block_timing_destructive("layer-1", "raster", "block-1", 50, 6,)
        );
        let tail_id = runtime
            .property_track("layer-1", "raster")
            .unwrap()
            .blocks
            .last()
            .unwrap()
            .id
            .clone();

        // Paint into that past-extent trailing blank (the replay/apply path):
        // unblanking must not consume the trailing-blank representative —
        // before the fix the track ended solid and past-extent playback
        // resolved nothing (found by the invariant fuzzer, J 2026-09-07).
        runtime.apply_action_record(SharedDocumentActionRecord::cell_patch_set(
            "a-past-extent-paint",
            "doc-1",
            "layer-1",
            "u1",
            "1",
            vec![SharedCellPatch::new(point(1, 1), None, Some(&cell('#')))],
            Some(tail_id),
        ));
        let blocks = &runtime.property_track("layer-1", "raster").unwrap().blocks;
        assert!(
            blocks.last().unwrap().is_blank,
            "the track still ends with a blank representative"
        );
        // The painted keyframe keeps its content.
        assert_eq!(
            runtime
                .resolved_canvas_for_layer("layer-1", 56, None)
                .unwrap()
                .len(),
            1
        );
        // Far past the new edge, resolution stays sane (nothing or held, never
        // a panic or a vaporized track).
        let _ = runtime.resolved_canvas_for_layer("layer-1", 10_000, None);
    }

    #[test]
    fn an_interpolating_raster_empty_resolves_a_blend_between_its_keyframes() {
        let document = SharedDocumentFile::single_layer("doc-1", "Doc", "layer-1", "Layer 1");
        let mut runtime = SharedDocumentRuntime::new(document);
        runtime.split_property_block("layer-1", "raster", "block-1", 8);
        runtime.split_property_block("layer-1", "raster", "block-2", 16);
        // Move block-2 (8..16) onto block-3 (16..24): full encapsulation deletes
        // the victim, the vacated 8..16 becomes a middle empty (default mode:
        // interpolate), and re-tiling keeps a trailing blank representative.
        assert!(
            runtime.set_property_block_timing_destructive("layer-1", "raster", "block-2", 16, 8,)
        );

        // Keyframe A (0..8): 'A' red, weight 0. Keyframe B (16..24): 'B' blue,
        // weight 8. The middle empty blends between them.
        let keyframe_a = PaintedCell {
            graphic: CellGraphic::Glyph('A'),
            color: PaintColor::flat_rgb(255, 0, 0),
            weight_index: 0,
        };
        let keyframe_b = PaintedCell {
            graphic: CellGraphic::Glyph('B'),
            color: PaintColor::flat_rgb(0, 0, 255),
            weight_index: 8,
        };
        let solid_a_id = runtime
            .property_track("layer-1", "raster")
            .unwrap()
            .blocks
            .iter()
            .find(|b| !b.is_blank && b.start_breath == 0)
            .map(|b| b.id.clone())
            .unwrap();
        let solid_b_id = runtime
            .property_track("layer-1", "raster")
            .unwrap()
            .blocks
            .iter()
            .find(|b| !b.is_blank && b.start_breath == 16)
            .map(|b| b.id.clone())
            .unwrap();
        for (block_id, painted) in [(&solid_a_id, &keyframe_a), (&solid_b_id, &keyframe_b)] {
            runtime.apply_action_record(SharedDocumentActionRecord::cell_patch_set(
                "a-blend",
                "doc-1",
                "layer-1",
                "u1",
                "1",
                vec![SharedCellPatch::new(point(0, 0), None, Some(painted))],
                Some(block_id.clone()),
            ));
        }

        // Solids resolve their own canvases exactly.
        assert_eq!(
            runtime
                .resolved_canvas_for_layer("layer-1", 0, None)
                .unwrap()
                .get(&point(0, 0)),
            Some(&keyframe_a)
        );
        assert_eq!(
            runtime
                .resolved_canvas_for_layer("layer-1", 16, None)
                .unwrap()
                .get(&point(0, 0)),
            Some(&keyframe_b)
        );
        // Breath 12 is the middle empty's halfway crossing plus one: t = 5/9
        // (~0.556, second half) — graphic from keyframe B, color and weight
        // blended 5/9 of the way from A to B.
        let blended = runtime
            .resolved_canvas_for_layer("layer-1", 12, None)
            .unwrap()
            .get(&point(0, 0))
            .unwrap()
            .clone();
        assert_eq!(blended.graphic, CellGraphic::Glyph('B'));
        assert_eq!(
            blended.color,
            PaintColor::flat_rgb(
                (255.0_f32 * 4.0 / 9.0).round() as u8,
                0,
                (255.0_f32 * 5.0 / 9.0).round() as u8
            )
        );
        assert_eq!(blended.weight_index, (8.0_f32 * 5.0 / 9.0).round() as i64);
        // First-half breath (t = 1/9) still shows keyframe A's graphic with the
        // color already blending.
        let early = runtime
            .resolved_canvas_for_layer("layer-1", 9, None)
            .unwrap()
            .get(&point(0, 0))
            .unwrap()
            .clone();
        assert_eq!(early.graphic, CellGraphic::Glyph('A'));
        assert_ne!(
            early.color, keyframe_a.color,
            "color blends even in the first half"
        );

        // The edit surface seam is untouched: the raw block canvas over the
        // empty is still empty, so strokes author a real keyframe there.
        assert!(runtime
            .canvas_for_layer("layer-1", 12)
            .is_none_or(|canvas| canvas.is_empty()));
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

    #[test]
    fn a_move_drag_inside_a_keyframe_authors_a_new_keyframe_with_an_interpolating_empty() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));
        // First drag authors keyframe 1 across the whole layer window.
        assert!(runtime.add_move_offset("layer-1", 2, WorldPoint { x: 2, y: 0, z: 0 }));
        // Second drag at breath 12 must author keyframe 2 — not pile onto the
        // same solid — and open a default (interpolating) empty between the
        // two bars, so the layer visibly moves instead of clipping.
        assert!(runtime.add_move_offset("layer-1", 12, WorldPoint { x: 10, y: 0, z: 0 }));
        let blocks: Vec<(u32, u32, bool, Option<i32>)> = runtime
            .property_track("layer-1", "move")
            .unwrap()
            .blocks
            .iter()
            .map(|b| {
                (
                    b.start_breath,
                    b.start_breath + b.length_breaths,
                    b.is_blank,
                    b.value
                        .as_ref()
                        .and_then(|v| v.get("x").and_then(|x| x.as_i64()))
                        .map(|x| x as i32),
                )
            })
            .collect();
        assert_eq!(
            blocks,
            vec![
                (0, 6, false, Some(2)),
                (6, 12, true, None),
                (12, 24, false, Some(12)),
                (24, 25, true, None)
            ],
            "track shape after two drags"
        );
        // The keyframe breaths carry their own offsets; the empty between
        // them interpolates strictly between the two values.
        assert_eq!(runtime.move_offset_for_layer("layer-1", 2).x, 2);
        assert_eq!(runtime.move_offset_for_layer("layer-1", 12).x, 12);
        let mid = runtime.move_offset_for_layer("layer-1", 8).x;
        assert!(
            mid > 2 && mid < 12,
            "mid-empty breath must interpolate ({mid})"
        );
    }

    #[test]
    fn a_move_drag_inside_an_empty_keeps_the_interpolation_region_and_lands_a_keyframe() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));
        assert!(runtime.add_move_offset("layer-1", 2, WorldPoint { x: 2, y: 0, z: 0 }));
        assert!(runtime.add_move_offset("layer-1", 12, WorldPoint { x: 10, y: 0, z: 0 }));
        // Dragging mid-transition must not swallow the whole empty into one
        // solid: the empty's left part stays empty, the new keyframe lands at
        // the drag breath carrying the resolved offset + delta.
        assert!(runtime.add_move_offset("layer-1", 9, WorldPoint { x: 5, y: 0, z: 0 }));
        let blocks: Vec<(u32, u32, bool, Option<i32>)> = runtime
            .property_track("layer-1", "move")
            .unwrap()
            .blocks
            .iter()
            .map(|b| {
                (
                    b.start_breath,
                    b.start_breath + b.length_breaths,
                    b.is_blank,
                    b.value
                        .as_ref()
                        .and_then(|v| v.get("x").and_then(|x| x.as_i64()))
                        .map(|x| x as i32),
                )
            })
            .collect();
        assert_eq!(
            blocks,
            vec![
                (0, 6, false, Some(2)),
                (6, 9, true, None),
                (9, 12, false, Some(13)),
                (12, 24, false, Some(12)),
                (24, 25, true, None)
            ],
            "track shape after a mid-empty drag"
        );
        assert_eq!(runtime.move_offset_for_layer("layer-1", 9).x, 13);
    }

    #[test]
    fn repeated_move_drags_at_a_keyframes_own_breath_stay_one_keyframe() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));
        assert!(runtime.add_move_offset("layer-1", 2, WorldPoint { x: 2, y: 0, z: 0 }));
        // Dragging at the keyframe's own start breath accumulates in place.
        assert!(runtime.add_move_offset("layer-1", 0, WorldPoint { x: 1, y: 0, z: 0 }));
        let blocks: Vec<(u32, u32, bool, Option<i32>)> = runtime
            .property_track("layer-1", "move")
            .unwrap()
            .blocks
            .iter()
            .map(|b| {
                (
                    b.start_breath,
                    b.start_breath + b.length_breaths,
                    b.is_blank,
                    b.value
                        .as_ref()
                        .and_then(|v| v.get("x").and_then(|x| x.as_i64()))
                        .map(|x| x as i32),
                )
            })
            .collect();
        assert_eq!(blocks, vec![(0, 24, false, Some(3)), (24, 25, true, None)]);
    }

    #[test]
    fn a_move_drag_past_the_stored_extent_lands_a_keyframe_without_vaporizing_the_track() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));
        assert!(runtime.add_move_offset("layer-1", 2, WorldPoint { x: 2, y: 0, z: 0 }));
        assert!(runtime.add_move_offset("layer-1", 12, WorldPoint { x: 10, y: 0, z: 0 }));
        // Dragging out in the infinite region (breath 30, past the tail
        // blank's stored extent) must land a one-breath keyframe there and
        // keep every earlier keyframe — not push a span solid that the retile
        // trims over the existing bars.
        assert!(runtime.add_move_offset("layer-1", 30, WorldPoint { x: 5, y: 0, z: 0 }));
        let blocks: Vec<(u32, u32, bool, Option<i32>)> = runtime
            .property_track("layer-1", "move")
            .unwrap()
            .blocks
            .iter()
            .map(|b| {
                (
                    b.start_breath,
                    b.start_breath + b.length_breaths,
                    b.is_blank,
                    b.value
                        .as_ref()
                        .and_then(|v| v.get("x").and_then(|x| x.as_i64()))
                        .map(|x| x as i32),
                )
            })
            .collect();
        // The tail blank and the retile's gap fill merge into one blank
        // (no-adjacent-empties), and the retile appends a fresh trailing
        // representative after the new keyframe.
        assert_eq!(
            blocks,
            vec![
                (0, 6, false, Some(2)),
                (6, 12, true, None),
                (12, 24, false, Some(12)),
                (24, 30, true, None),
                (30, 31, false, Some(17)),
                (31, 32, true, None)
            ],
            "track shape after a past-extent drag"
        );
        // The new keyframe carries the offset that was playing there (the
        // x=12 keyframe holds through the trailing blank) plus the delta.
        assert_eq!(runtime.move_offset_for_layer("layer-1", 30).x, 17);
        assert_eq!(runtime.move_offset_for_layer("layer-1", 12).x, 12);
    }

    #[test]
    fn a_move_drag_inside_a_loop_out_trailing_blank_lands_a_keyframe_and_the_mode_migrates_to_the_new_edge(
    ) {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));
        assert!(runtime.add_move_offset("layer-1", 2, WorldPoint { x: 2, y: 0, z: 0 }));
        assert!(runtime.add_move_offset("layer-1", 12, WorldPoint { x: 10, y: 0, z: 0 }));
        // Loop out on the trailing blank, then drag inside the loop region:
        // the keyframe lands there (extending the authored region) and the
        // loop mode strips off the now-interior blank per the edge lock.
        let track = runtime.property_track("layer-1", "move").unwrap();
        let tail_id = track.blocks.last().unwrap().id.clone();
        // Cycle the tail blank None -> hold -> loop_out (next_mode steps past
        // the current, so two cycles land on loop_out).
        for _ in 0..2 {
            runtime.cycle_property_block_interp_mode("layer-1", "move", &tail_id);
        }
        let track = runtime.property_track("layer-1", "move").unwrap();
        assert!(runtime.add_move_offset("layer-1", 26, WorldPoint { x: 3, y: 0, z: 0 }));
        let track = runtime.property_track("layer-1", "move").unwrap();
        let last = track.blocks.last().unwrap();
        assert!(
            last.is_blank,
            "the track still ends with a blank representative"
        );
        assert_eq!(
            last.interpretation, None,
            "no stranded loop mode survives a retile"
        );
        // The new keyframe carries the value the loop was playing there
        // (breath 26 wraps to region breath 2, the x=2 keyframe) plus delta.
        assert_eq!(runtime.move_offset_for_layer("layer-1", 26).x, 5);
        // Keyframes before the drag are untouched.
        assert_eq!(runtime.move_offset_for_layer("layer-1", 2).x, 2);
        assert_eq!(runtime.move_offset_for_layer("layer-1", 12).x, 12);
    }

    // --- property-track invariant fuzzer -------------------------------------
    // The past-extent keyframe-vaporizing bug was found by hand-testing one
    // seam against the binary-tiling invariants. This fuzzer does that
    // systematically: it drives every mutating seam with deterministic
    // pseudo-random arguments and asserts, after EVERY operation, that the
    // track still holds the invariants the design truth promises (J
    // 2026-09-07): contiguous tiling, no overlaps/gaps, no adjacent empties,
    // trailing blank representative, unique ids, edge-locked loop modes.
    struct FuzzRand(u64);
    impl FuzzRand {
        fn next(&mut self) -> u64 {
            // xorshift64*
            let mut x = self.0;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            self.0 = x;
            x.wrapping_mul(0x2545F4914F6CDD1D)
        }
        fn below(&mut self, bound: u64) -> u64 {
            self.next() % bound.max(1)
        }
    }

    fn assert_track_invariants(
        runtime: &SharedDocumentRuntime,
        layer_id: &str,
        property_id: &str,
        seed: u64,
        op: usize,
    ) {
        let track = runtime
            .property_track(layer_id, property_id)
            .unwrap_or_else(|| panic!("seed {seed} op {op}: move track vanished"));
        let blocks = &track.blocks;
        assert!(!blocks.is_empty(), "seed {seed} op {op}: empty track");
        assert!(
            blocks.last().unwrap().is_blank,
            "seed {seed} op {op}: track must end with a blank representative; shape {}",
            runtime.property_track_shape(layer_id, property_id)
        );
        let mut used_ids: Vec<&str> = Vec::new();
        for (index, block) in blocks.iter().enumerate() {
            assert!(
                block.length_breaths >= 1,
                "seed {seed} op {op}: zero-length block {}",
                block.id
            );
            assert!(
                !used_ids.contains(&block.id.as_str()),
                "seed {seed} op {op}: duplicate id {}",
                block.id
            );
            used_ids.push(&block.id);
            if index > 0 {
                let previous = &blocks[index - 1];
                let previous_end = previous.start_breath + previous.length_breaths;
                assert_eq!(
                    block.start_breath,
                    previous_end,
                    "seed {seed} op {op}: gap/overlap before block {} (shape {})",
                    block.id,
                    runtime.property_track_shape(layer_id, property_id)
                );
                assert!(
                    !(previous.is_blank && block.is_blank),
                    "seed {seed} op {op}: adjacent empties {} + {}",
                    previous.id,
                    block.id
                );
            }
            match block.interpretation.as_deref() {
                Some("loop_out") => assert!(
                    index + 1 == blocks.len(),
                    "seed {seed} op {op}: loop_out stranded off the last blank"
                ),
                Some("loop_in") => assert!(
                    index == 0,
                    "seed {seed} op {op}: loop_in stranded off the first blank"
                ),
                _ => {}
            }
            // Canvas-map coverage (raster only: value channels like move live
            // entirely in block values and carry no canvases): every block must
            // have a canvas entry, and a blank's canvas must be empty — a
            // missing entry makes a solid render nothing silently (the same
            // vaporize shape the tiling invariants guard against, one map over).
            if property_id == "raster" {
                let canvas = runtime
                    .block_canvas(layer_id, &block.id)
                    .unwrap_or_else(|| {
                        panic!(
                            "seed {seed} op {op}: block {} has no canvas entry",
                            block.id
                        )
                    });
                if block.is_blank {
                    assert!(
                        canvas.is_empty(),
                        "seed {seed} op {op}: blank block {} carries content",
                        block.id
                    );
                }
            }
        }
    }

    /// Resolution sanity over one fuzzed raster track: the resolver never
    /// panics, a breath covered by a solid resolves exactly that block's
    /// canvas, and any blend result only contains positions present in at
    /// least one surrounding solid's canvas (blending invents nothing).
    fn assert_raster_resolution_sane(
        runtime: &SharedDocumentRuntime,
        seed: u64,
        op: usize,
        breaths: &[u32],
        graphic_fade: Option<&thaum_renderer_domain::shape_fade::fade::ShapeFade>,
    ) {
        let track = runtime.property_track("layer-1", "raster").unwrap();
        for breath in breaths {
            let resolved = runtime.resolved_canvas_for_layer("layer-1", *breath, graphic_fade);
            let covering = track
                .blocks
                .iter()
                .find(|block| {
                    crate::properties::breath_in_span(
                        *breath,
                        block.start_breath,
                        block.length_breaths,
                    )
                })
                .map(|block| (block.id.clone(), block.is_blank));
            match (covering, resolved) {
                (Some((id, false)), Some(resolved)) => {
                    let canvas = runtime.block_canvas("layer-1", &id).unwrap();
                    assert_eq!(
                        &resolved, canvas,
                        "seed {seed} op {op}: breath {breath} over solid {id} must resolve its own canvas"
                    );
                }
                (Some((id, false)), None) => {
                    panic!("seed {seed} op {op}: breath {breath} over solid {id} resolved nothing")
                }
                (Some((id, true)), Some(resolved)) => {
                    // A blank resolves a blend: every position must exist in a
                    // neighboring solid's canvas (blending invents nothing).
                    let index = track
                        .blocks
                        .iter()
                        .position(|block| block.id == id)
                        .unwrap();
                    let allowed: BTreeSet<CellPoint> = track.blocks[..index]
                        .iter()
                        .rev()
                        .find(|block| !block.is_blank)
                        .and_then(|block| runtime.block_canvas("layer-1", &block.id))
                        .map(|canvas| canvas.keys().copied().collect::<Vec<_>>())
                        .unwrap_or_default()
                        .into_iter()
                        .chain(
                            track.blocks[index + 1..]
                                .iter()
                                .find(|block| !block.is_blank)
                                .and_then(|block| runtime.block_canvas("layer-1", &block.id))
                                .map(|canvas| canvas.keys().copied().collect::<Vec<_>>())
                                .unwrap_or_default(),
                        )
                        .collect();
                    for position in resolved.keys() {
                        assert!(
                            allowed.contains(position),
                            "seed {seed} op {op}: blend at breath {breath} invented position {position:?}"
                        );
                    }
                }
                _ => {}
            }
        }
    }

    #[test]
    fn fuzzed_property_track_mutations_hold_the_binary_tiling_invariants() {
        for property in ["move", "raster"] {
            for seed in [0x9E3779B97F4A7C15, 0xD1B54A32D192ED03, 0x4873A2B5F1E0C6D9] {
                let mut rand = FuzzRand(seed);
                let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
                    "doc-1", "Doc", "layer-1", "Layer 1",
                ));
                let layer_window_end: u32 = 24;
                let mut self_action_counter = 0usize;
                for op in 0..300usize {
                    let track = runtime.property_track("layer-1", property).unwrap();
                    let pick = rand.below(11);
                    // Random breaths deliberately walk past the stored extent —
                    // the infinite region is where the vaporizing bug lived.
                    let breath = rand.below((layer_window_end as u64) * 2) as u32;
                    let block_id = track.blocks[rand.below(track.blocks.len() as u64) as usize]
                        .id
                        .clone();
                    let (changed, trace): (bool, String) = match pick {
                        0 => (
                            runtime.add_move_offset(
                                "layer-1",
                                breath,
                                WorldPoint {
                                    x: rand.below(7) as i32 - 3,
                                    y: 0,
                                    z: 0,
                                },
                            ),
                            format!("add_move at {breath}"),
                        ),
                        1 => (
                            runtime
                                .split_property_block("layer-1", property, &block_id, breath)
                                .is_some(),
                            format!("split {block_id} at {breath}"),
                        ),
                        2 => {
                            let new_length = 1 + rand.below(6) as u32;
                            let new_start = breath.min(layer_window_end.saturating_sub(1));
                            (
                                runtime.set_property_block_timing_destructive(
                                    "layer-1", property, &block_id, new_start, new_length,
                                ),
                                format!("destructive {block_id} -> {new_start}+{new_length}"),
                            )
                        }
                        3 => {
                            let new_length = 1 + rand.below(6) as u32;
                            let new_start = breath.min(layer_window_end.saturating_sub(1));
                            (
                                runtime.set_property_block_timing_pushed(
                                    "layer-1", property, &block_id, new_start, new_length,
                                ),
                                format!("pushed {block_id} -> {new_start}+{new_length}"),
                            )
                        }
                        4 => (
                            runtime
                                .cycle_property_block_interp_mode("layer-1", property, &block_id),
                            format!("cycle-mode {block_id}"),
                        ),
                        5 => (
                            runtime.cycle_property_block_ease_out("layer-1", property, &block_id),
                            format!("ease-out {block_id}"),
                        ),
                        6 => (
                            runtime.cycle_property_block_ease_in("layer-1", property, &block_id),
                            format!("ease-in {block_id}"),
                        ),
                        7 => (
                            runtime.merge_empty_property_block(
                                "layer-1",
                                property,
                                &block_id,
                                rand.below(2) == 0,
                            ),
                            format!("merge-empty {block_id}"),
                        ),
                        8 => (
                            runtime.blank_property_block("layer-1", property, &block_id),
                            format!("blank {block_id}"),
                        ),
                        9 => {
                            // Paint (raster only): a random cell onto the block
                            // covering the breath — the content the blends and
                            // canvas-coverage invariants actually chew on.
                            if property != "raster" {
                                (false, "paint skipped on move".to_string())
                            } else if let Some(target_block) =
                                runtime.active_raster_block_id("layer-1", breath)
                            {
                                let painted = PaintedCell {
                                    graphic: CellGraphic::Glyph(
                                        ['a', 'b', 'c', '#'][rand.below(4) as usize],
                                    ),
                                    color: PaintColor::flat_rgb(
                                        rand.below(256) as u8,
                                        rand.below(256) as u8,
                                        rand.below(256) as u8,
                                    ),
                                    weight_index: rand.below(4) as i64,
                                };
                                let point = CellPoint {
                                    x: rand.below(6) as i32,
                                    y: rand.below(6) as i32,
                                    z: 0,
                                };
                                self_action_counter += 1;
                                runtime.apply_action_record(
                                    SharedDocumentActionRecord::cell_patch_set(
                                        format!("fuzz-paint-{self_action_counter}"),
                                        "doc-1",
                                        "layer-1",
                                        "u1",
                                        "1",
                                        vec![SharedCellPatch::new(point, None, Some(&painted))],
                                        Some(target_block.clone()),
                                    ),
                                );
                                (true, format!("paint {point:?} onto {target_block}"))
                            } else {
                                (false, "paint with no covering block".to_string())
                            }
                        }
                        _ => {
                            let other_id = track.blocks
                                [rand.below(track.blocks.len() as u64) as usize]
                                .id
                                .clone();
                            (
                                runtime.swap_property_blocks(
                                    "layer-1", property, &block_id, &other_id,
                                ) || runtime
                                    .duplicate_property_block("layer-1", property, &block_id)
                                    .is_some(),
                                format!("swap/dup {block_id} <-> {other_id}"),
                            )
                        }
                    };
                    let _ = changed;
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        assert_track_invariants(&runtime, "layer-1", property, seed, op);
                        if property == "raster" {
                            assert_raster_resolution_sane(
                                &runtime,
                                seed,
                                op,
                                &[0, 1, breath, layer_window_end, layer_window_end * 2],
                                None,
                            );
                        }
                    }));
                    if result.is_err() {
                        eprintln!("failing op {op} on {property}: {trace}");
                        std::panic::resume_unwind(result.unwrap_err());
                    }
                }
            }
        }
    }

    #[test]
    fn j_flow_save_reload_interpolation_round_trips() {
        // J's flow, now automatic from add_move_offset: drag at 2 (keyframe 1),
        // drag at 12 (keyframe 2) — the interpolating empty opens by itself.
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));
        assert!(runtime.add_move_offset("layer-1", 2, WorldPoint { x: 2, y: 0, z: 0 }));
        assert!(runtime.add_move_offset("layer-1", 12, WorldPoint { x: 10, y: 0, z: 0 }));
        assert_eq!(
            runtime.property_track_shape("layer-1", "move"),
            "[0..6 6..12/e 12..24 24..25/e]"
        );

        // Save + reload through the same path boot uses.
        let dir = std::env::temp_dir().join("repro-interp");
        let _ = std::fs::remove_dir_all(&dir);
        let paths = SharedDocumentPaths {
            root: dir.clone(),
            document_file_path: dir.join("document.json"),
            actions_file_path: dir.join("actions.jsonl"),
        };
        save_shared_document_snapshot(&paths, &mut runtime).unwrap();
        let reloaded = load_or_create_shared_document(
            &paths,
            SharedDocumentFile::single_layer("doc-1", "Doc", "layer-1", "Layer 1"),
        )
        .unwrap();
        let shape2 = reloaded.property_track_shape("layer-1", "move");
        assert_eq!(shape2, "[0..6 6..12/e 12..24 24..25/e]");
        assert_eq!(reloaded.move_offset_for_layer("layer-1", 2).x, 2);
        assert_eq!(reloaded.move_offset_for_layer("layer-1", 12).x, 12);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn interp_test_layer_track(
        runtime: &SharedDocumentRuntime,
        property_id: &str,
    ) -> Vec<(String, u32, u32, bool, Option<String>)> {
        runtime
            .property_track("layer-1", property_id)
            .unwrap()
            .blocks
            .iter()
            .map(|b| {
                (
                    b.id.clone(),
                    b.start_breath,
                    b.start_breath + b.length_breaths,
                    b.is_blank,
                    b.interpretation.clone(),
                )
            })
            .collect()
    }

    #[test]
    fn cycling_a_middle_empty_skips_the_edge_locked_loop_modes() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));
        runtime.split_property_block("layer-1", "raster", "block-1", 8);
        // Track: block-1 (0..8 solid), block-2 (8..24 solid), tail (24..25 blank).
        // Make a MIDDLE empty by swapping: destructive-drag block-1 to 12..16, then
        // the vacated gap blank (0..12) is first — instead build the middle empty
        // directly: split block-1 into 0..4 / 4..8 and swap 4..8 with 8..16? Simpler:
        // split twice and swap so a blank sits between two solids.
        runtime.split_property_block("layer-1", "raster", "block-1", 4);
        // blocks: block-1 (0..4), block-3 (4..8), block-2 (8..24), tail.
        // Swap block-3 (4..8) with a middle slice of block-2 via destructive drag:
        // move block-3 to 12..16 leaves a 4..12 gap blank (middle empty).
        assert!(
            runtime.set_property_block_timing_destructive("layer-1", "raster", "block-3", 12, 4,)
        );
        let shape = interp_test_layer_track(&runtime, "raster");
        let middle_blank_id = shape
            .iter()
            .find(|(_, start, end, is_blank, _)| *is_blank && *start > 0)
            .map(|(id, ..)| id.clone())
            .expect("a middle empty should exist");
        let middle_index = shape
            .iter()
            .position(|(id, ..)| *id == middle_blank_id)
            .unwrap();
        assert!(middle_index > 0 && middle_index + 1 < shape.len());

        // The cycle skips both loop modes: interpolate -> hold -> interpolate -> ...
        for expected in ["hold", "interpolate", "hold", "interpolate"] {
            assert!(runtime.cycle_property_block_interp_mode(
                "layer-1",
                "raster",
                &middle_blank_id,
            ));
            let mode = runtime
                .property_track("layer-1", "raster")
                .unwrap()
                .blocks
                .iter()
                .find(|b| b.id == middle_blank_id)
                .unwrap()
                .interpretation
                .clone();
            assert_eq!(mode.as_deref(), Some(expected));
        }
    }

    #[test]
    fn the_trailing_blank_cycles_onto_loop_out_and_the_leading_blank_onto_loop_in() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));
        // The default track's tail is the right edge: interpolate -> hold -> loop_out.
        let tail_id = runtime
            .property_track("layer-1", "raster")
            .unwrap()
            .blocks
            .last()
            .unwrap()
            .id
            .clone();
        runtime.cycle_property_block_interp_mode("layer-1", "raster", &tail_id);
        runtime.cycle_property_block_interp_mode("layer-1", "raster", &tail_id);
        assert_eq!(
            runtime
                .property_track("layer-1", "raster")
                .unwrap()
                .blocks
                .last()
                .unwrap()
                .interpretation
                .as_deref(),
            Some("loop_out")
        );

        // Build a leading blank: destructive-drag block-1 (0..8) right to 8..12,
        // cropping block-2's front — the vacated 0..8 becomes the first block.
        assert!(
            runtime.set_property_block_timing_destructive("layer-1", "raster", "block-1", 8, 4,)
        );
        let shape = interp_test_layer_track(&runtime, "raster");
        let leading_blank_id = shape[0]
            .3
            .then(|| shape[0].0.clone())
            .expect("first block should be a blank");
        runtime.cycle_property_block_interp_mode("layer-1", "raster", &leading_blank_id);
        runtime.cycle_property_block_interp_mode("layer-1", "raster", &leading_blank_id);
        assert_eq!(
            runtime
                .property_track("layer-1", "raster")
                .unwrap()
                .blocks
                .first()
                .unwrap()
                .interpretation
                .as_deref(),
            Some("loop_in")
        );
    }

    #[test]
    fn swapping_strands_loop_modes_back_to_the_default_interpolate() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));
        runtime.split_property_block("layer-1", "raster", "block-1", 8);
        // Track: block-1 (0..8), block-2 (8..24), tail (24..25).
        let tail_id = runtime
            .property_track("layer-1", "raster")
            .unwrap()
            .blocks
            .last()
            .unwrap()
            .id
            .clone();
        runtime.cycle_property_block_interp_mode("layer-1", "raster", &tail_id);
        runtime.cycle_property_block_interp_mode("layer-1", "raster", &tail_id);
        assert_eq!(
            runtime
                .property_track("layer-1", "raster")
                .unwrap()
                .blocks
                .last()
                .unwrap()
                .interpretation
                .as_deref(),
            Some("loop_out")
        );

        // Swap the loop_out tail with block-1: the tail's span (24..25) now sits in
        // the middle of the track — the loop mode must not survive there.
        assert!(runtime.swap_property_blocks("layer-1", "raster", "block-1", &tail_id));
        let blocks = &runtime.property_track("layer-1", "raster").unwrap().blocks;
        assert!(
            blocks.iter().all(|b| b.interpretation.is_none()),
            "no stranded loop modes: {:?}",
            blocks
                .iter()
                .map(|b| (b.id.as_str(), b.interpretation.clone()))
                .collect::<Vec<_>>()
        );
        // And the track still ends blank with a fresh trailing representative.
        assert!(blocks.last().unwrap().is_blank);
    }

    #[test]
    fn loading_a_document_normalizes_tracks_into_the_binary_tiling() {
        // A pre-binary document: one solid move block, no trailing blank, and a
        // loop_out stranded on a middle blank. Loading must re-tile.
        let mut document = SharedDocumentFile::single_layer("doc-1", "Doc", "layer-1", "Layer 1");
        document.layers[0].property_tracks[1].blocks = vec![SharedDocumentPropertyBlock {
            id: "block-1".to_string(),
            start_breath: 0,
            length_breaths: 8,
            is_blank: false,
            value: Some(serde_json::json!({"x": 5, "y": 0, "z": 0})),
            interpretation: None,
            ease_out_percent: None,
            ease_in_percent: None,
        }];
        let runtime = SharedDocumentRuntime::new(document);
        let blocks = &runtime.property_track("layer-1", "move").unwrap().blocks;
        assert_eq!(blocks.len(), 2, "solid + trailing blank after load");
        assert!(!blocks[0].is_blank);
        assert!(blocks[1].is_blank);
    }

    #[test]
    fn move_playback_interpolates_holds_and_loops_through_the_empty_modes() {
        let mut runtime = SharedDocumentRuntime::new(SharedDocumentFile::single_layer(
            "doc-1", "Doc", "layer-1", "Layer 1",
        ));
        // Build: solid (0..4, +0x), empty (4..8), solid (8..12, +8x), tail.
        // Default track is block-1 (0..24 solid) + tail (24..25); split block-1
        // at 4, then drag block-2's span to 8..12 — the 4..8 gap becomes the
        // empty and 12..24 merges into the tail blank on re-tile.
        runtime.split_property_block("layer-1", "move", "block-1", 4);
        {
            let track = runtime
                .ensure_property_track_mut("layer-1", "move")
                .unwrap();
            track.blocks[0].value = Some(serde_json::json!({"x": 0, "y": 0, "z": 0}));
            track.blocks[1].start_breath = 8;
            track.blocks[1].length_breaths = 4;
            track.blocks[1].value = Some(serde_json::json!({"x": 8, "y": 0, "z": 0}));
            track.blocks = retiled_property_track(
                std::mem::take(&mut track.blocks),
                0,
                default_layer_length_breaths(),
            );
        }
        let spans: Vec<(u32, u32, bool)> = runtime
            .property_track("layer-1", "move")
            .unwrap()
            .blocks
            .iter()
            .map(|b| {
                (
                    b.start_breath,
                    b.start_breath + b.length_breaths,
                    b.is_blank,
                )
            })
            .collect();
        assert_eq!(
            spans,
            vec![(0, 4, false), (4, 8, true), (8, 12, false), (12, 25, true)]
        );
        // Solid at 0..4 is +0, solid at 8..12 is +8, empty 4..8 interpolates.
        assert_eq!(runtime.move_offset_for_layer("layer-1", 0).x, 0);
        assert_eq!(runtime.move_offset_for_layer("layer-1", 2).x, 0);
        assert_eq!(runtime.move_offset_for_layer("layer-1", 4).x, 2);
        assert_eq!(runtime.move_offset_for_layer("layer-1", 7).x, 6);
        assert_eq!(runtime.move_offset_for_layer("layer-1", 8).x, 8);

        // Cycle the 4..8 empty to hold: it carries the previous keyframe's +0.
        let empty_id = runtime
            .property_track("layer-1", "move")
            .unwrap()
            .blocks
            .iter()
            .find(|b| b.is_blank && b.start_breath == 4)
            .unwrap()
            .id
            .clone();
        runtime.cycle_property_block_interp_mode("layer-1", "move", &empty_id);
        assert_eq!(
            runtime
                .property_track("layer-1", "move")
                .unwrap()
                .blocks
                .iter()
                .find(|b| b.id == empty_id)
                .unwrap()
                .interpretation
                .as_deref(),
            Some("hold")
        );
        assert_eq!(runtime.move_offset_for_layer("layer-1", 6).x, 0);

        // Loop out on the tail: the authored region (0..12) repeats through it,
        // including past the tail's stored extent.
        let tail_id = runtime
            .property_track("layer-1", "move")
            .unwrap()
            .blocks
            .last()
            .unwrap()
            .id
            .clone();
        runtime.cycle_property_block_interp_mode("layer-1", "move", &tail_id);
        runtime.cycle_property_block_interp_mode("layer-1", "move", &tail_id);
        assert_eq!(
            runtime
                .property_track("layer-1", "move")
                .unwrap()
                .blocks
                .last()
                .unwrap()
                .interpretation
                .as_deref(),
            Some("loop_out")
        );
        assert_eq!(
            runtime.move_offset_for_layer("layer-1", 12).x,
            0,
            "loop wraps to region start"
        );
        assert_eq!(
            runtime.move_offset_for_layer("layer-1", 20).x,
            8,
            "loop maps into the +8 keyframe"
        );
        assert_eq!(runtime.move_offset_for_layer("layer-1", 23).x, 8);
        assert_eq!(
            runtime.move_offset_for_layer("layer-1", 24).x,
            0,
            "loop wraps again"
        );
        assert_eq!(
            runtime.move_offset_for_layer("layer-1", 36).x,
            0,
            "far past the stored extent the loop keeps playing"
        );
    }
}
