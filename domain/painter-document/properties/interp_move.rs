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

/// Axis stagger (J 2026-09-10): on a low-resolution raster grid, simultaneous
/// multi-axis steps read as jumpy diagonal leaps — equal deltas give the axes
/// identical rounding thresholds, so they always round across together. Each
/// axis's progress is therefore evaluated a hair later in sub-breath time
/// (x, then y, then z), which de-coincides the thresholds: unit axis-by-axis
/// steps become the default without barring genuine diagonal moves. 1/8 of a
/// breath ≈ one display frame at 60Hz against the 125ms breath — large
/// enough to split coincident steps across frames, far below timing
/// perception. Pure function of breath: no state, so scrubbing, multiplayer
/// determinism, and the fractional-playback seam are unaffected. All axes
/// clamp back together at t = 1, so arrival on the next keyframe stays exact.
pub(crate) const AXIS_STAGGER_BREATHS: f32 = 1.0 / 8.0;

/// Resolves the move offset authored for `breath` across one property track's
/// blocks, or `None` when nothing resolves (no blocks, no values anywhere) —
/// the caller renders the layer unshifted. Breaths past the trailing blank's
/// stored extent resolve AS the trailing blank, so a loop-out keeps playing in
/// the infinite region instead of dropping to the zero offset.
pub fn resolve_move_offset(
    blocks: &[SharedDocumentPropertyBlock],
    breath: u32,
) -> Option<[i32; 3]> {
    resolve_move_offset_fractional(blocks, breath, 0.0)
}

/// Fractional-breath resolution (J 2026-09-10): playback samples the eased
/// curve BETWEEN breaths — `fraction` (0..1) is the elapsed portion of the
/// current breath — so an interpolating empty renders one offset per display
/// frame instead of one per breath. Positions still snap to the integer grid
/// (no AA); the perceived smoothness comes from unequal per-cell dwell times,
/// which is how coarse-grid easing reads as a parabolic speed graph. Solids
/// hold across their spans regardless of fraction; scrub/edit paths pass 0.0
/// and resolve identically to `resolve_move_offset`.
pub fn resolve_move_offset_fractional(
    blocks: &[SharedDocumentPropertyBlock],
    breath: u32,
    fraction: f32,
) -> Option<[i32; 3]> {
    // Strictly below 1.0: the fraction belongs to THIS breath — at exactly 1.0
    // a loop blank's mirror distance goes negative and the wrap lookup can
    // miss, and the next frame's tick re-anchors at the next breath anyway.
    let fraction = fraction.clamp(0.0, 0.999);
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
    resolve_block(blocks, index, breath as f32 + fraction)
}

/// Resolves one block at `breath` (the breath is always inside the block's span
/// or mapped into it by the loop resolver, which cannot re-enter a loop blank).
fn resolve_block(
    blocks: &[SharedDocumentPropertyBlock],
    index: usize,
    breath: f32,
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
    breath: f32,
) -> Option<[i32; 3]> {
    let previous = solid_value_before(blocks, index);
    let next = solid_value_after(blocks, index);
    match (previous, next) {
        (Some(from), Some(to)) => {
            let progresses = staggered_axis_progresses(&blocks[index], breath);
            Some(lerp_offset(from, to, progresses))
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
    breath: f32,
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
    // The playhead's fractional part carries through the wrap, so a loop edge
    // also plays smoothly between breaths. The integer part drives the block
    // lookup; the fraction rides along into the mapped block's resolver.
    let mapped = if mirror {
        let blank_end = (blank.start_breath + blank.length_breaths) as f32;
        let distance = (blank_end - 1.0) - breath;
        region_end as f32 - 1.0 - (distance % loop_len as f32)
    } else {
        let distance = breath - blank.start_breath as f32;
        region_start as f32 + (distance % loop_len as f32)
    };
    let mapped_breath = mapped.floor().max(0.0) as u32;
    let mapped_index = blocks.iter().position(|block| {
        crate::properties::breath_in_span(mapped_breath, block.start_breath, block.length_breaths)
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

/// Eased progress across one empty's span at a whole breath (0..=1). The
/// RASTER channel's path: it keeps the original power ease and whole-breath
/// sampling — its blend behavior (and the unfinished smear development on
/// top of it) is intentionally untouched by the move channel's playback
/// work (J 2026-09-10). Keyframes hold constant across their own spans, so
/// the empty's span is the whole transition: its first breath has just left
/// the previous keyframe, and the breath after the empty lands on the next
/// one exactly.
pub(crate) fn empty_progress(blank: &SharedDocumentPropertyBlock, breath: u32) -> f32 {
    let length = blank.length_breaths.max(1) as f32;
    let raw = (breath - blank.start_breath) as f32;
    let t = ((raw + 1.0) / (length + 1.0)).clamp(0.0, 1.0);
    power_eased_progress(t, blank.ease_out_percent, blank.ease_in_percent)
}

/// Per-axis eased progress across one empty's span at `breath`: the shared
/// time `t` (fractional during playback, whole otherwise) evaluated once per
/// axis with the axis's stagger added — see `AXIS_STAGGER_BREATHS`. Clamped
/// to 1.0 so all axes converge on the next keyframe exactly.
fn staggered_axis_progresses(blank: &SharedDocumentPropertyBlock, breath: f32) -> [f32; 3] {
    let length = blank.length_breaths.max(1) as f32;
    let raw = breath - blank.start_breath as f32;
    let t = ((raw + 1.0) / (length + 1.0)).clamp(0.0, 1.0);
    let mut progresses = [0.0f32; 3];
    for (axis, progress) in progresses.iter_mut().enumerate() {
        let shifted = (t + axis as f32 * AXIS_STAGGER_BREATHS).min(1.0);
        *progress = bezier_eased_progress(shifted, blank.ease_out_percent, blank.ease_in_percent);
    }
    progresses
}

/// The RASTER channel's ease model (J 2026-09-07, four authored strength
/// steps 0/33/66/100%): ease-out strength `o` slows the departure from the
/// previous keyframe (`t^(1+2o)`), ease-in strength `i` slows the arrival at
/// the next keyframe (`1-(1-x)^(1+2i)`), composed so 0% on both ends is
/// exactly linear. Kept as-is for raster/smear; the move channel moved to
/// the AE-style bezier below because this composition COMPOUNDS the two ends
/// (the ease-in reshapes the already-eased ease-out result), which back-loads
/// the motion — at both-max the time midpoint reached only 33% of the
/// distance, reading as a double-slow start and a fast arrival.
fn power_eased_progress(t: f32, ease_out_percent: Option<u8>, ease_in_percent: Option<u8>) -> f32 {
    let out = ease_out_percent.unwrap_or(0).min(100) as f32 / 100.0;
    let into = ease_in_percent.unwrap_or(0).min(100) as f32 / 100.0;
    let departed = t.powf(1.0 + 2.0 * out);
    1.0 - (1.0 - departed).powf(1.0 + 2.0 * into)
}

/// The MOVE channel's ease model (J 2026-09-10): one cubic bezier per
/// transition, the After Effects temporal-ease shape (Adobe: Easy Ease gives
/// each keyframe speed 0 with 33.33% influence — influence is how much of
/// the segment's TIME the speed-0 handle spans). The outgoing handle leaves
/// the previous keyframe at speed 0 spanning `out` of the empty's time;
/// the incoming handle arrives at speed 0 spanning `in` back from the next
/// keyframe. The two ends share ONE curve instead of compounding, so equal
/// strengths bend evenly around the midpoint (f(0.5) = 0.5 exactly, for
/// every equal-strength pairing) — the fix for the max-eased end reading as
/// faster than the max-eased start. 33% reads as AE's Easy Ease; 0/0 is
/// exactly linear; monotonic for every strength combination.
fn bezier_eased_progress(t: f32, ease_out_percent: Option<u8>, ease_in_percent: Option<u8>) -> f32 {
    let out = ease_out_percent.unwrap_or(0).min(100) as f32 / 100.0;
    let into = ease_in_percent.unwrap_or(0).min(100) as f32 / 100.0;
    if out == 0.0 && into == 0.0 {
        return t;
    }
    cubic_bezier_ease(t, out, 1.0 - into)
}

/// Evaluates the ease bezier at normalized time `t`: control points
/// P0=(0,0), P1=(x1,0), P2=(x2,1), P3=(1,1) — y handles pinned to the ends
/// (speed 0 at both keyframes), x handles carry the influence strengths.
/// Solves the x parameterization for `t` with Newton-Raphson and a bisection
/// fallback (the standard CSS/Chromium approach; x1, x2 are in [0,1] so x is
/// monotonic and a solution always exists — bisection covers the flat-x'
/// spots near both-max where Newton can stall).
fn cubic_bezier_ease(t: f32, x1: f32, x2: f32) -> f32 {
    let x_at = |b: f32| {
        let u = 1.0 - b;
        3.0 * u * u * b * x1 + 3.0 * u * b * b * x2 + b * b * b
    };
    let mut b = t;
    for _ in 0..8 {
        let u = 1.0 - b;
        let x = x_at(b) - t;
        let dx = 3.0 * u * u * x1 + 6.0 * u * b * (x2 - x1) + 3.0 * b * b * (1.0 - x2);
        if dx.abs() < 1e-6 {
            break;
        }
        b -= x / dx;
        b = b.clamp(0.0, 1.0);
    }
    if (x_at(b) - t).abs() > 1e-4 {
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        for _ in 0..32 {
            let mid = 0.5 * (lo + hi);
            if x_at(mid) < t {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        b = 0.5 * (lo + hi);
    }
    // Y(b) with P1y=0, P2y=1: 3u²b·0 + 3ub²·1 + b³.
    let u = 1.0 - b;
    3.0 * u * b * b + b * b * b
}

fn lerp_offset(from: [i32; 3], to: [i32; 3], progresses: [f32; 3]) -> [i32; 3] {
    [
        from[0] + ((to[0] - from[0]) as f32 * progresses[0]).round() as i32,
        from[1] + ((to[1] - from[1]) as f32 * progresses[1]).round() as i32,
        from[2] + ((to[2] - from[2]) as f32 * progresses[2]).round() as i32,
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
    fn equal_strengths_bend_evenly_around_the_midpoint() {
        // The compounding-bias fix (J 2026-09-10): the old power composition
        // back-loaded equal-strength eases (both-max reached only 33% of the
        // distance at the time midpoint). The bezier shares one curve between
        // the ends, so equal strengths pass exactly 0.5 at t = 0.5 and mirror
        // around it.
        for (out, into) in [(33u8, 33u8), (66, 66), (100, 100)] {
            let mid = bezier_eased_progress(0.5, Some(out), Some(into));
            assert!(
                (mid - 0.5).abs() < 1e-4,
                "out={out} in={into}: midpoint {mid} must be 0.5"
            );
            for t in [0.1f32, 0.25, 0.4, 0.6, 0.75, 0.9] {
                let forward = bezier_eased_progress(t, Some(out), Some(into));
                let mirrored = 1.0 - bezier_eased_progress(1.0 - t, Some(out), Some(into));
                assert!(
                    (forward - mirrored).abs() < 1e-3,
                    "out={out} in={into}: f({t})={forward} must mirror {mirrored}"
                );
            }
        }
    }

    #[test]
    fn the_ease_curve_is_exactly_linear_at_zero_strength_and_holds_endpoints() {
        for t in [0.0f32, 0.1, 0.37, 0.5, 0.83, 1.0] {
            let linear = bezier_eased_progress(t, None, None);
            assert!((linear - t).abs() < 1e-6, "0/0 at {t} gave {linear}");
        }
        for (out, into) in [(100u8, 100u8), (33, 66), (66, 0)] {
            assert_eq!(bezier_eased_progress(0.0, Some(out), Some(into)), 0.0);
            assert_eq!(bezier_eased_progress(1.0, Some(out), Some(into)), 1.0);
        }
    }

    #[test]
    fn the_ease_curve_is_monotonic_for_every_strength_combination() {
        for out in [0u8, 33, 66, 100] {
            for into in [0u8, 33, 66, 100] {
                let mut previous = -1.0f32;
                for step in 0..=200u32 {
                    let t = step as f32 / 200.0;
                    let value = bezier_eased_progress(t, Some(out), Some(into));
                    assert!(
                        value >= previous - 1e-5,
                        "out={out} in={into}: regressed at t={t} ({value} < {previous})"
                    );
                    previous = value;
                }
            }
        }
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
    fn fractional_breath_zero_matches_the_integer_result() {
        let blocks = vec![
            solid("a", 0, 4, [0, 0, 0]),
            blank("b", 4, 4, Some("interpolate"), Some(100), Some(100)),
            solid("c", 8, 4, [8, 0, 0]),
            blank("tail", 12, 1, None, None, None),
        ];
        for breath in 0..13u32 {
            assert_eq!(
                resolve_move_offset(&blocks, breath),
                resolve_move_offset_fractional(&blocks, breath, 0.0),
                "fraction 0 at breath {breath} must be the whole-breath result"
            );
        }
    }

    #[test]
    fn fractional_breath_samples_the_ease_between_breaths() {
        // Full ease-out over a 4-breath empty (values calibrated to the move
        // channel's AE-style bezier). Whole-breath samples land on offsets 0,
        // 1, 1, 3; the mid-breath sample lands on 2 — a distinct position no
        // whole breath ever shows, which is where the smoother motion comes
        // from.
        let blocks = vec![
            solid("a", 0, 4, [0, 0, 0]),
            blank("b", 4, 4, Some("interpolate"), Some(100), None),
            solid("c", 8, 4, [8, 0, 0]),
            blank("tail", 12, 1, None, None, None),
        ];
        assert_eq!(resolve_move_offset(&blocks, 6).unwrap()[0], 1);
        assert_eq!(
            resolve_move_offset_fractional(&blocks, 6, 0.5).unwrap()[0],
            2
        );
        // The empty's final fraction lands exactly on the next keyframe, so
        // playback is continuous across the empty/keyframe boundary.
        assert_eq!(
            resolve_move_offset_fractional(&blocks, 7, 0.999).unwrap()[0],
            8
        );
        // Monotonic across the sampled fraction of one breath.
        let at = |fraction: f32| {
            resolve_move_offset_fractional(&blocks, 5, fraction).unwrap()[0]
        };
        assert!(at(0.0) <= at(0.25) && at(0.25) <= at(0.5) && at(0.5) <= at(0.75));
    }

    #[test]
    fn fractional_breath_is_ignored_on_solids_hold_and_loop() {
        // Solids hold their offset across their spans at any fraction.
        let blocks = vec![
            solid("a", 0, 4, [3, 0, 0]),
            blank("b", 4, 4, Some("hold"), None, None),
            solid("c", 8, 4, [4, 0, 0]),
            blank("tail", 12, 1, Some("loop_out"), None, None),
        ];
        assert_eq!(
            resolve_move_offset_fractional(&blocks, 2, 0.5),
            Some([3, 0, 0])
        );
        assert_eq!(
            resolve_move_offset_fractional(&blocks, 6, 0.5),
            Some([3, 0, 0])
        );
        // A loop blank maps the fraction through the wrap: breath 12.5 plays
        // the region halfway between its breath-0 and breath-1 values.
        let looped = resolve_move_offset_fractional(&blocks, 12, 0.5).unwrap();
        assert!(looped[0] >= 3 && looped[0] <= 4, "mapped inside the region");
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
    fn linear_equal_axis_deltas_step_axis_by_axis_not_diagonally() {
        // The stagger's whole point (J 2026-09-10): equal x/y deltas used to
        // share identical rounding thresholds, so every visible step was a
        // 2-cell diagonal leap. With the stagger, a linear (3,3) move over a
        // 6-breath empty alternates single-axis unit steps — y leads because
        // x is axis 0 and gets no shift.
        let blocks = vec![
            solid("a", 0, 2, [0, 0, 0]),
            blank("b", 2, 6, None, None, None),
            solid("c", 8, 2, [3, 3, 0]),
            blank("tail", 10, 1, None, None, None),
        ];
        assert_eq!(resolve_move_offset(&blocks, 2), Some([0, 1, 0]));
        assert_eq!(resolve_move_offset(&blocks, 3), Some([1, 1, 0]));
        assert_eq!(resolve_move_offset(&blocks, 4), Some([1, 2, 0]));
        assert_eq!(resolve_move_offset(&blocks, 5), Some([2, 2, 0]));
        assert_eq!(resolve_move_offset(&blocks, 6), Some([2, 3, 0]));
        assert_eq!(resolve_move_offset(&blocks, 7), Some([3, 3, 0]));
        // Arrival on the keyframe is exact.
        assert_eq!(resolve_move_offset(&blocks, 8), Some([3, 3, 0]));
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
        // Progress at breath 6 = 1/12 (linear), at breath 16 = 11/12. The
        // axis stagger (J 2026-09-10) shifts y/z a hair later, so breath 6's
        // y/z cross their first rounding threshold one step earlier than the
        // un-staggered result — breaths 11/16 still land on the same cells.
        assert_eq!(resolve_move_offset(&blocks, 6), Some([0, -1, -1]));
        assert_eq!(resolve_move_offset(&blocks, 11), Some([0, -2, -2]));
        assert_eq!(resolve_move_offset(&blocks, 16), Some([0, -3, -3]));
        // The keyframe and its trailing empty hold as before.
        assert_eq!(resolve_move_offset(&blocks, 20), Some([0, -3, -3]));
        assert_eq!(resolve_move_offset(&blocks, 24), Some([0, -3, -3]));
    }
}
