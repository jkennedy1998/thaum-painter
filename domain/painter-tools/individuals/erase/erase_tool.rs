use crate::painter_tools::{require_by_id, RegisteredTool, ToolDescriptor};

/// The erase tool's registration home under `painter-tools/individuals/`.
/// Session paint behavior still lives in `painter-session/tool-state` match
/// arms; this folder owns erase's registration truth and hosts the behavior
/// when it migrates.
pub struct EraseTool;

impl RegisteredTool for EraseTool {
    fn descriptor() -> &'static ToolDescriptor {
        require_by_id("erase")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn erase_resolves_its_own_descriptor() {
        let descriptor = EraseTool::descriptor();
        assert_eq!(descriptor.id, "erase");
        assert_eq!(descriptor.label, "Erase");
        assert_eq!(descriptor.icon, '=');
        assert_eq!(descriptor.property_row_ids, &["brush_size"]);
        assert_eq!(descriptor.select_action, None);
        assert_eq!(descriptor.hotkey, None);
    }
}
