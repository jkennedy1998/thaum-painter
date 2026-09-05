use crate::painter_tools::{require_by_id, RegisteredTool, ToolDescriptor};

/// The stamp tool's registration home under `painter-tools/individuals/`.
/// A click with the stamp places the user's copied world cells at the click
/// cell, resolved through the hand's locked graphic/color/weight exactly
/// like brush/fill; the payload shape lives in `painter-session/clipboard`
/// and the application seams in `painter-session/tool-state`.
pub struct StampTool;

impl RegisteredTool for StampTool {
    fn descriptor() -> &'static ToolDescriptor {
        require_by_id("stamp")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamp_resolves_its_own_descriptor() {
        let descriptor = StampTool::descriptor();
        assert_eq!(descriptor.id, "stamp");
        assert_eq!(descriptor.label, "Stamp");
        assert_eq!(descriptor.icon, '❖');
        assert!(descriptor.property_row_ids.is_empty());
        assert_eq!(descriptor.select_action, Some("painter_select_stamp"));
        assert_eq!(descriptor.hotkey, Some("V"));
    }
}
