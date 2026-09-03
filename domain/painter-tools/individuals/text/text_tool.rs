use crate::painter_tools::{require_by_id, RegisteredTool, ToolDescriptor};

/// The text tool's registration home under `painter-tools/individuals/`.
/// Typing behavior still lives in `painter-session/tool-state` plus the
/// session's `TextEntryState` bridge; this folder owns text's registration
/// truth and hosts the behavior when it migrates.
pub struct TextTool;

impl RegisteredTool for TextTool {
    fn descriptor() -> &'static ToolDescriptor {
        require_by_id("text")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_resolves_its_own_descriptor() {
        let descriptor = TextTool::descriptor();
        assert_eq!(descriptor.id, "text");
        assert_eq!(descriptor.label, "Text");
        assert_eq!(descriptor.icon, 'T');
        assert!(descriptor.property_row_ids.is_empty());
        assert_eq!(descriptor.select_action, None);
        assert_eq!(descriptor.hotkey, None);
    }
}
