//! Empty-bar keyframe axes (J 2026-09-07): every empty property block carries
//! three authored values — the interpolation mode (what happens between the
//! surrounding keyframes) and the ease-out / ease-in strengths that modulate
//! adjustable modes. Stored on `SharedDocumentPropertyBlock`: `interpretation`
//! holds the mode, `ease_out_percent` / `ease_in_percent` hold the ends. This
//! module is the single vocabulary: cycle order, edge locking, adjustability,
//! and glyphs. Resolution is per property channel — the move row resolves for
//! real in `interp_move`; the raster channel in `interp_raster` (blend color
//! and weight, hard-cutoff the discrete graphic at the halfway crossing).

/// The interpolation modes an empty cycles through, in cycle order.
pub const INTERP_MODES: [&str; 4] = ["interpolate", "hold", "loop_out", "loop_in"];

/// Ease strength steps, in cycle order. 0% reads as linear ease.
pub const EASE_STRENGTHS: [u8; 4] = [0, 33, 66, 100];

/// The mode an empty reads as when nothing is stored.
pub const DEFAULT_MODE: &str = "interpolate";

/// Normalizes the stored interpretation slot into a mode name. Unknown or
/// missing values resolve to the default rather than failing — the slot is a
/// forward-compatible seam, not a closed enum on disk.
pub fn resolve_mode(interpretation: Option<&str>) -> &'static str {
    match interpretation {
        Some(mode) => INTERP_MODES
            .iter()
            .find(|candidate| **candidate == mode)
            .copied()
            .unwrap_or(DEFAULT_MODE),
        None => DEFAULT_MODE,
    }
}

/// Whether a mode uses the ease ends. Hold, loop in, and loop out do not —
/// their ends render as the fixed non-adjustable glyphs and cycling onto them
/// clears both stored ease strengths (the ends are not utilizable there).
pub fn mode_is_ease_adjustable(mode: &str) -> bool {
    mode == "interpolate"
}

/// Whether a mode is allowed on an empty at a given track position (J
/// 2026-09-07): `loop_out` only on the track's LAST blank — the trailing blank,
/// the right edge of time — and `loop_in` only on the FIRST blank — the leading
/// blank, the left edge. Interpolate and hold are allowed anywhere. A track
/// whose first and last block are the same blank allows both.
pub fn mode_allowed_at(mode: &str, is_first: bool, is_last: bool) -> bool {
    match mode {
        "loop_out" => is_last,
        "loop_in" => is_first,
        _ => true,
    }
}

/// The next mode in the cycle after the stored interpretation.
pub fn next_mode(interpretation: Option<&str>) -> &'static str {
    let current = resolve_mode(interpretation);
    let index = INTERP_MODES
        .iter()
        .position(|candidate| *candidate == current)
        .unwrap_or(0);
    INTERP_MODES[(index + 1) % INTERP_MODES.len()]
}

/// The next ease strength in the cycle (percent). Unset reads as 0%.
pub fn next_ease_percent(current: Option<u8>) -> u8 {
    let index = EASE_STRENGTHS
        .iter()
        .position(|candidate| *candidate == current.unwrap_or(0))
        .unwrap_or(0);
    EASE_STRENGTHS[(index + 1) % EASE_STRENGTHS.len()]
}

/// Center glyph for a mode (also the single-cell empty's glyph — a single
/// empty is all center, no separate ends to style).
pub fn mode_center_glyph(mode: &str) -> char {
    match mode {
        "hold" => '□',
        "loop_out" => '⟧',
        "loop_in" => '⟦',
        _ => '▣',
    }
}

/// Left-end (ease-out) glyph at the given strength. Unset/0% is the plain
/// dash — a linear ease needs no arrow.
pub fn ease_out_glyph(ease_out_percent: Option<u8>) -> char {
    match ease_out_percent.unwrap_or(0) {
        33 => '›',
        66 => '⟶',
        100 => '»',
        _ => '-',
    }
}

/// Right-end (ease-in) glyph at the given strength — the left-end mirror.
pub fn ease_in_glyph(ease_in_percent: Option<u8>) -> char {
    match ease_in_percent.unwrap_or(0) {
        33 => '‹',
        66 => '⟵',
        100 => '«',
        _ => '-',
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_or_missing_interpretation_resolves_to_the_default_mode() {
        assert_eq!(resolve_mode(None), "interpolate");
        assert_eq!(resolve_mode(Some("nonsense")), "interpolate");
    }

    #[test]
    fn modes_cycle_back_to_the_start() {
        assert_eq!(next_mode(None), "hold");
        assert_eq!(next_mode(Some("hold")), "loop_out");
        assert_eq!(next_mode(Some("loop_out")), "loop_in");
        assert_eq!(next_mode(Some("loop_in")), "interpolate");
        assert_eq!(next_mode(Some("interpolate")), "hold");
    }

    #[test]
    fn only_interpolate_uses_the_ease_ends() {
        assert!(mode_is_ease_adjustable("interpolate"));
        assert!(!mode_is_ease_adjustable("hold"));
        assert!(!mode_is_ease_adjustable("loop_out"));
        assert!(!mode_is_ease_adjustable("loop_in"));
    }

    #[test]
    fn loop_modes_are_locked_to_their_track_edges() {
        // loop_out lives on the right edge only; loop_in on the left edge only.
        assert!(!mode_allowed_at("loop_out", false, false));
        assert!(!mode_allowed_at("loop_out", true, false));
        assert!(mode_allowed_at("loop_out", false, true));
        assert!(!mode_allowed_at("loop_in", false, false));
        assert!(!mode_allowed_at("loop_in", false, true));
        assert!(mode_allowed_at("loop_in", true, false));
        // A lone blank is both edges at once.
        assert!(mode_allowed_at("loop_out", true, true));
        assert!(mode_allowed_at("loop_in", true, true));
        // Interpolate and hold are allowed anywhere.
        assert!(mode_allowed_at("interpolate", false, false));
        assert!(mode_allowed_at("hold", false, false));
    }

    #[test]
    fn ease_strengths_cycle_through_four_steps() {
        assert_eq!(next_ease_percent(None), 33);
        assert_eq!(next_ease_percent(Some(0)), 33);
        assert_eq!(next_ease_percent(Some(33)), 66);
        assert_eq!(next_ease_percent(Some(66)), 100);
        assert_eq!(next_ease_percent(Some(100)), 0);
    }

    #[test]
    fn mode_and_ease_glyphs_match_the_dictated_set() {
        assert_eq!(mode_center_glyph("hold"), '□');
        assert_eq!(mode_center_glyph("interpolate"), '▣');
        assert_eq!(mode_center_glyph("loop_out"), '⟧');
        assert_eq!(mode_center_glyph("loop_in"), '⟦');
        assert_eq!(ease_out_glyph(None), '-');
        assert_eq!(ease_out_glyph(Some(33)), '›');
        assert_eq!(ease_out_glyph(Some(66)), '⟶');
        assert_eq!(ease_out_glyph(Some(100)), '»');
        assert_eq!(ease_in_glyph(None), '-');
        assert_eq!(ease_in_glyph(Some(33)), '‹');
        assert_eq!(ease_in_glyph(Some(66)), '⟵');
        assert_eq!(ease_in_glyph(Some(100)), '«');
    }
}
