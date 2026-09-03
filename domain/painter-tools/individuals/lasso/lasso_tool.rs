use crate::painter_tools::{require_by_id, RegisteredTool, ToolDescriptor};

/// The lasso tool's registration home under `painter-tools/individuals/`.
/// The lasso records a freehand bound on drag and fills the enclosed cells
/// on release; rasterization is pure (`painter-operations/shapes/lasso`)
/// and session application lives in `painter-session/tool-state`.
pub struct LassoTool;

impl RegisteredTool for LassoTool {
    fn descriptor() -> &'static ToolDescriptor {
        require_by_id("lasso")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lasso_resolves_its_own_descriptor() {
        let descriptor = LassoTool::descriptor();
        assert_eq!(descriptor.id, "lasso");
        assert_eq!(descriptor.label, "Lasso");
        assert_eq!(descriptor.icon, '◌');
        assert!(descriptor.property_row_ids.is_empty());
        assert_eq!(descriptor.select_action, Some("painter_select_lasso"));
        assert_eq!(descriptor.hotkey, Some("L"));
    }
}
