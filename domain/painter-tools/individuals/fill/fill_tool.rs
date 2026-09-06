use crate::painter_tools::{require_by_id, RegisteredTool, ToolDescriptor};

/// The fill tool's registration home under `painter-tools/individuals/`.
/// Session paint behavior still lives in `painter-session/tool-state` match
/// arms; this folder owns fill's registration truth and hosts the behavior
/// when it migrates.
pub struct FillTool;

impl RegisteredTool for FillTool {
    fn descriptor() -> &'static ToolDescriptor {
        require_by_id("fill")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_resolves_its_own_descriptor() {
        let descriptor = FillTool::descriptor();
        assert_eq!(descriptor.id, "fill");
        assert_eq!(descriptor.label, "Fill");
        assert_eq!(descriptor.icon, '█');
        assert_eq!(
            descriptor.property_row_ids,
            &["fill_diagonal", "fill_match_channels"]
        );
        assert_eq!(descriptor.select_action, Some("painter_select_bucket"));
        assert_eq!(descriptor.hotkey, Some("B"));
    }
}
