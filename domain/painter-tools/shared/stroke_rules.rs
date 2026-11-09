//! Cross-tool drag behavior rules: how a registered tool behaves across a
//! pointer drag on the paint canvas. Written once here so tool-state and the
//! pointer dispatch classify a tool by behavior instead of matching tool
//! identities arm by arm.

/// How a tool behaves across a pointer drag on the paint canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragBehavior {
    /// Applies at every drag position through tool-state's per-position
    /// seams (brush, fill, and future paint tools).
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
    /// Holds a selection-move stroke: a press captures the active plane
    /// selection's content, drags track the offset, release commits one
    /// bounded move (clear old area, paste at the new place, re-anchor the
    /// selection there). The pointer lifecycle owns the in-progress move
    /// stroke (move).
    MoveSelection,
    /// A press begins a vector construction session that stays open across
    /// many presses; Enter, Escape, or switching tools commits the
    /// accumulated shape — the pointer lifecycle owns the in-progress
    /// point/handle state (vector).
    VectorConstruction,
}

/// Which drag behavior the tool with this registration id uses.
pub fn drag_behavior(tool_id: &str) -> DragBehavior {
    match tool_id {
        "lasso" => DragBehavior::ReleaseBound,
        "text" => DragBehavior::TypingSession,
        "picker" => DragBehavior::ClickOnly,
        "stamp" => DragBehavior::ClickOnly,
        // Detailer seeds through its click dispatch; canvas-pointer then
        // applies its separate cell-local subcell samples across the drag.
        "detailer" => DragBehavior::ClickOnly,
        "move" => DragBehavior::MoveSelection,
        "vector" => DragBehavior::VectorConstruction,
        // Unknown registrations behave as ordinary per-position paint tools.
        _ => DragBehavior::PerPosition,
    }
}

/// Whether the tool with this registration id carries the shared stroke/fill
/// property rows (the per-hand Left/None/Right authoring-source pair). Only
/// lasso and vector author drawn shapes today (J 2026-11-09); the other six
/// tools keep their own content seams.
pub fn carries_stroke_fill_rows(tool_id: &str) -> bool {
    matches!(tool_id, "lasso" | "vector")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drag_behavior_covers_every_registered_tool() {
        assert_eq!(drag_behavior("brush"), DragBehavior::PerPosition);
        assert_eq!(drag_behavior("fill"), DragBehavior::PerPosition);
        assert_eq!(drag_behavior("lasso"), DragBehavior::ReleaseBound);
        assert_eq!(drag_behavior("text"), DragBehavior::TypingSession);
        assert_eq!(drag_behavior("picker"), DragBehavior::ClickOnly);
        assert_eq!(drag_behavior("stamp"), DragBehavior::ClickOnly);
        assert_eq!(drag_behavior("detailer"), DragBehavior::ClickOnly);
    }

    #[test]
    fn unknown_registrations_default_to_per_position() {
        assert_eq!(drag_behavior("future-shape"), DragBehavior::PerPosition);
    }

    #[test]
    fn vector_is_a_construction_session() {
        assert_eq!(drag_behavior("vector"), DragBehavior::VectorConstruction);
    }

    #[test]
    fn only_lasso_and_vector_carry_the_stroke_fill_rows() {
        assert!(carries_stroke_fill_rows("lasso"));
        assert!(carries_stroke_fill_rows("vector"));
        assert!(!carries_stroke_fill_rows("brush"));
        assert!(!carries_stroke_fill_rows("fill"));
        assert!(!carries_stroke_fill_rows("text"));
        assert!(!carries_stroke_fill_rows("picker"));
        assert!(!carries_stroke_fill_rows("stamp"));
        assert!(!carries_stroke_fill_rows("detailer"));
        assert!(!carries_stroke_fill_rows("move"));
        assert!(!carries_stroke_fill_rows("future-shape"));
    }
}
