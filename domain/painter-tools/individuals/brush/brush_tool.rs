use crate::painter_tools::{require_by_id, RegisteredTool, ToolDescriptor};

/// The brush tool's registration home under `painter-tools/individuals/`.
/// Session paint behavior still lives in `painter-session/tool-state` match
/// arms; this folder owns brush's registration truth and hosts the behavior
/// when it migrates.
pub struct BrushTool;

impl RegisteredTool for BrushTool {
    fn descriptor() -> &'static ToolDescriptor {
        require_by_id("brush")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brush_resolves_its_own_descriptor() {
        let descriptor = BrushTool::descriptor();
        assert_eq!(descriptor.id, "brush");
        assert_eq!(descriptor.label, "Brush");
        assert_eq!(descriptor.icon, '✎');
        assert_eq!(descriptor.property_row_ids, &["brush_size"]);
        assert_eq!(descriptor.select_action, Some("painter_select_pencil"));
        assert_eq!(descriptor.hotkey, Some("P"));
    }
}
