use crate::painter_tools::{require_by_id, RegisteredTool, ToolDescriptor};

/// The subcell glyph-detailing tool's registration home.
pub struct DetailerTool;

impl RegisteredTool for DetailerTool {
    fn descriptor() -> &'static ToolDescriptor {
        require_by_id("detailer")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detailer_resolves_its_own_descriptor() {
        let descriptor = DetailerTool::descriptor();
        assert_eq!(descriptor.id, "detailer");
        assert_eq!(
            descriptor.property_row_ids,
            &["detailer_mode", "multi_weight", "brush_size"]
        );
    }
}
