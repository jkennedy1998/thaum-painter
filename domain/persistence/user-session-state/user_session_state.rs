use serde::{Deserialize, Serialize};
use thaum_renderer_domain::{
    CellGraphic, CellMaterialId, CommandBar, ControlsProfile, PersistedCommandBarState,
    PersistedRendererUiSessionState, SpriteGraphic,
};

use crate::{
    tool_state::ChannelMask,
    DrawingSpaceWheelMode, HandState, PaintColor, PaintHand, PaintTarget, PaintTool, SelectionMode,
    ToolState,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PainterUserSessionState {
    pub schema_version: u32,
    pub app_id: String,
    pub user_id: String,
    pub workspace_id: String,
    pub renderer: PersistedRendererUiSessionState,
    pub painter: PersistedPainterUiState,
    /// Per-user control-binding overrides over the painter's declared
    /// defaults. Rides the user session (never the document) so a user's
    /// remaps follow them across every document they open.
    #[serde(default = "ControlsProfile::new")]
    pub controls_profile: ControlsProfile,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedPainterUiState {
    pub drawing_space_wheel_mode: String,
    pub selection_mode: String,
    #[serde(default)]
    pub command_bar: PersistedCommandBarState,
    #[serde(default)]
    pub active_layer_id: Option<String>,
    /// Restored playhead breath. Boot clamps it to a breath the active layer's
    /// raster track actually covers, so a restored playhead can never sit in a
    /// raster gap where the layer renders nothing and strokes are rejected.
    #[serde(default)]
    pub current_breath: u32,
    pub tool_state: PersistedToolState,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedToolState {
    pub active_hand: String,
    pub left_tool: String,
    pub right_tool: String,
    pub left_hand: PersistedHandState,
    pub right_hand: PersistedHandState,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedHandState {
    pub graphic: PersistedGraphic,
    pub color: PersistedPaintColor,
    pub weight_index: i64,
    pub brush_size: i32,
    pub fill_diagonal: bool,
    #[serde(default)]
    pub fill_match_channels: bool,
    #[serde(default)]
    pub pick_opposite_hand: bool,
    pub edit_channels: PersistedChannelMask,
    pub select_channels: PersistedChannelMask,
    pub target: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedChannelMask {
    pub graphic: bool,
    pub color: bool,
    pub weight: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum PersistedPaintColor {
    FlatRgb { red: u8, green: u8, blue: u8 },
    Material { material: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum PersistedGraphic {
    None,
    Glyph { glyph: char },
    Sprite { atlas_relative_path: String },
}

impl PersistedPainterUiState {
    pub fn from_runtime(
        drawing_space_wheel_mode: DrawingSpaceWheelMode,
        selection_mode: SelectionMode,
        command_bar: &CommandBar,
        active_layer_id: Option<&str>,
        current_breath: u32,
        tool_state: &ToolState,
    ) -> Self {
        Self {
            drawing_space_wheel_mode: wheel_mode_name(drawing_space_wheel_mode).to_string(),
            selection_mode: selection_mode_name(selection_mode).to_string(),
            command_bar: command_bar.persisted_state(),
            active_layer_id: active_layer_id.map(str::to_string),
            current_breath,
            tool_state: PersistedToolState::from_runtime(tool_state),
        }
    }

    pub fn apply_to_runtime(
        &self,
        drawing_space_wheel_mode: &mut DrawingSpaceWheelMode,
        selection_mode: &mut SelectionMode,
        command_bar: &mut CommandBar,
        tool_state: &mut ToolState,
    ) {
        if let Some(mode) = wheel_mode_from_name(&self.drawing_space_wheel_mode) {
            *drawing_space_wheel_mode = mode;
        }
        if let Some(mode) = selection_mode_from_name(&self.selection_mode) {
            *selection_mode = mode;
        }
        command_bar.apply_persisted_state(&self.command_bar);
        self.tool_state.apply_to_runtime(tool_state);
    }
}

impl PersistedToolState {
    pub fn from_runtime(tool_state: &ToolState) -> Self {
        Self {
            active_hand: hand_name(tool_state.active_hand).to_string(),
            left_tool: tool_name(tool_state.left_tool).to_string(),
            right_tool: tool_name(tool_state.right_tool).to_string(),
            left_hand: PersistedHandState::from_runtime(&tool_state.left_hand),
            right_hand: PersistedHandState::from_runtime(&tool_state.right_hand),
        }
    }

    pub fn apply_to_runtime(&self, tool_state: &mut ToolState) {
        if let Some(hand) = hand_from_name(&self.active_hand) {
            tool_state.active_hand = hand;
        }
        if let Some(tool) = tool_from_name(&self.left_tool) {
            tool_state.left_tool = tool;
        }
        if let Some(tool) = tool_from_name(&self.right_tool) {
            tool_state.right_tool = tool;
        }
        self.left_hand.apply_to_runtime(&mut tool_state.left_hand);
        self.right_hand.apply_to_runtime(&mut tool_state.right_hand);
    }
}

impl PersistedHandState {
    pub fn from_runtime(hand: &HandState) -> Self {
        Self {
            graphic: PersistedGraphic::from_runtime(&hand.graphic),
            color: PersistedPaintColor::from_runtime(hand.color),
            weight_index: hand.weight_index,
            brush_size: hand.brush_size,
            fill_diagonal: hand.fill_diagonal,
            fill_match_channels: hand.fill_match_channels,
            pick_opposite_hand: hand.pick_opposite_hand,
            edit_channels: PersistedChannelMask::from_mask(hand.edit_channels),
            select_channels: PersistedChannelMask::from_mask(hand.select_channels),
            target: target_name(hand.target).to_string(),
        }
    }

    pub fn apply_to_runtime(&self, hand: &mut HandState) {
        hand.graphic = self.graphic.to_runtime();
        hand.color = self.color.to_runtime();
        hand.weight_index = self.weight_index.clamp(0, 3);
        hand.brush_size = self.brush_size.clamp(1, 5);
        hand.fill_diagonal = self.fill_diagonal;
        hand.fill_match_channels = self.fill_match_channels;
        hand.pick_opposite_hand = self.pick_opposite_hand;
        hand.edit_channels = self.edit_channels.to_mask();
        hand.select_channels = self.select_channels.to_mask();
        if let Some(target) = target_from_name(&self.target) {
            hand.target = target;
        }
    }
}

impl PersistedChannelMask {
    fn from_mask(mask: ChannelMask) -> Self {
        Self {
            graphic: mask.graphic,
            color: mask.color,
            weight: mask.weight,
        }
    }

    fn to_mask(&self) -> ChannelMask {
        ChannelMask {
            graphic: self.graphic,
            color: self.color,
            weight: self.weight,
        }
    }
}

impl PersistedPaintColor {
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

impl PersistedGraphic {
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

fn wheel_mode_name(mode: DrawingSpaceWheelMode) -> &'static str {
    match mode {
        DrawingSpaceWheelMode::Pan => "pan",
        DrawingSpaceWheelMode::Depth => "depth",
        DrawingSpaceWheelMode::Time => "time",
    }
}

fn wheel_mode_from_name(name: &str) -> Option<DrawingSpaceWheelMode> {
    match name {
        "pan" => Some(DrawingSpaceWheelMode::Pan),
        "depth" => Some(DrawingSpaceWheelMode::Depth),
        "time" => Some(DrawingSpaceWheelMode::Time),
        _ => None,
    }
}

fn selection_mode_name(mode: SelectionMode) -> &'static str {
    match mode {
        SelectionMode::Replace => "replace",
        SelectionMode::Additive => "additive",
        SelectionMode::Subtract => "subtract",
        SelectionMode::Intersect => "intersect",
    }
}

fn selection_mode_from_name(name: &str) -> Option<SelectionMode> {
    match name {
        "replace" => Some(SelectionMode::Replace),
        "additive" => Some(SelectionMode::Additive),
        "subtract" => Some(SelectionMode::Subtract),
        "intersect" => Some(SelectionMode::Intersect),
        _ => None,
    }
}

fn hand_name(hand: PaintHand) -> &'static str {
    match hand {
        PaintHand::Left => "left",
        PaintHand::Right => "right",
    }
}

fn hand_from_name(name: &str) -> Option<PaintHand> {
    match name {
        "left" => Some(PaintHand::Left),
        "right" => Some(PaintHand::Right),
        _ => None,
    }
}

fn tool_name(tool: PaintTool) -> &'static str {
    // Persistence names are the tool's stable registration id.
    tool.id()
}

fn tool_from_name(name: &str) -> Option<PaintTool> {
    PaintTool::from_id(name)
}

fn target_name(target: PaintTarget) -> &'static str {
    match target {
        PaintTarget::Image => "image",
        PaintTarget::Selection => "selection",
    }
}

fn target_from_name(name: &str) -> Option<PaintTarget> {
    match name {
        "image" => Some(PaintTarget::Image),
        "selection" => Some(PaintTarget::Selection),
        _ => None,
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

// --- session-state file IO and projection -----------------------------------

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use thaum_renderer_domain::{Camera, ModuleRegistry, UiPalette};

/// The acting user for this session: the `THAUM_SESSION_USER_ID` override, else
/// the OS user name, else a fixed local fallback.
pub fn session_user_id() -> String {
    env::var("THAUM_SESSION_USER_ID")
        .or_else(|_| env::var("USER"))
        .unwrap_or_else(|_| "local-user".to_string())
}

pub fn painter_session_state_path(artifacts_root: &Path, user_id: &str) -> PathBuf {
    artifacts_root
        .join("user-session-state")
        .join(format!("{user_id}.json"))
}

pub fn load_painter_user_session_state(path: &Path) -> Result<Option<PainterUserSessionState>> {
    if !path.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(path).with_context(|| {
        format!(
            "failed to read painter user session state at {}",
            path.display()
        )
    })?;
    let state = serde_json::from_str(&text).with_context(|| {
        format!(
            "failed to parse painter user session state JSON at {}",
            path.display()
        )
    })?;
    Ok(Some(state))
}

pub fn save_painter_user_session_state(path: &Path, state: &PainterUserSessionState) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create painter user session state directory {}",
                parent.display()
            )
        })?;
    }
    let text = serde_json::to_string_pretty(state)
        .context("failed to serialize painter user session state")?;
    fs::write(path, text).with_context(|| {
        format!(
            "failed to write painter user session state at {}",
            path.display()
        )
    })
}

#[allow(clippy::too_many_arguments)]
pub fn build_user_session_state(
    user_id: &str,
    camera: Camera,
    modules: &ModuleRegistry,
    ui_palette: &UiPalette,
    command_bar: &CommandBar,
    active_layer_id: Option<&str>,
    current_breath: u32,
    drawing_space_wheel_mode: crate::DrawingSpaceWheelMode,
    selection_mode: SelectionMode,
    tool_state: &ToolState,
) -> PainterUserSessionState {
    PainterUserSessionState {
        schema_version: 1,
        app_id: "thaum-painter".to_string(),
        user_id: user_id.to_string(),
        workspace_id: "default-workspace".to_string(),
        renderer: PersistedRendererUiSessionState::new(
            camera,
            modules.persisted_ui_state(),
            ui_palette,
        ),
        painter: PersistedPainterUiState::from_runtime(
            drawing_space_wheel_mode,
            selection_mode,
            command_bar,
            active_layer_id,
            current_breath,
            tool_state,
        ),
        controls_profile: ControlsProfile::new(),
    }
}
