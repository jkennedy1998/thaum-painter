//! Cross-tool behavior rules shared by every registered tool. Residents:
//! which selection mode a tool forces when its hand edits the selection
//! surface, and how a tool behaves across a pointer drag. Geometry and
//! shape computations join here as tools accumulate, so shared logic is
//! written once and consumed through this standard seam.

#[path = "stroke_rules.rs"]
pub mod stroke_rules;

pub use stroke_rules::{drag_behavior, DragBehavior};

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
pub fn selection_behavior(_tool_id: &str) -> ToolSelectionBehavior {
    // Clearing is an authored character, not a tool, so every registered
    // tool follows the selection's current mode. A future genuinely
    // subtract-only tool can opt into ForceSubtract here.
    ToolSelectionBehavior::UseCurrentMode
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_and_registered_tools_use_the_current_mode() {
        for id in ["clear", "brush", "fill", "lasso", "text"] {
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
