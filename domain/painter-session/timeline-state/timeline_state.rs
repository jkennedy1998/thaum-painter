use crate::pieces::{covering_bar_cell, BreathBar, CellType};

/// Live, unsaved playhead and auto-key state for the painter session, plus
/// the playback session (play/pause and loop-wrap) that drives the playhead
/// across the document's loop window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelineState {
    pub current_breath: u32,
    pub auto_key_enabled: bool,
    pub playing: bool,
    pub loop_enabled: bool,
}

impl Default for TimelineState {
    fn default() -> Self {
        Self {
            current_breath: 0,
            auto_key_enabled: false,
            playing: false,
            // The old system defaulted the document to loop playback.
            loop_enabled: true,
        }
    }
}

impl TimelineState {
    pub fn set_current_breath(&mut self, breath: u32) {
        self.current_breath = breath;
    }

    pub fn toggle_auto_key(&mut self) {
        self.auto_key_enabled = !self.auto_key_enabled;
    }

    pub fn toggle_play(&mut self) {
        self.playing = !self.playing;
    }

    pub fn toggle_loop(&mut self) {
        self.loop_enabled = !self.loop_enabled;
    }

    /// Advances playback one breath. Starting from outside the window (or a
    /// past-the-end breath) snaps to the window start; past the end the
    /// playhead wraps to the start when looping, otherwise playback stops
    /// (returns `None` and clears `playing`). Returns the new breath when it
    /// moved, so the caller can resync the edit surface.
    pub fn advance_playback(&mut self, window_start: u32, window_end: u32) -> Option<u32> {
        if !self.playing {
            return None;
        }
        let window_end = window_end.max(window_start);
        if self.current_breath < window_start || self.current_breath >= window_end {
            self.current_breath = window_start;
            return Some(window_start);
        }
        let next = self.current_breath + 1;
        if next > window_end {
            if self.loop_enabled {
                self.current_breath = window_start;
                Some(window_start)
            } else {
                self.playing = false;
                None
            }
        } else {
            self.current_breath = next;
            Some(next)
        }
    }

    /// Resolves which breath a property edit at the current playhead is allowed to land on.
    /// Auto-key on always allows the current playhead. Auto-key off allows the playhead only
    /// when the covering bar is **solid** (the user is looking at a key); over an **empty**
    /// bar the edit is rejected — the user cannot submit a new key with auto-key off, and
    /// under binary tiling an empty bar covers the breath just like a solid one, so the old
    /// any-bar-covers check would wrongly admit it.
    pub fn resolve_editable_breath<B: BreathBar>(&self, blocks: &[B]) -> Option<u32> {
        if self.auto_key_enabled {
            return Some(self.current_breath);
        }
        covering_bar_cell(blocks, self.current_breath)
            .filter(|cell| cell.cell_type == CellType::Solid)
            .map(|_| self.current_breath)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::SharedDocumentPropertyBlock;


    fn bar(start_breath: u32, length_breaths: u32, is_blank: bool) -> SharedDocumentPropertyBlock {
        SharedDocumentPropertyBlock {
            id: format!("b{start_breath}-{length_breaths}"),
            start_breath,
            length_breaths,
            is_blank,
            value: None,
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
        assert_eq!(
            state.resolve_editable_breath(&[] as &[SharedDocumentPropertyBlock]),
            Some(42)
        );
    }

    #[test]
    fn auto_key_off_allows_the_breath_when_a_solid_bar_covers_it() {
        let mut state = TimelineState::default();
        state.set_current_breath(5);
        let blocks = vec![bar(0, 8, false)];
        assert_eq!(state.resolve_editable_breath(&blocks), Some(5));
    }

    #[test]
    fn auto_key_off_rejects_the_edit_when_the_playhead_sits_over_an_empty_bar() {
        // Binary tiling: an empty bar covers the breath, but the user cannot submit
        // a new key with auto-key off and is not looking at a key — reject.
        let mut state = TimelineState::default();
        state.set_current_breath(3);
        let blocks = vec![bar(0, 8, true)];
        assert_eq!(state.resolve_editable_breath(&blocks), None);
    }

    #[test]
    fn auto_key_on_allows_the_breath_over_an_empty_bar_too() {
        let mut state = TimelineState::default();
        state.auto_key_enabled = true;
        state.set_current_breath(3);
        let blocks = vec![bar(0, 8, true)];
        assert_eq!(state.resolve_editable_breath(&blocks), Some(3));
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
