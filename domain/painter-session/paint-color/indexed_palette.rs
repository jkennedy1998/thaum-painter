pub const LEGACY_INDEXED_PALETTE: [[u8; 3]; 37] = [
    [0x12, 0x0a, 0x1a],
    [0xfe, 0xff, 0xe5],
    [0xe3, 0x63, 0x25],
    [0xe2, 0xe9, 0x90],
    [0x3d, 0x23, 0x29],
    [0x2d, 0x3a, 0x26],
    [0x2a, 0x2a, 0x41],
    [0x42, 0x24, 0x90],
    [0x7f, 0x37, 0xbc],
    [0xa5, 0x44, 0xff],
    [0xde, 0xbf, 0xe0],
    [0xab, 0x8a, 0xb5],
    [0xff, 0x26, 0xa8],
    [0xff, 0x69, 0xa9],
    [0xdc, 0x34, 0x26],
    [0xb2, 0x15, 0x35],
    [0xa8, 0x56, 0x1a],
    [0xc4, 0x70, 0x2b],
    [0xea, 0x98, 0x27],
    [0xf2, 0x66, 0x57],
    [0xff, 0xc6, 0x2f],
    [0xff, 0xf3, 0xb3],
    [0x8c, 0x9d, 0x4f],
    [0xa9, 0xc4, 0x48],
    [0x4f, 0x9d, 0x35],
    [0x8b, 0xf5, 0xc6],
    [0x4d, 0xc6, 0xe4],
    [0x44, 0x77, 0xff],
    [0x27, 0x49, 0xd0],
    [0x40, 0x48, 0x63],
    [0x78, 0x7d, 0x8b],
    [0x9d, 0xa5, 0xae],
    [0xe0, 0xe8, 0xd0],
    [0xac, 0x9d, 0x7c],
    [0xfa, 0xd5, 0xaf],
    [0xc5, 0xb5, 0xa8],
    [0xd2, 0x79, 0x79],
];

pub const fn legacy_indexed_palette() -> &'static [[u8; 3]; 37] {
    &LEGACY_INDEXED_PALETTE
}

/// The shipped default file palette (J 2026-10-10): the legacy 37 colors are
/// every user's starting point. A saved file carries its own palette only
/// once it has been edited away from this default.
pub fn default_file_palette() -> Vec<[u8; 3]> {
    LEGACY_INDEXED_PALETTE.to_vec()
}

/// True when `palette` is exactly the shipped default, so a file need not
/// carry a palette section that says nothing new.
pub fn is_default_file_palette(palette: &[[u8; 3]]) -> bool {
    palette == LEGACY_INDEXED_PALETTE
}

/// Returns the palette entry nearest to `rgb` in squared RGB distance over
/// the given file palette. Ties retain palette order, so the result is
/// deterministic. The palette is caller-owned file truth — this seam never
/// falls back to a hidden global.
pub fn nearest_indexed_rgb(rgb: [u8; 3], palette: &[[u8; 3]]) -> [u8; 3] {
    *palette
        .iter()
        .min_by_key(|candidate| {
            candidate
                .iter()
                .zip(rgb)
                .map(|(&channel, target)| {
                    let delta = i32::from(channel) - i32::from(target);
                    delta * delta
                })
                .sum::<i32>()
        })
        .expect("the indexed palette is non-empty")
}

/// Adds one color to the end of the file palette unless an identical color
/// is already present. Returns whether the palette changed.
pub fn palette_push_color(palette: &mut Vec<[u8; 3]>, rgb: [u8; 3]) -> bool {
    if palette.contains(&rgb) {
        return false;
    }
    palette.push(rgb);
    true
}

/// Removes every occurrence of `rgb` from the file palette and returns
/// whether anything was removed. Painted cells keep their raw RGB — palette
/// membership is bookkeeping, never a rewrite of cell colors.
pub fn palette_remove_color(palette: &mut Vec<[u8; 3]>, rgb: [u8; 3]) -> bool {
    let before = palette.len();
    palette.retain(|&candidate| candidate != rgb);
    before != palette.len()
}

/// Parses a typed color entry as hex: an optional leading `#` followed by
/// exactly six hex digits (`RRGGBB`). Case-insensitive; nothing else parses.
pub fn parse_hex_color(text: &str) -> Option<[u8; 3]> {
    let text = text.trim().trim_start_matches('#');
    if text.len() != 6 || !text.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |range: std::ops::Range<usize>| u8::from_str_radix(&text[range], 16).ok();
    Some([
        channel(0..2)?,
        channel(2..4)?,
        channel(4..6)?,
    ])
}

/// Parses a typed color entry as decimal rgb: three channels 0..=255
/// separated by spaces and/or commas (`255 90 20`, `255,90,20`), or nine
/// raw digits with no separators (split 3+3+3). Nothing else parses.
pub fn parse_rgb_color(text: &str) -> Option<[u8; 3]> {
    let text = text.trim();
    if text.is_empty()
        || !text
            .chars()
            .all(|ch| ch.is_ascii_digit() || ch == ' ' || ch == ',')
    {
        return None;
    }
    let digits: String = text.chars().filter(|ch| ch.is_ascii_digit()).collect();
    let channel_values: Vec<Option<u8>> = if text.chars().all(|ch| ch.is_ascii_digit()) {
        if digits.len() != 9 {
            return None;
        }
        (0..3)
            .map(|index| u8::from_str_radix(&digits[index * 3..index * 3 + 3], 10).ok())
            .collect()
    } else {
        let tokens: Vec<&str> = text.split([' ', ',']).filter(|token| !token.is_empty()).collect();
        if tokens.len() != 3 || tokens.iter().any(|token| token.len() > 3) {
            return None;
        }
        tokens
            .iter()
            .map(|token| token.parse::<u8>().ok())
            .collect()
    };
    Some([
        channel_values[0]?,
        channel_values[1]?,
        channel_values[2]?,
    ])
}

/// The canonical display form of one palette color: uppercase hex with the
/// leading `#`.
pub fn hex_color_string(rgb: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2])
}

/// The canonical display form of one palette color as decimal rgb:
/// zero-padded `RRR GGG BBB`.
pub fn rgb_color_string(rgb: [u8; 3]) -> String {
    format!("{:03} {:03} {:03}", rgb[0], rgb[1], rgb[2])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_indexed_rgb_returns_a_palette_entry_and_preserves_exact_matches() {
        assert_eq!(
            nearest_indexed_rgb([0xe3, 0x63, 0x25], legacy_indexed_palette()),
            [0xe3, 0x63, 0x25]
        );
        let resolved = nearest_indexed_rgb([25, 50, 10], legacy_indexed_palette());
        assert!(legacy_indexed_palette().contains(&resolved));
    }

    #[test]
    fn nearest_indexed_rgb_resolves_against_the_supplied_palette_not_the_default() {
        let palette = vec![[10, 0, 0], [240, 0, 0]];
        assert_eq!(nearest_indexed_rgb([0, 0, 0], &palette), [10, 0, 0]);
        assert_eq!(nearest_indexed_rgb([250, 0, 0], &palette), [240, 0, 0]);
    }

    #[test]
    fn default_file_palette_matches_the_legacy_list_and_default_detection() {
        assert_eq!(default_file_palette(), legacy_indexed_palette().to_vec());
        assert!(is_default_file_palette(&default_file_palette()));
        assert!(!is_default_file_palette(&vec![[1, 2, 3]]));
    }

    #[test]
    fn palette_push_dedupes_and_remove_drops_only_that_color() {
        let mut palette = vec![[1, 2, 3]];
        assert!(palette_push_color(&mut palette, [4, 5, 6]));
        assert!(!palette_push_color(&mut palette, [4, 5, 6]));
        assert_eq!(palette, vec![[1, 2, 3], [4, 5, 6]]);
        assert!(palette_remove_color(&mut palette, [1, 2, 3]));
        assert!(!palette_remove_color(&mut palette, [9, 9, 9]));
        assert_eq!(palette, vec![[4, 5, 6]]);
    }

    #[test]
    fn hex_entries_parse_with_or_without_the_leading_hash() {
        assert_eq!(parse_hex_color("E36325"), Some([0xe3, 0x63, 0x25]));
        assert_eq!(parse_hex_color("#e36325"), Some([0xe3, 0x63, 0x25]));
        assert_eq!(parse_hex_color("12Oa1a"), None);
        assert_eq!(parse_hex_color(""), None);
        assert_eq!(parse_hex_color("#fff"), None);
        assert_eq!(hex_color_string([0x12, 0x0a, 0x1a]), "#120A1A");
    }

    #[test]
    fn rgb_entries_parse_as_separated_channels_or_nine_raw_digits() {
        assert_eq!(parse_rgb_color("255 090 020"), Some([255, 90, 20]));
        assert_eq!(parse_rgb_color("255,90,20"), Some([255, 90, 20]));
        assert_eq!(parse_rgb_color(" 12 , 34 , 5 "), Some([12, 34, 5]));
        assert_eq!(parse_rgb_color("255090020"), Some([255, 90, 20]));
        assert_eq!(parse_rgb_color("256 0 0"), None);
        assert_eq!(parse_rgb_color("25509002"), None);
        assert_eq!(parse_rgb_color("255 90"), None);
        assert_eq!(parse_rgb_color("255 90 20 1"), None);
        assert_eq!(parse_rgb_color(""), None);
        assert_eq!(parse_rgb_color("12x34x5"), None);
        assert_eq!(rgb_color_string([255, 90, 20]), "255 090 020");
    }
}
