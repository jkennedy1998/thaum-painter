//! Cross-tool drag behavior rules: how a registered tool behaves across a
//! pointer drag on the paint canvas. Written once here so tool-state and the
//! pointer dispatch classify a tool by behavior instead of matching tool
//! identities arm by arm.

/// How a tool behaves across a pointer drag on the paint canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragBehavior {
    /// Applies at every drag position through tool-state's per-position
    /// seams (brush, erase, fill).
    PerPosition,
    /// Records a bound across the drag and commits once on release — the
    /// pointer lifecycle holds an in-progress stroke for this tool (lasso).
    ReleaseBound,
    /// Never paints through pointer dispatch; a press begins a typing
    /// session that owns the keyboard until it exits (text).
    TypingSession,
    /// Never paints and never selects; a press samples the cell under the
    /// cursor into a hand's state once, and drags do nothing (picker).
    ClickOnly,
}

/// Which drag behavior the tool with this registration id uses.
pub fn drag_behavior(tool_id: &str) -> DragBehavior {
    match tool_id {
        "lasso" => DragBehavior::ReleaseBound,
        "text" => DragBehavior::TypingSession,
        "picker" => DragBehavior::ClickOnly,
        "stamp" => DragBehavior::ClickOnly,
        // Unknown registrations behave as ordinary per-position paint tools.
        _ => DragBehavior::PerPosition,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drag_behavior_covers_every_registered_tool() {
        assert_eq!(drag_behavior("brush"), DragBehavior::PerPosition);
        assert_eq!(drag_behavior("erase"), DragBehavior::PerPosition);
        assert_eq!(drag_behavior("fill"), DragBehavior::PerPosition);
        assert_eq!(drag_behavior("lasso"), DragBehavior::ReleaseBound);
        assert_eq!(drag_behavior("text"), DragBehavior::TypingSession);
        assert_eq!(drag_behavior("picker"), DragBehavior::ClickOnly);
        assert_eq!(drag_behavior("stamp"), DragBehavior::ClickOnly);
    }

    #[test]
    fn unknown_registrations_default_to_per_position() {
        assert_eq!(drag_behavior("future-shape"), DragBehavior::PerPosition);
    }
}
