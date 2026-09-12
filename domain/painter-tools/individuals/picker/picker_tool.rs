use crate::painter_tools::{require_by_id, RegisteredTool, ToolDescriptor};

/// The picker tool's registration home under `painter-tools/individuals/`.
/// Session sampling behavior lives in `painter-session/tool-state`
/// (`pick_at_for_hand`); this folder owns picker's registration truth and
/// hosts the behavior when it migrates.
///
/// The picker is a selection-based tool, not an editing one: a press
/// samples the cells under the picking hand's brush-tip footprint into the
/// receiving hand's graphic/color/weight, gated by the picking hand's
/// edit-channel toggles, and never paints or touches the selection surface.
///
/// Empty-cell rule (J 2026-09-12): picking an empty cell sets the receiving
/// hand's character to the blank — the picker doubles as the eraser, so J
/// keeps it out instead of swapping to an empty brush. Empties carry no
/// color or weight, so those channels are untouched.
///
/// Size rule: with a size-N footprint the sampled cells fold through the
/// raster interpolation seam (`interp_raster::blend_cell_run`) at the
/// halfway crossing, so a multi-character pick resolves to a character
/// blended between the sampled ones (colors lerp then snap to the indexed
/// palette, weights lerp, glyphs walk the shape-fade gradient when a
/// resolver is injected).
///
/// Clear rule (J 2026-09-12): a footprint straddling clear interpolates
/// between its content and its clear cells exactly the way raster
/// interpolation fades one-sided cells into clear — the clear fraction
/// walks the folded glyph toward the dot `.` clear-transition glyph and
/// scales its weight toward Zero by the content fraction.
pub struct PickerTool;

impl RegisteredTool for PickerTool {
    fn descriptor() -> &'static ToolDescriptor {
        require_by_id("picker")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picker_resolves_its_own_descriptor() {
        let descriptor = PickerTool::descriptor();
        assert_eq!(descriptor.id, "picker");
        assert_eq!(descriptor.label, "Picker");
        assert_eq!(descriptor.icon, 'V');
        assert_eq!(
            descriptor.property_row_ids,
            &["picker_opposite_hand", "brush_size"]
        );
        assert_eq!(descriptor.select_action, None);
        assert_eq!(descriptor.hotkey, None);
    }
}
