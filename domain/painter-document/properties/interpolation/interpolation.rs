use crate::file_schema::{PropertyBlock, RasterSegment};

/// What renders for a breath that no block or segment covers.
///
/// Only `NoContent` exists today: the group renders as if it has nothing for
/// this property/raster track at this breath, matching prior behavior. Real
/// interpolation (holding the nearest earlier block, lerping between the
/// surrounding blocks, etc.) lands as new variants here later.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GapFill {
    NoContent,
}

/// A breath-ranged item: a property block or a raster segment.
pub trait BreathRanged {
    fn start_breath(&self) -> u32;
    fn end_breath(&self) -> u32;
}

impl BreathRanged for PropertyBlock {
    fn start_breath(&self) -> u32 {
        self.start_breath
    }
    fn end_breath(&self) -> u32 {
        self.end_breath
    }
}

impl BreathRanged for RasterSegment {
    fn start_breath(&self) -> u32 {
        self.start_breath
    }
    fn end_breath(&self) -> u32 {
        self.end_breath
    }
}

/// The nearest item ending before `breath` and the nearest item starting
/// after `breath` — the inputs a future interpolation behavior would need.
pub fn surrounding_items<T: BreathRanged>(items: &[T], breath: u32) -> (Option<&T>, Option<&T>) {
    let prev = items
        .iter()
        .filter(|item| item.end_breath() < breath)
        .max_by_key(|item| item.end_breath());
    let next = items
        .iter()
        .filter(|item| item.start_breath() > breath)
        .min_by_key(|item| item.start_breath());
    (prev, next)
}

/// Resolves what should render for a breath not covered by any item.
/// Always `NoContent` today; this is the landing point for real
/// interpolation behavior later.
pub fn resolve_gap_fill<T: BreathRanged>(_items: &[T], _breath: u32) -> GapFill {
    GapFill::NoContent
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(id: &str, start_breath: u32, end_breath: u32) -> PropertyBlock {
        PropertyBlock {
            id: id.to_string(),
            start_breath,
            end_breath,
            value: serde_json::Value::Null,
        }
    }

    #[test]
    fn surrounding_items_finds_the_nearest_block_on_each_side_of_a_gap() {
        let blocks = vec![block("a", 0, 2), block("b", 8, 10)];
        let (prev, next) = surrounding_items(&blocks, 5);
        assert_eq!(prev.unwrap().id, "a");
        assert_eq!(next.unwrap().id, "b");
    }

    #[test]
    fn surrounding_items_returns_none_on_either_side_with_nothing_there() {
        let blocks = vec![block("a", 8, 10)];
        let (prev, next) = surrounding_items(&blocks, 5);
        assert!(prev.is_none());
        assert_eq!(next.unwrap().id, "a");
    }

    #[test]
    fn resolve_gap_fill_is_always_no_content_today() {
        let blocks = vec![block("a", 0, 2), block("b", 8, 10)];
        assert_eq!(resolve_gap_fill(&blocks, 5), GapFill::NoContent);
    }
}
