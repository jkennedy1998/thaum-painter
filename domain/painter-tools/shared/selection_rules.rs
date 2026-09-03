//! Cross-tool behavior rules shared by every registered tool. First
//! resident: which selection mode a tool forces when its hand edits the
//! selection surface. Geometry and shape computations join here as tools
//! accumulate, so shared logic is written once and consumed through this
//! standard seam.

/// How a tool behaves when its hand edits the selection surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolSelectionBehavior {
    /// Apply the selection's own current mode.
    UseCurrentMode,
    /// Always apply subtract, regardless of the current mode.
    ForceSubtract,
}

impl ToolSelectionBehavior {
    /// Resolve this behavior into a concrete mode. `forced` is the value
    /// forcing behaviors resolve to; the caller supplies the selection-owned
    /// type so this seam stays dependency-free.
    pub fn resolve<M>(self, current: M, forced: M) -> M {
        match self {
            Self::UseCurrentMode => current,
            Self::ForceSubtract => forced,
        }
    }
}

/// Which selection behavior the tool with this registration id uses when
/// its hand edits the selection surface.
pub fn selection_behavior(tool_id: &str) -> ToolSelectionBehavior {
    match tool_id {
        "erase" => ToolSelectionBehavior::ForceSubtract,
        _ => ToolSelectionBehavior::UseCurrentMode,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn erase_forces_subtract_and_other_tools_use_the_current_mode() {
        assert_eq!(
            selection_behavior("erase"),
            ToolSelectionBehavior::ForceSubtract
        );
        for id in ["brush", "fill", "text"] {
            assert_eq!(
                selection_behavior(id),
                ToolSelectionBehavior::UseCurrentMode,
                "{id} should use the current selection mode"
            );
        }
    }

    #[test]
    fn resolve_picks_the_matching_arm() {
        assert_eq!(
            ToolSelectionBehavior::ForceSubtract.resolve("current", "forced"),
            "forced"
        );
        assert_eq!(
            ToolSelectionBehavior::UseCurrentMode.resolve("current", "forced"),
            "current"
        );
    }

    #[test]
    fn every_registered_tool_resolves_a_behavior() {
        for descriptor in crate::painter_tools::all() {
            let _ = selection_behavior(descriptor.id);
        }
    }
}
