use crate::file_schema::PropertyBlock;

/// Finds the property block, if any, whose breath range covers `breath`.
pub fn block_covering_breath(blocks: &[PropertyBlock], breath: u32) -> Option<&PropertyBlock> {
    blocks
        .iter()
        .find(|block| breath >= block.start_breath && breath <= block.end_breath)
}

/// Exclusive end of a start+length breath span. Every block shape in the painter
/// (file-schema `PropertyBlock`, stored `SharedDocumentPropertyBlock`, panel rows) resolves
/// its range through these span helpers so breath semantics stay defined in one place.
pub fn span_end_breath(start_breath: u32, length_breaths: u32) -> u32 {
    start_breath.saturating_add(length_breaths)
}

/// True when `breath` falls inside the start+length span (the span covers it).
pub fn breath_in_span(breath: u32, start_breath: u32, length_breaths: u32) -> bool {
    breath >= start_breath && breath < span_end_breath(start_breath, length_breaths)
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
/// every overlapped neighbor resolves. Under binary tiling victims become blanks — a
/// partially overlapped neighbor keeps only its un-covered remainder and turns empty
/// (content discarded), and a fully covered neighbor is removed outright (the edited
/// block covers its range, so no blank is needed). The edited block's own vacated
/// range is not part of this result; the storage seam re-tiles it into a blank.
/// Indices refer to positions in the `others` slice passed to `destructive_breath_span`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DestructiveBreathSpan {
    pub edited: (u32, u32),
    /// Partially overlapped neighbors: (index, residual (start, length) span) that
    /// becomes a blank block, content discarded.
    pub blanked: Vec<(usize, (u32, u32))>,
    /// Fully covered neighbors, removed outright.
    pub removed: Vec<usize>,
}

/// Destructive resolution: the edited block takes its full requested span and every
/// other block it covers yields into an empty cell type. A shrink request overlaps
/// nothing, so it behaves as a plain trim — destructiveness only shows when growing
/// or sliding over neighbors.
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
        } else if start < edited.0 {
            result.blanked.push((index, (start, edited.0 - start)));
        } else {
            result.blanked.push((index, (edited_end, end - edited_end)));
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
    fn destructive_breath_span_blanks_and_removes_covered_neighbors() {
        // Edited block takes 8..28 outright: the 4..12 neighbor keeps only 4..8 and
        // turns empty, and the 20..24 block is fully covered so it is removed outright.
        let others = [(4, 8), (20, 4), (40, 4)];
        let destructive = destructive_breath_span(&others, (8, 20));
        assert_eq!(destructive.edited, (8, 20));
        assert_eq!(destructive.blanked, vec![(0, (4, 4))]);
        assert_eq!(destructive.removed, vec![1]);
    }

    #[test]
    fn destructive_breath_span_shrink_behaves_as_a_plain_trim() {
        let others = [(0, 8), (20, 8)];
        let destructive = destructive_breath_span(&others, (8, 2));
        assert_eq!(destructive.edited, (8, 2));
        assert!(destructive.blanked.is_empty());
        assert!(destructive.removed.is_empty());
    }
}
