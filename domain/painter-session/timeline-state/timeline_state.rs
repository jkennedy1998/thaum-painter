use crate::manifest::PropertyBlock;
use crate::properties::block_covering_breath;

/// Live, unsaved playhead and auto-key state for the painter session.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TimelineState {
    pub current_breath: u32,
    pub auto_key_enabled: bool,
}

impl TimelineState {
    pub fn set_current_breath(&mut self, breath: u32) {
        self.current_breath = breath;
    }

    pub fn toggle_auto_key(&mut self) {
        self.auto_key_enabled = !self.auto_key_enabled;
    }

    /// Resolves which breath a property edit at the current playhead is allowed to land on.
    /// Auto-key on always allows the current playhead. Auto-key off only allows the playhead
    /// when an existing bar in `blocks` already covers it; otherwise the edit is rejected.
    pub fn resolve_editable_breath(&self, blocks: &[PropertyBlock]) -> Option<u32> {
        if self.auto_key_enabled {
            return Some(self.current_breath);
        }
        block_covering_breath(blocks, self.current_breath).map(|_| self.current_breath)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn block(id: &str, start_breath: u32, end_breath: u32) -> PropertyBlock {
        PropertyBlock {
            id: id.to_string(),
            start_breath,
            end_breath,
            value: Value::Null,
        }
    }

    #[test]
    fn default_timeline_state_starts_at_breath_zero_with_auto_key_off() {
        let state = TimelineState::default();
        assert_eq!(state.current_breath, 0);
        assert!(!state.auto_key_enabled);
    }

    #[test]
    fn auto_key_on_always_allows_the_current_breath() {
        let mut state = TimelineState::default();
        state.auto_key_enabled = true;
        state.set_current_breath(42);
        assert_eq!(state.resolve_editable_breath(&[]), Some(42));
    }

    #[test]
    fn auto_key_off_allows_the_breath_when_a_bar_covers_it() {
        let mut state = TimelineState::default();
        state.set_current_breath(5);
        let blocks = vec![block("a", 0, 8)];
        assert_eq!(state.resolve_editable_breath(&blocks), Some(5));
    }

    #[test]
    fn auto_key_off_rejects_the_edit_when_the_playhead_sits_in_a_gap() {
        let mut state = TimelineState::default();
        state.set_current_breath(3);
        let blocks = vec![block("a", 0, 2), block("b", 5, 8)];
        assert_eq!(state.resolve_editable_breath(&blocks), None);
    }

    #[test]
    fn toggle_auto_key_flips_the_flag() {
        let mut state = TimelineState::default();
        state.toggle_auto_key();
        assert!(state.auto_key_enabled);
        state.toggle_auto_key();
        assert!(!state.auto_key_enabled);
    }
}
