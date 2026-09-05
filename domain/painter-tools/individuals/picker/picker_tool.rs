use crate::painter_tools::{require_by_id, RegisteredTool, ToolDescriptor};

/// The picker tool's registration home under `painter-tools/individuals/`.
/// Session sampling behavior lives in `painter-session/tool-state`
/// (`pick_at_for_hand`); this folder owns picker's registration truth and
/// hosts the behavior when it migrates.
///
/// The picker is a selection-based tool, not an editing one: a press
/// samples the cell under the cursor into the receiving hand's
/// graphic/color/weight, gated by the picking hand's edit-channel toggles,
/// and never paints or touches the selection surface.
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
        assert_eq!(descriptor.icon, '◉');
        assert_eq!(descriptor.property_row_ids, &["picker_opposite_hand"]);
        assert_eq!(descriptor.select_action, None);
        assert_eq!(descriptor.hotkey, None);
    }
}
