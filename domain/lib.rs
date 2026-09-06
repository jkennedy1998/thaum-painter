#[path = "painter-operations/brush/brush.rs"]
pub mod brush;
#[path = "rendering/camera/camera_actions.rs"]
pub mod camera_actions;
#[path = "rendering/camera/camera_viewport.rs"]
pub mod camera_viewport;
#[path = "painter-session/canvas-pointer/canvas_pointer.rs"]
pub mod canvas_pointer;
#[path = "painter-session/clipboard/copy_paste.rs"]
pub mod clipboard;
#[path = "painter-session/identity/identity.rs"]
pub mod identity;
/// Debug logging moved into the renderer domain so renderer and host share one
/// sink; the painter keeps the same call path through this re-export.
pub use thaum_renderer_domain::debug_log;
#[path = "file/document_locations.rs"]
pub mod document_locations;
#[path = "painter-session/sync/document_sync.rs"]
pub mod document_sync;
#[path = "painter-operations/fill/fill.rs"]
pub mod fill;
#[path = "modules/individuals/graphic-picker/graphic_picker_module.rs"]
pub mod graphic_picker_module;
#[path = "modules/individuals/hand-settings/hand_settings_module.rs"]
pub mod hand_settings_module;
#[path = "painter-document/properties/interpolation/interpolation.rs"]
pub mod interpolation;
#[path = "painter-operations/shapes/lasso.rs"]
pub mod lasso;
#[path = "painter-session/tool-state/lasso_stroke.rs"]
pub mod lasso_stroke;
#[path = "modules/individuals/layers-panel/layers_panel_module.rs"]
pub mod layers_panel_module;
#[path = "painter-document/layers/layers_runtime.rs"]
pub mod layers_runtime;
#[path = "modules/shared/legacy_indexed_palette.rs"]
pub mod legacy_indexed_palette;
#[path = "file/manifest/manifest.rs"]
pub mod manifest;
#[path = "modules/individuals/material-picker/material_picker_module.rs"]
pub mod material_picker_module;
#[path = "modules/individuals/paint-canvas-bounds/paint_canvas_bounds_module.rs"]
pub mod paint_canvas_bounds_module;
#[path = "painter-session/paint-color/paint_color.rs"]
pub mod paint_color;
#[path = "modules/individuals/paint-color-block/paint_color_block_module.rs"]
pub mod paint_color_block_module;
#[path = "modules/individuals/paint-color-picker/paint_color_picker_module.rs"]
pub mod paint_color_picker_module;
#[path = "painter-tools/painter_tools.rs"]
pub mod painter_tools;
#[path = "painter-document/properties/properties.rs"]
pub mod properties;
#[path = "rendering/render-space/render_space.rs"]
pub mod render_space;
#[path = "painter-session/selection/selection_actions.rs"]
pub mod selection_actions;
#[path = "painter-session/selection/selection_state.rs"]
pub mod selection_state;
#[path = "painter-session/selection/selection_stroke.rs"]
pub mod selection_stroke;
#[path = "painter-session/session_document.rs"]
pub mod session_document;
#[path = "file/storage/storage.rs"]
pub mod storage;
#[path = "tai/tai.rs"]
pub mod tai;
#[path = "painter-operations/text/text.rs"]
pub mod text;
#[path = "painter-session/text-entry/text_entry.rs"]
pub mod text_entry;
#[path = "painter-session/timeline-state/timeline_state.rs"]
pub mod timeline_state;
#[path = "painter-session/tool-state/tool_state.rs"]
pub mod tool_state;
#[path = "modules/individuals/toolbar/toolbar_module.rs"]
pub mod toolbar_module;
#[path = "modules/individuals/toolbox/toolbox_module.rs"]
pub mod toolbox_module;
#[path = "persistence/user-session-state/user_session_state.rs"]
pub mod user_session_state;

pub use brush::{apply_brush, brush_points, erase, Canvas, PaintedCell};
pub use camera_actions::{apply_painter_camera_action, apply_painter_pan_action};
pub use fill::{
    flood_fill, flood_fill_with_connectivity, CanvasBounds, CanvasPlaneAxis, FillConnectivity,
};
pub use graphic_picker_module::GraphicPickerModule;
pub use identity::{SessionIdentity, PRESENCE_CANDIDATE_COLORS};
pub use hand_settings_module::HandSettingsModule;
pub use interpolation::{resolve_gap_fill, surrounding_items, BreathRanged, GapFill};
pub use layers_panel_module::{
    LayerPropertyKind, LayerRow, LayersPanelAction, LayersPanelModule, LayersPanelState,
    MergeDirection, PropertyTrackBlock, PropertyTrackRow,
};
pub use legacy_indexed_palette::legacy_indexed_palette;
pub use manifest::{
    parse_manifest, parse_manifest_from_str, BreathWindow, DocumentBounds, DocumentContent,
    GridPoint, Group, GroupProperty, ImportExportBookkeeping, LastExport, Manifest,
    ManifestMetadata, ParticleEffect, ParticleEffectVisual, PlaybackWindow, PropertyBlock,
    RasterSegment, Rgb, SavedCameraDefaults, TimeAssets, Voxel,
};
pub use material_picker_module::MaterialPickerModule;
pub use paint_canvas_bounds_module::{DrawingSpaceWheelMode, PaintCanvasBoundsModule};
pub use paint_color::PaintColor;
pub use paint_color_block_module::PaintColorBlockModule;
pub use paint_color_picker_module::PaintColorPickerModule;
pub use properties::{block_covering_breath, breath_in_span, clamped_breath_span, span_end_breath};
pub use render_space::{build_composition, build_data_lanes, build_render_space, RenderSpace};
pub use selection_actions::apply_painter_selection_action;
pub use selection_state::{
    flood_select_points, PainterSelection, PlaneSelection, SelectionMode, WorldSelection,
};
pub use storage::{
    append_action_record, load_or_create_shared_document, save_shared_document_snapshot,
    write_action_records_atomic, PersistedCellPoint, PersistedSharedGraphic,
    PersistedSharedPaintColor, PersistedSharedPaintedCell, PropertyBlockMergeDirection,
    SharedCellPatch, SharedDocumentAction, SharedDocumentActionRecord, SharedDocumentFile,
    SharedDocumentLayer, SharedDocumentPaths, SharedDocumentPropertyBlock,
    SharedDocumentPropertyTrack, SharedDocumentRuntime, SharedDocumentSelection,
    SharedDocumentSelectionChannel, SharedSelectionWriteMode, DEFAULT_SELECTION_CHANNEL_ID,
    SHARED_DOCUMENT_KIND, SHARED_DOCUMENT_SCHEMA_VERSION, UNDO_HISTORY_DEPTH,
};
pub use timeline_state::TimelineState;
pub use tool_state::{
    EditChannels, HandState, PaintChannel, PaintHand, PaintTarget, PaintTool, ToolState,
};
pub use toolbar_module::{ToolbarButton, ToolbarModule};
pub use toolbox_module::{ToolDef, ToolboxModule};
pub use user_session_state::{
    PainterUserSessionState, PersistedPainterUiState, painter_default_camera,
};
