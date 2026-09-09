//! Move-channel interpolation (J 2026-09-07): the first real per-property-channel
//! resolver. Interpolation is different per property — you cannot interpolate
//! raster like you interpolate move — so each channel gets its own module; this
//! one owns the move row only. Move keyframes are solid bars that hold one
//! `{x,y,z}` offset across their span; the empties between them resolve per the
//! empty's authored mode:
//!
//! - **hold** — carries the previous keyframe's offset (constant through the empty).
//! - **interpolate** — lerps from the previous keyframe's offset to the next
//!   keyframe's offset across the empty's span, with the empty's ease ends
//!   bending progress (ease-out slows the departure, ease-in slows the arrival).
//! - **loop_out / loop_in** — edge-locked modes (enforced in storage): repeat the
//!   authored region through the trailing blank / before the leading blank.
//!
//! First pass covers the move row only; raster resolves per its own channel
//! rules in `interp_raster` (J 2026-09-07).

use crate::interp_mode;
use crate::storage::{parse_move_offset, SharedDocumentPropertyBlock};

/// Resolves the move offset authored for `breath` across one property track's
/// blocks, or `None` when nothing resolves (no blocks, no values anywhere) —
/// the caller renders the layer unshifted. Breaths past the trailing blank's
/// stored extent resolve AS the trailing blank, so a loop-out keeps playing in
/// the infinite region instead of dropping to the zero offset.
pub fn resolve_move_offset(
    blocks: &[SharedDocumentPropertyBlock],
    breath: u32,
) -> Option<[i32; 3]> {
    let index = blocks
        .iter()
        .position(|block| {
            crate::properties::breath_in_span(breath, block.start_breath, block.length_breaths)
        })
        .or_else(|| {
            // Past the stored extent: the trailing blank covers to infinity.
            blocks
                .last()
                .filter(|block| block.is_blank)
                .map(|_| blocks.len() - 1)
        })?;
    resolve_block(blocks, index, breath)
}

/// Resolves one block at `breath` (the breath is always inside the block's span
/// or mapped into it by the loop resolver, which cannot re-enter a loop blank).
fn resolve_block(
    blocks: &[SharedDocumentPropertyBlock],
    index: usize,
    breath: u32,
) -> Option<[i32; 3]> {
    let block = &blocks[index];
    if !block.is_blank {
        return block.value.as_ref().and_then(parse_move_offset);
    }
    match interp_mode::resolve_mode(block.interpretation.as_deref()) {
        "loop_out" => resolve_loop(blocks, index, breath, false),
        "loop_in" => resolve_loop(blocks, index, breath, true),
        "hold" => resolve_hold(blocks, index),
        _ => resolve_interpolate(blocks, index, breath),
    }
}

/// Hold: the nearest solid keyframe to the LEFT holds its offset through the
/// empty. With nothing behind (the empty starts the track) the nearest solid to
/// the right holds instead; with no content at all nothing resolves.
fn resolve_hold(blocks: &[SharedDocumentPropertyBlock], index: usize) -> Option<[i32; 3]> {
    solid_value_before(blocks, index).or_else(|| solid_value_after(blocks, index))
}

/// Interpolate: lerp from the previous keyframe's offset to the next keyframe's
/// offset across the empty's span, progress bent by the empty's ease ends.
/// A missing side degrades to holding the existing side's offset.
fn resolve_interpolate(
    blocks: &[SharedDocumentPropertyBlock],
    index: usize,
    breath: u32,
) -> Option<[i32; 3]> {
    let previous = solid_value_before(blocks, index);
    let next = solid_value_after(blocks, index);
    match (previous, next) {
        (Some(from), Some(to)) => {
            let progress = empty_progress(&blocks[index], breath);
            Some(lerp_offset(from, to, progress))
        }
        (only, None) => only,
        (None, only) => only,
    }
}

/// Loop out / loop in: repeat the authored region (first keyframe start through
/// last keyframe end) through the edge blank. Loop out plays forward from the
/// region's start; loop in mirrors backward from the region's end, so the
/// breath immediately before the first keyframe resolves to the region's last
/// value (the After Effects loop-in behavior). The mapped breath always lands
/// inside the authored region — never back in the loop blank — so the recursion
/// is depth-one.
fn resolve_loop(
    blocks: &[SharedDocumentPropertyBlock],
    index: usize,
    breath: u32,
    mirror: bool,
) -> Option<[i32; 3]> {
    let blank = &blocks[index];
    let first_solid = blocks.iter().position(|block| !block.is_blank)?;
    let last_solid = blocks.iter().rposition(|block| !block.is_blank)?;
    let region_start = blocks[first_solid].start_breath;
    let region_end = blocks[last_solid].start_breath + blocks[last_solid].length_breaths;
    let loop_len = region_end.saturating_sub(region_start);
    if loop_len == 0 {
        return None;
    }
    let mapped = if mirror {
        let blank_end = blank.start_breath + blank.length_breaths;
        let distance = blank_end.saturating_sub(1).saturating_sub(breath);
        region_end - 1 - (distance % loop_len)
    } else {
        let distance = breath.saturating_sub(blank.start_breath);
        region_start + (distance % loop_len)
    };
    let mapped_index = blocks.iter().position(|block| {
        crate::properties::breath_in_span(mapped, block.start_breath, block.length_breaths)
    })?;
    resolve_block(blocks, mapped_index, mapped)
}

/// The nearest solid keyframe entirely left of `index`, if any. A valueless
/// solid resolves as the identity keyframe `[0, 0, 0]`, not "no keyframe":
/// the born-tiled placeholder (and split remainders of it) renders unshifted
/// at every breath it covers, so it IS the unmoved position any interpolating
/// empty beside it should lerp from (J 2026-09-09 live repro: a placeholder
/// left of a real keyframe made the empty between them degrade to hold-next
/// and the move track never animated). Returning `None` here is reserved for
/// genuinely no solid on that side.
fn solid_value_before(blocks: &[SharedDocumentPropertyBlock], index: usize) -> Option<[i32; 3]> {
    blocks[..index]
        .iter()
        .rev()
        .find(|block| !block.is_blank)
        .map(solid_keyframe_value)
}

/// Mirror of `solid_value_before` for the right side.
fn solid_value_after(blocks: &[SharedDocumentPropertyBlock], index: usize) -> Option<[i32; 3]> {
    blocks[index + 1..]
        .iter()
        .find(|block| !block.is_blank)
        .map(solid_keyframe_value)
}

/// One solid block's interpolation keyframe value: its parsed offset, or the
/// identity for a valueless solid (same resolution the solid itself gets at
/// render time — unshifted).
fn solid_keyframe_value(block: &SharedDocumentPropertyBlock) -> [i32; 3] {
    block
        .value
        .as_ref()
        .and_then(parse_move_offset)
        .unwrap_or([0, 0, 0])
}

/// Eased progress across one empty's span at `breath` (0..=1). Keyframes hold
/// constant across their own spans, so the empty's span is the whole
/// transition: its first breath has just left the previous keyframe, and the
/// breath after the empty lands on the next one exactly. Shared with the
/// raster channel's interpolator so both channels bend identically.
pub(crate) fn empty_progress(blank: &SharedDocumentPropertyBlock, breath: u32) -> f32 {
    let length = blank.length_breaths.max(1) as f32;
    let raw = (breath - blank.start_breath) as f32;
    let t = ((raw + 1.0) / (length + 1.0)).clamp(0.0, 1.0);
    eased_progress(t, blank.ease_out_percent, blank.ease_in_percent)
}

/// The ease model (J 2026-09-07, four authored strength steps 0/33/66/100%):
/// ease-out strength `o` slows the departure from the previous keyframe
/// (`t^(1+2o)`), ease-in strength `i` slows the arrival at the next keyframe
/// (`1-(1-x)^(1+2i)`), composed so 0% on both ends is exactly linear. The
/// composition stays monotonic for every strength combination.
fn eased_progress(t: f32, ease_out_percent: Option<u8>, ease_in_percent: Option<u8>) -> f32 {
    let out = ease_out_percent.unwrap_or(0).min(100) as f32 / 100.0;
    let into = ease_in_percent.unwrap_or(0).min(100) as f32 / 100.0;
    let departed = t.powf(1.0 + 2.0 * out);
    1.0 - (1.0 - departed).powf(1.0 + 2.0 * into)
}

fn lerp_offset(from: [i32; 3], to: [i32; 3], progress: f32) -> [i32; 3] {
    [
        from[0] + ((to[0] - from[0]) as f32 * progress).round() as i32,
        from[1] + ((to[1] - from[1]) as f32 * progress).round() as i32,
        from[2] + ((to[2] - from[2]) as f32 * progress).round() as i32,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn solid(id: &str, start: u32, length: u32, offset: [i32; 3]) -> SharedDocumentPropertyBlock {
        SharedDocumentPropertyBlock {
            id: id.to_string(),
            start_breath: start,
            length_breaths: length,
            is_blank: false,
            value: Some(json!({ "x": offset[0], "y": offset[1], "z": offset[2] })),
            interpretation: None,
            ease_out_percent: None,
            ease_in_percent: None,
        }
    }

    fn blank(
        id: &str,
        start: u32,
        length: u32,
        interpretation: Option<&str>,
        ease_out: Option<u8>,
        ease_in: Option<u8>,
    ) -> SharedDocumentPropertyBlock {
        SharedDocumentPropertyBlock {
            id: id.to_string(),
            start_breath: start,
            length_breaths: length,
            is_blank: true,
            value: None,
            interpretation: interpretation.map(str::to_string),
            ease_out_percent: ease_out,
            ease_in_percent: ease_in,
        }
    }

    fn valueless_solid(id: &str, start: u32, length: u32) -> SharedDocumentPropertyBlock {
        SharedDocumentPropertyBlock {
            id: id.to_string(),
            start_breath: start,
            length_breaths: length,
            is_blank: false,
            value: None,
            interpretation: None,
            ease_out_percent: None,
            ease_in_percent: None,
        }
    }

    #[test]
    fn a_solid_block_holds_its_offset_across_its_whole_span() {
        let blocks = vec![
            solid("a", 0, 5, [3, 0, 0]),
            blank("tail", 5, 1, None, None, None),
        ];
        assert_eq!(resolve_move_offset(&blocks, 0), Some([3, 0, 0]));
        assert_eq!(resolve_move_offset(&blocks, 4), Some([3, 0, 0]));
    }

    #[test]
    fn a_default_empty_resolves_interpolate_linearly() {
        let blocks = vec![
            solid("a", 0, 4, [0, 0, 0]),
            blank("b", 4, 4, None, None, None),
            solid("c", 8, 4, [8, 0, 0]),
            blank("tail", 12, 1, None, None, None),
        ];
        // The empty's four breaths walk 0 -> 8 in five equal steps (first breath
        // has just left the previous keyframe; the breath after the empty lands
        // on it exactly): 1.6, 3.2, 4.8, 6.4.
        assert_eq!(resolve_move_offset(&blocks, 4), Some([2, 0, 0]));
        assert_eq!(resolve_move_offset(&blocks, 5), Some([3, 0, 0]));
        assert_eq!(resolve_move_offset(&blocks, 7), Some([6, 0, 0]));
        assert_eq!(resolve_move_offset(&blocks, 8), Some([8, 0, 0]));
    }

    #[test]
    fn hold_carries_the_previous_keyframe_through_the_empty() {
        let blocks = vec![
            solid("a", 0, 4, [0, 0, 0]),
            blank("b", 4, 4, Some("hold"), None, None),
            solid("c", 8, 4, [8, 0, 0]),
            blank("tail", 12, 1, None, None, None),
        ];
        assert_eq!(resolve_move_offset(&blocks, 4), Some([0, 0, 0]));
        assert_eq!(resolve_move_offset(&blocks, 7), Some([0, 0, 0]));
        assert_eq!(resolve_move_offset(&blocks, 8), Some([8, 0, 0]));
    }

    #[test]
    fn ease_strengths_bend_interpolate_progress() {
        // Full ease-out: the first breath barely leaves the previous keyframe.
        let blocks = vec![
            solid("a", 0, 4, [0, 0, 0]),
            blank("b", 4, 4, Some("interpolate"), Some(100), None),
            solid("c", 8, 4, [8, 0, 0]),
            blank("tail", 12, 1, None, None, None),
        ];
        let eased = resolve_move_offset(&blocks, 4).unwrap()[0];
        let linear = 2;
        assert!(
            eased < linear,
            "ease-out must start slower than linear ({eased} < {linear})"
        );
        // Full ease-in: the value hugs the target early and crawls the last bit
        // (velocity -> 0 at arrival), so the last empty breath is AHEAD of linear.
        let blocks = vec![
            solid("a", 0, 4, [0, 0, 0]),
            blank("b", 4, 4, Some("interpolate"), None, Some(100)),
            solid("c", 8, 4, [8, 0, 0]),
            blank("tail", 12, 1, None, None, None),
        ];
        let eased = resolve_move_offset(&blocks, 7).unwrap()[0];
        assert!(
            eased > 6,
            "ease-in must decelerate into the target ({eased} > 6)"
        );
    }

    #[test]
    fn interpolate_with_a_missing_side_holds_the_existing_side() {
        let blocks = vec![
            blank("lead", 0, 2, None, None, None),
            solid("c", 2, 4, [6, 0, 0]),
            blank("tail", 6, 1, None, None, None),
        ];
        assert_eq!(resolve_move_offset(&blocks, 0), Some([6, 0, 0]));
        assert_eq!(resolve_move_offset(&blocks, 1), Some([6, 0, 0]));
        let blocks = vec![
            solid("a", 0, 4, [2, 0, 0]),
            blank("tail", 4, 1, None, None, None),
        ];
        assert_eq!(resolve_move_offset(&blocks, 4), Some([2, 0, 0]));
    }

    #[test]
    fn loop_out_repeats_the_authored_region_through_the_trailing_blank() {
        let blocks = vec![
            solid("a", 0, 4, [0, 0, 0]),
            blank("b", 4, 4, Some("hold"), None, None),
            solid("c", 8, 4, [4, 0, 0]),
            blank("tail", 12, 1, Some("loop_out"), None, None),
        ];
        // Authored region is 0..12 (12 breaths): solid 0 (0..4), hold blank
        // 4..8, solid 4 (8..12). Wrapping maps breath 12 -> 0, 16..19 -> the
        // hold blank (0), 20..23 -> solid 4, 24 -> wraps to 0 again.
        assert_eq!(resolve_move_offset(&blocks, 12), Some([0, 0, 0]));
        assert_eq!(resolve_move_offset(&blocks, 16), Some([0, 0, 0]));
        assert_eq!(resolve_move_offset(&blocks, 19), Some([0, 0, 0]));
        assert_eq!(resolve_move_offset(&blocks, 20), Some([4, 0, 0]));
        assert_eq!(resolve_move_offset(&blocks, 23), Some([4, 0, 0]));
        // Far past the stored extent, the loop keeps playing.
        assert_eq!(resolve_move_offset(&blocks, 24), Some([0, 0, 0]));
    }

    #[test]
    fn loop_in_mirrors_backward_from_the_leading_blank() {
        let blocks = vec![
            blank("lead", 0, 2, Some("loop_in"), None, None),
            solid("a", 2, 4, [0, 0, 0]),
            blank("b", 6, 4, Some("hold"), None, None),
            solid("c", 10, 4, [4, 0, 0]),
            blank("tail", 14, 1, None, None, None),
        ];
        // Authored region is 2..14 (12 breaths). The breath right before the
        // first keyframe resolves to the region's LAST value (the After Effects
        // loop-in behavior), and walking left replays the region backward.
        assert_eq!(resolve_move_offset(&blocks, 1), Some([4, 0, 0]));
        assert_eq!(resolve_move_offset(&blocks, 0), Some([4, 0, 0]));
    }

    #[test]
    fn a_track_with_no_values_resolves_nothing() {
        let blocks = vec![blank("tail", 0, 24, None, None, None)];
        assert_eq!(resolve_move_offset(&blocks, 5), None);
        assert_eq!(resolve_move_offset(&[], 0), None);
    }

    #[test]
    fn a_placeholder_solid_is_the_identity_keyframe_beside_an_interpolating_empty() {
        // J 2026-09-09 live repro: the born-tiled placeholder (or a split
        // remainder of it) left of a real keyframe used to make the empty
        // between them degrade to hold-next — the track never animated.
        // Track shape straight from the run log:
        // 0..6/vNone 6..17/e 17..24/vSome({0,-3,-3}) 24..25/e
        let blocks = vec![
            valueless_solid("a", 0, 6),
            blank("b", 6, 11, None, None, None),
            solid("c", 17, 7, [0, -3, -3]),
            blank("tail", 24, 1, None, None, None),
        ];
        // The placeholder itself renders unshifted (identity).
        assert_eq!(resolve_move_offset(&blocks, 0), None);
        assert_eq!(resolve_move_offset(&blocks, 5), None);
        // The interpolating empty lerps identity -> {0,-3,-3}: real motion.
        // Progress at breath 6 = 1/12 (linear), at breath 16 = 11/12.
        assert_eq!(resolve_move_offset(&blocks, 6), Some([0, 0, 0]));
        assert_eq!(resolve_move_offset(&blocks, 11), Some([0, -2, -2]));
        assert_eq!(resolve_move_offset(&blocks, 16), Some([0, -3, -3]));
        // The keyframe and its trailing empty hold as before.
        assert_eq!(resolve_move_offset(&blocks, 20), Some([0, -3, -3]));
        assert_eq!(resolve_move_offset(&blocks, 24), Some([0, -3, -3]));
    }
}
