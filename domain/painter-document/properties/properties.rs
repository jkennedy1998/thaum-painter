use crate::manifest::PropertyBlock;

/// Finds the property block, if any, whose breath range covers `breath`.
pub fn block_covering_breath(blocks: &[PropertyBlock], breath: u32) -> Option<&PropertyBlock> {
    blocks
        .iter()
        .find(|block| breath >= block.start_breath && breath <= block.end_breath)
}

/// Exclusive end of a start+length breath span. Every block shape in the painter
/// (manifest `PropertyBlock`, stored `SharedDocumentPropertyBlock`, panel rows) resolves
/// its range through these span helpers so breath semantics stay defined in one place.
pub fn span_end_breath(start_breath: u32, length_breaths: u32) -> u32 {
    start_breath.saturating_add(length_breaths)
}

/// True when `breath` falls inside the start+length span (the span covers it).
pub fn breath_in_span(breath: u32, start_breath: u32, length_breaths: u32) -> bool {
    breath >= start_breath && breath < span_end_breath(start_breath, length_breaths)
}

/// Clamps a requested (start, length) span so it cannot cross any other block in its
/// property channel: no two blocks in one channel may ever share a breath. `original`
/// is the moving block's current span before the edit; `others` are the channel's
/// remaining (start, length) spans. Blocks fully left/right of the original set the
/// free window's bounds; a block straddling the original (only possible in
/// already-overlapping stored data) folds to whichever side of the original's midpoint
/// it sits on, so moving a block also unwedges previously overlapping data. The result
/// always fits inside the free window, preserving the requested length where possible.
pub fn clamped_breath_span(
    others: &[(u32, u32)],
    original: (u32, u32),
    requested: (u32, u32),
) -> (u32, u32) {
    let original_end = span_end_breath(original.0, original.1);
    let original_middle = original.0 + original.1 / 2;
    let mut left_limit = 0u32;
    let mut right_limit = u32::MAX;
    for &(start, length) in others {
        let end = span_end_breath(start, length);
        if end <= original.0 {
            left_limit = left_limit.max(end);
        } else if start >= original_end || original_middle < start + length / 2 {
            right_limit = right_limit.min(start);
        } else {
            left_limit = left_limit.max(end);
        }
    }
    let length = requested
        .1
        .max(1)
        .min(right_limit.saturating_sub(left_limit));
    let start = requested.0.clamp(left_limit, right_limit - length);
    (start, length)
}

/// Result of a push (time-preserving) span edit: the edited block's new span plus the
/// spans of the other blocks that rippled sideways to keep the channel gap-free.
/// Indices refer to positions in the `others` slice passed to `pushed_breath_span`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PushedBreathSpan {
    pub edited: (u32, u32),
    pub shifted: Vec<(usize, (u32, u32))>,
}

/// Ripple edit: resolves a requested span by shifting the blocks on the dragged side
/// of the channel by the same delta, so relative spacing is preserved and no gap or
/// overlap can appear. Growing a right edge pushes every block that starts at or after
/// the original end rightward; shrinking it pulls them leftward to close the gap. The
/// mirror applies to the start edge. `requested` must already carry the anchored-edge
/// length (callers re-derive it, as the layers panel does).
pub fn pushed_breath_span(
    others: &[(u32, u32)],
    original: (u32, u32),
    requested: (u32, u32),
) -> PushedBreathSpan {
    let original_end = span_end_breath(original.0, original.1);
    let start_delta = requested.0 as i64 - original.0 as i64;
    let end_delta = span_end_breath(requested.0, requested.1) as i64 - original_end as i64;
    let mut result = PushedBreathSpan {
        edited: (requested.0, requested.1.max(1)),
        shifted: Vec::new(),
    };
    // Each edge ripples its own side: a start-edge move shifts everything left of the
    // block, an end-edge move shifts everything at/after the original end. A pure edge
    // drag moves only one edge; if both deltas are zero there is nothing to shift.
    for (index, &(start, length)) in others.iter().enumerate() {
        let end = span_end_breath(start, length);
        if start_delta != 0 && end <= original.0 {
            let shifted_start = (start as i64 + start_delta).max(0) as u32;
            result.shifted.push((index, (shifted_start, length)));
        } else if end_delta != 0 && start >= original_end {
            let shifted_start = (start as i64 + end_delta).max(0) as u32;
            result.shifted.push((index, (shifted_start, length)));
        }
    }
    result
}

/// Result of a destructive (overwrite) span edit: the edited block's new span plus how
/// every overlapped neighbor resolves. Target content wins — covered neighbors are
/// truncated, fully covered ones removed, and a neighbor that straddles the whole new
/// span splits into two pieces around it. Indices refer to positions in the `others`
/// slice passed to `destructive_breath_span`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DestructiveBreathSpan {
    pub edited: (u32, u32),
    pub truncated: Vec<(usize, (u32, u32))>,
    pub removed: Vec<usize>,
    pub splits: Vec<(usize, u32)>,
}

/// Destructive resolution: the edited block takes its full requested span and every
/// other block it covers yields. A shrink request overlaps nothing, so it behaves as a
/// plain trim — destructiveness only shows when growing or sliding over neighbors.
pub fn destructive_breath_span(
    others: &[(u32, u32)],
    requested: (u32, u32),
) -> DestructiveBreathSpan {
    let edited = (requested.0, requested.1.max(1));
    let edited_end = span_end_breath(edited.0, edited.1);
    let mut result = DestructiveBreathSpan {
        edited,
        ..Default::default()
    };
    for (index, &(start, length)) in others.iter().enumerate() {
        let end = span_end_breath(start, length);
        if end <= edited.0 || start >= edited_end {
            continue;
        }
        if start >= edited.0 && end <= edited_end {
            result.removed.push(index);
        } else if start < edited.0 && end > edited_end {
            result.splits.push((index, edited.0));
        } else if start < edited.0 {
            result.truncated.push((index, (start, edited.0 - start)));
        } else {
            result
                .truncated
                .push((index, (edited_end, end - edited_end)));
        }
    }
    result
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
    fn finds_the_block_covering_an_exact_boundary_breath() {
        let blocks = vec![block("a", 0, 3), block("b", 4, 8)];
        assert_eq!(block_covering_breath(&blocks, 4).unwrap().id, "b");
        assert_eq!(block_covering_breath(&blocks, 3).unwrap().id, "a");
    }

    #[test]
    fn returns_none_for_a_breath_in_a_gap_between_blocks() {
        let blocks = vec![block("a", 0, 2), block("b", 5, 8)];
        assert!(block_covering_breath(&blocks, 3).is_none());
        assert!(block_covering_breath(&blocks, 4).is_none());
    }

    #[test]
    fn returns_none_for_an_empty_track() {
        let blocks: Vec<PropertyBlock> = Vec::new();
        assert!(block_covering_breath(&blocks, 0).is_none());
    }

    #[test]
    fn span_helpers_cover_and_resolve_consistently() {
        assert_eq!(span_end_breath(4, 6), 10);
        assert_eq!(span_end_breath(0, 0), 0);
        assert!(breath_in_span(4, 4, 6));
        assert!(breath_in_span(9, 4, 6));
        assert!(!breath_in_span(10, 4, 6));
        assert!(!breath_in_span(3, 4, 6));
    }

    #[test]
    fn clamped_breath_span_moves_freely_without_neighbors() {
        assert_eq!(clamped_breath_span(&[], (8, 4), (100, 4)), (100, 4));
    }

    #[test]
    fn clamped_breath_span_stops_at_the_neighboring_block() {
        // Neighbors at 0..8 and 20..28; the block at 8..12 can only slide within the
        // free window 8..20, its end stopping at the right neighbor's start.
        let others = [(0, 8), (20, 8)];
        assert_eq!(clamped_breath_span(&others, (8, 4), (40, 4)), (16, 4));
        assert_eq!(clamped_breath_span(&others, (8, 4), (0, 4)), (8, 4));
    }

    #[test]
    fn clamped_breath_span_shrinks_a_trim_that_would_cross_a_neighbor() {
        let others = [(0, 8)];
        // Growing the end of a block at 8..12 with no right neighbor stays free.
        assert_eq!(clamped_breath_span(&others, (8, 4), (8, 40)), (8, 40));
        // With a right neighbor the length cannot cross it.
        let others = [(0, 8), (20, 8)];
        assert_eq!(clamped_breath_span(&others, (8, 4), (8, 40)), (8, 12));
    }

    #[test]
    fn clamped_breath_span_unwedges_previously_overlapping_data() {
        // Broken stored data: two blocks share breaths. Moving either one folds the
        // straddling twin to its nearer side, so the result is non-overlapping again.
        let others = [(4, 8)];
        let (start, length) = clamped_breath_span(&others, (6, 4), (6, 4));
        assert!(!breath_in_span(start, 4, 8) && !breath_in_span(4, start, length));
    }

    #[test]
    fn pushed_breath_span_grows_over_the_right_neighbor_ripple_style() {
        // Block at 8..12 grows right to 8..20: the neighbor at 20..28 shifts right by
        // the growth amount, relative spacing beyond it is untouched.
        let others = [(0, 8), (20, 8), (40, 4)];
        let pushed = pushed_breath_span(&others, (8, 4), (8, 12));
        assert_eq!(pushed.edited, (8, 12));
        assert_eq!(pushed.shifted, vec![(1, (28, 8)), (2, (48, 4))]);
    }

    #[test]
    fn pushed_breath_span_shrink_pulls_the_following_blocks_left() {
        // Block at 8..12 shrinks to 8..10: the blocks after it pull left to close the
        // gap, so the channel keeps its spacing with no hole.
        let others = [(0, 8), (12, 4), (20, 4)];
        let pushed = pushed_breath_span(&others, (8, 4), (8, 2));
        assert_eq!(pushed.edited, (8, 2));
        assert_eq!(pushed.shifted, vec![(1, (10, 4)), (2, (18, 4))]);
    }

    #[test]
    fn pushed_breath_span_start_edge_ripples_the_left_side() {
        // Start edge moves left (grow left): everything left of the block shifts left
        // by the same amount; shrink from the start would push them right.
        let others = [(0, 4), (20, 4)];
        let pushed = pushed_breath_span(&others, (8, 4), (4, 8));
        assert_eq!(pushed.edited, (4, 8));
        assert_eq!(pushed.shifted, vec![(0, (0, 4))]);
        // Shrinking from the start (end anchored) pushes the left-side blocks right.
        let pushed = pushed_breath_span(&others, (8, 4), (10, 2));
        assert_eq!(pushed.shifted, vec![(0, (2, 4))]);
    }

    #[test]
    fn pushed_breath_span_never_shifts_blocks_below_breath_zero() {
        let others = [(0, 4)];
        let pushed = pushed_breath_span(&others, (8, 4), (0, 12));
        assert_eq!(pushed.edited, (0, 12));
        assert_eq!(pushed.shifted, vec![(0, (0, 4))]);
    }

    #[test]
    fn destructive_breath_span_truncates_removes_and_splits_neighbors() {
        // Edited block takes 8..28 outright: the 4..12 neighbor truncates to 4..8, the
        // 20..24 block vanishes, and the straddling 0..48 block splits around it.
        let others = [(4, 8), (20, 4), (0, 48), (40, 4)];
        let destructive = destructive_breath_span(&others, (8, 20));
        assert_eq!(destructive.edited, (8, 20));
        assert_eq!(destructive.truncated, vec![(0, (4, 4))]);
        assert_eq!(destructive.removed, vec![1]);
        assert_eq!(destructive.splits, vec![(2, 8)]);
    }

    #[test]
    fn destructive_breath_span_shrink_behaves_as_a_plain_trim() {
        let others = [(0, 8), (20, 8)];
        let destructive = destructive_breath_span(&others, (8, 2));
        assert_eq!(destructive.edited, (8, 2));
        assert!(destructive.truncated.is_empty());
        assert!(destructive.removed.is_empty());
        assert!(destructive.splits.is_empty());
    }
}
