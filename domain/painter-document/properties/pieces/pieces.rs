use crate::file_schema::PropertyBlock;
use crate::storage::SharedDocumentPropertyBlock;

/// Which UX piece of a property bar a breath sits on. UX routes by piece, never by
/// bar size: a 1-breath bar is a single head, a 2-breath bar is a left + right head,
/// and a 3+-breath bar is a left head, centers filling the middle, and a right head.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarPiece {
    /// The one cell of a 1-breath bar.
    Single,
    /// The first cell of a 2+-breath bar.
    LeftHead,
    /// The last cell of a 2+-breath bar.
    RightHead,
    /// A middle cell of a 3+-breath bar.
    Center,
}

/// The two cell types of a binary property channel: a bar is either empty (blank) or
/// solid (content). Voids are not a cell type — under the tiling invariant every
/// breath inside the layer span is covered by exactly one bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellType {
    Empty,
    Solid,
}

/// The full classification result for one breath: which piece of which cell type.
/// The 48-branch interaction surface in the layers panel keys off exactly this pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BarCell {
    pub piece: BarPiece,
    pub cell_type: CellType,
}

/// Local view over any bar shape so every block kind in the painter classifies
/// through this one seam instead of per-call-site span math. Implemented for the
/// stored (`SharedDocumentPropertyBlock`) and file-schema (`PropertyBlock`) shapes;
/// panel-side mirrors implement it next to their own module.
pub trait BreathBar {
    fn bar_start_breath(&self) -> u32;
    fn bar_length_breaths(&self) -> u32;
    fn bar_is_blank(&self) -> bool;
}

impl BreathBar for SharedDocumentPropertyBlock {
    fn bar_start_breath(&self) -> u32 {
        self.start_breath
    }
    fn bar_length_breaths(&self) -> u32 {
        self.length_breaths
    }
    fn bar_is_blank(&self) -> bool {
        self.is_blank
    }
}

impl BreathBar for PropertyBlock {
    fn bar_start_breath(&self) -> u32 {
        self.start_breath
    }
    fn bar_length_breaths(&self) -> u32 {
        // FileSchema blocks carry an inclusive end breath; length is cell count.
        self.end_breath.saturating_sub(self.start_breath) + 1
    }
    fn bar_is_blank(&self) -> bool {
        // FileSchema blocks are value-carrying content by construction.
        false
    }
}

/// Classifies which piece of a start+length bar the given breath sits on. The breath
/// is expected to be inside the bar; out-of-bar breaths resolve to [`BarPiece::Single`]
/// only for zero-length bars, otherwise the caller's covering lookup already failed.
pub fn classify_bar_piece(start_breath: u32, length_breaths: u32, breath: u32) -> BarPiece {
    if length_breaths <= 1 {
        return BarPiece::Single;
    }
    let end = start_breath + length_breaths - 1;
    if breath == start_breath {
        BarPiece::LeftHead
    } else if breath == end {
        BarPiece::RightHead
    } else {
        BarPiece::Center
    }
}

/// Reads the cell type off a bar's blank flag: blank is empty, anything else is solid.
pub fn cell_type_of(is_blank: bool) -> CellType {
    if is_blank {
        CellType::Empty
    } else {
        CellType::Solid
    }
}

/// Resolves the breath to the covering bar's piece × cell type. Under the tiling
/// invariant this is total inside the layer span; `None` remains only while stored
/// data still carries gap breaths (pre-tiling documents), never as a UX concept.
pub fn covering_bar_cell<B: BreathBar>(blocks: &[B], breath: u32) -> Option<BarCell> {
    let bar = blocks.iter().find(|bar| {
        let start = bar.bar_start_breath();
        breath >= start && breath < start + bar.bar_length_breaths()
    })?;
    Some(BarCell {
        piece: classify_bar_piece(
            bar.bar_start_breath(),
            bar.bar_length_breaths(),
            breath,
        ),
        cell_type: cell_type_of(bar.bar_is_blank()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stored(start: u32, length: u32, is_blank: bool) -> SharedDocumentPropertyBlock {
        SharedDocumentPropertyBlock {
            id: format!("b{start}-{length}"),
            start_breath: start,
            length_breaths: length,
            is_blank,
            value: None,
            interpretation: None,
        }
    }

    #[test]
    fn a_one_breath_bar_is_a_single_head() {
        assert_eq!(classify_bar_piece(8, 1, 8), BarPiece::Single);
    }

    #[test]
    fn a_two_breath_bar_is_a_left_and_a_right_head_only() {
        assert_eq!(classify_bar_piece(4, 2, 4), BarPiece::LeftHead);
        assert_eq!(classify_bar_piece(4, 2, 5), BarPiece::RightHead);
    }

    #[test]
    fn a_three_plus_breath_bar_has_heads_and_centers() {
        assert_eq!(classify_bar_piece(2, 5, 2), BarPiece::LeftHead);
        assert_eq!(classify_bar_piece(2, 5, 3), BarPiece::Center);
        assert_eq!(classify_bar_piece(2, 5, 4), BarPiece::Center);
        assert_eq!(classify_bar_piece(2, 5, 5), BarPiece::Center);
        assert_eq!(classify_bar_piece(2, 5, 6), BarPiece::RightHead);
    }

    #[test]
    fn cell_type_reads_the_blank_flag() {
        assert_eq!(cell_type_of(false), CellType::Solid);
        assert_eq!(cell_type_of(true), CellType::Empty);
    }

    #[test]
    fn covering_lookup_classifies_stored_solid_and_empty_bars() {
        let blocks = vec![stored(0, 3, false), stored(3, 2, true)];
        assert_eq!(
            covering_bar_cell(&blocks, 0),
            Some(BarCell { piece: BarPiece::LeftHead, cell_type: CellType::Solid })
        );
        assert_eq!(
            covering_bar_cell(&blocks, 2),
            Some(BarCell { piece: BarPiece::RightHead, cell_type: CellType::Solid })
        );
        assert_eq!(
            covering_bar_cell(&blocks, 3),
            Some(BarCell { piece: BarPiece::LeftHead, cell_type: CellType::Empty })
        );
        assert_eq!(
            covering_bar_cell(&blocks, 4),
            Some(BarCell { piece: BarPiece::RightHead, cell_type: CellType::Empty })
        );
    }

    #[test]
    fn covering_lookup_resolves_file_schema_blocks_as_solid_content() {
        let schema = PropertyBlock {
            id: "m".to_string(),
            start_breath: 4,
            end_breath: 6,
            value: serde_json::Value::Null,
        };
        assert_eq!(
            covering_bar_cell(&[schema], 6),
            Some(BarCell { piece: BarPiece::RightHead, cell_type: CellType::Solid })
        );
    }

    #[test]
    fn covering_lookup_is_none_only_for_gap_breaths() {
        // Pre-tiling data only: under the tiling invariant a layer span is fully
        // covered, so this branch becomes unreachable inside the span.
        let blocks = vec![stored(0, 2, false), stored(5, 2, false)];
        assert!(covering_bar_cell(&blocks, 3).is_none());
        assert!(covering_bar_cell::<SharedDocumentPropertyBlock>(&[], 0).is_none());
    }
}
