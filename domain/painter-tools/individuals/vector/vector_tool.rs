use crate::painter_tools::{require_by_id, RegisteredTool, ToolDescriptor};

/// The vector tool's registration home under `painter-tools/individuals/`.
/// The vector tool authors shapes from points and handles across a
/// construction session; geometry is pure (`painter-operations/shapes/vector`)
/// and session behavior lives in `painter-session/tool-state`.
pub struct VectorTool;

impl RegisteredTool for VectorTool {
    fn descriptor() -> &'static ToolDescriptor {
        require_by_id("vector")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vector_resolves_its_own_descriptor() {
        let descriptor = VectorTool::descriptor();
        assert_eq!(descriptor.id, "vector");
        assert_eq!(descriptor.label, "Vector");
        assert_eq!(descriptor.icon, '/');
        assert_eq!(descriptor.property_row_ids, &["stroke", "fill", "brush_size"]);
        assert_eq!(descriptor.select_action, Some("painter_select_vector"));
        assert_eq!(descriptor.hotkey, Some("H"));
    }
}
