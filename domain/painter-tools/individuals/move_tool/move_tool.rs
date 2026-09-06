use crate::painter_tools::{require_by_id, RegisteredTool, ToolDescriptor};

/// The move tool's registration home under `painter-tools/individuals/`.
/// With an active selection, a press-drag-release moves the selected raster
/// content (one bounded undoable op: clear the old selection area, paste at
/// the new place, re-anchor the selection there); without a selection it is
/// a stub for the future layer-offset behavior. Interaction semantics live
/// in `painter-session/canvas-pointer`, gating in
/// `painter-session/tool-state`.
pub struct MoveTool;

impl RegisteredTool for MoveTool {
    fn descriptor() -> &'static ToolDescriptor {
        require_by_id("move")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_resolves_its_own_descriptor() {
        let descriptor = MoveTool::descriptor();
        assert_eq!(descriptor.id, "move");
        assert_eq!(descriptor.label, "Move");
        assert_eq!(descriptor.icon, '#');
        assert!(descriptor.property_row_ids.is_empty());
        assert_eq!(descriptor.select_action, Some("painter_select_move"));
        assert_eq!(descriptor.hotkey, Some("M"));
    }
}
