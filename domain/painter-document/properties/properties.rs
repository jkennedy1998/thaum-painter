use crate::manifest::PropertyBlock;

/// Finds the property block, if any, whose breath range covers `breath`.
pub fn block_covering_breath(blocks: &[PropertyBlock], breath: u32) -> Option<&PropertyBlock> {
    blocks
        .iter()
        .find(|block| breath >= block.start_breath && breath <= block.end_breath)
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
}
