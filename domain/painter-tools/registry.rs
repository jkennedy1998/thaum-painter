//! Registration seam for painter tools. Each tool folder under
//! `painter-tools/individuals/` owns one descriptor registered here; every
//! registration consumer (toolbox list, hand-settings labels, persistence
//! names, hotkey drift tests) reads this list instead of keeping its own
//! copy, so adding a tool is one folder plus one registry line.

/// One registered painter tool's registration truth. Session behavior
/// (edit/select semantics) still lives in `painter-session/tool-state`
/// match arms until per-tool behavior migration; this descriptor only owns
/// what registration consumers need.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolDescriptor {
    /// Stable persistence id, e.g. "brush"; matches tool-state's `PaintTool::id`.
    pub id: &'static str,
    /// Human label shown by the toolbox and hand-settings panel.
    pub label: &'static str,
    /// Toolbox glyph.
    pub icon: char,
    /// Toolbox tooltip copy: what the tool does.
    pub description: &'static str,
    /// Tool-specific property row ids this tool uses in the properties panel.
    pub property_row_ids: &'static [&'static str],
    /// Hotkey action that equips this tool, if any, e.g. "painter_select_pencil".
    pub select_action: Option<&'static str>,
    /// Default key bound to `select_action` in the tai bindings, if any.
    pub hotkey: Option<&'static str>,
}

/// Every registered painter tool, in toolbox order.
pub const ALL_TOOLS: &[ToolDescriptor] = &[
    ToolDescriptor {
        id: "brush",
        label: "Brush",
        icon: '|',
        description: "edit cell content near the cursor",
        property_row_ids: &["brush_size"],
        select_action: Some("painter_select_pencil"),
        hotkey: Some("P"),
    },
    ToolDescriptor {
        id: "erase",
        label: "Erase",
        icon: '=',
        description: "clear cell content near the cursor",
        property_row_ids: &["brush_size"],
        select_action: None,
        hotkey: None,
    },
    ToolDescriptor {
        id: "fill",
        label: "Fill",
        icon: '▧',
        description: "flood-fill matching cells near the cursor",
        property_row_ids: &["fill_diagonal", "fill_match_channels"],
        select_action: Some("painter_select_bucket"),
        hotkey: Some("B"),
    },
    ToolDescriptor {
        id: "lasso",
        label: "Lasso",
        icon: '◌',
        description: "select cells by drawing around them",
        property_row_ids: &[],
        select_action: Some("painter_select_lasso"),
        hotkey: Some("L"),
    },
    ToolDescriptor {
        id: "stamp",
        label: "Stamp",
        icon: '□',
        description: "use the current clipboard content as a stamp",
        property_row_ids: &[],
        select_action: Some("painter_select_stamp"),
        hotkey: Some("V"),
    },
    ToolDescriptor {
        id: "move",
        label: "Move",
        icon: '#',
        description: "move selected cell content",
        property_row_ids: &[],
        select_action: Some("painter_select_move"),
        hotkey: Some("M"),
    },
    ToolDescriptor {
        id: "text",
        label: "Text",
        icon: 'a',
        description: "type out cell content",
        property_row_ids: &["text_char_step", "text_enter_step"],
        select_action: None,
        hotkey: None,
    },
    ToolDescriptor {
        id: "picker",
        label: "Picker",
        icon: '◉',
        description: "pick the content and color under the cursor",
        property_row_ids: &["picker_opposite_hand"],
        select_action: None,
        hotkey: None,
    },
];

pub fn all() -> &'static [ToolDescriptor] {
    ALL_TOOLS
}

pub fn by_id(id: &str) -> Option<&'static ToolDescriptor> {
    ALL_TOOLS.iter().find(|descriptor| descriptor.id == id)
}

pub fn require_by_id(id: &str) -> &'static ToolDescriptor {
    by_id(id).unwrap_or_else(|| panic!("unregistered painter tool id: {id}"))
}

/// One folder under `individuals/` implements this so its descriptor has a
/// named owner and the drift tests can assert folder <-> registry parity.
pub trait RegisteredTool {
    fn descriptor() -> &'static ToolDescriptor;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_ids_are_unique_and_non_empty() {
        let mut ids: Vec<&str> = all().iter().map(|d| d.id).collect();
        assert!(!ids.is_empty());
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), all().len());
    }

    #[test]
    fn select_actions_and_hotkeys_are_paired() {
        for descriptor in all() {
            assert_eq!(
                descriptor.select_action.is_some(),
                descriptor.hotkey.is_some(),
                "{} must pair select_action with hotkey",
                descriptor.id
            );
        }
    }

    #[test]
    fn registered_tools_match_their_individual_folders() {
        // One assertion per `individuals/` folder: each tool type resolves
        // its own descriptor from this registry.
        assert_eq!(BrushTool::descriptor().id, "brush");
        assert_eq!(EraseTool::descriptor().id, "erase");
        assert_eq!(FillTool::descriptor().id, "fill");
        assert_eq!(LassoTool::descriptor().id, "lasso");
        assert_eq!(TextTool::descriptor().id, "text");
        assert_eq!(PickerTool::descriptor().id, "picker");
        assert_eq!(StampTool::descriptor().id, "stamp");
        assert_eq!(MoveTool::descriptor().id, "move");
        assert_eq!(all().len(), 8);
    }

    use crate::painter_tools::brush::BrushTool;
    use crate::painter_tools::erase::EraseTool;
    use crate::painter_tools::fill::FillTool;
    use crate::painter_tools::lasso::LassoTool;
    use crate::painter_tools::move_tool::MoveTool;
    use crate::painter_tools::picker::PickerTool;
    use crate::painter_tools::stamp::StampTool;
    use crate::painter_tools::text::TextTool;
}
