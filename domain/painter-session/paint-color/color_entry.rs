//! In-place color entry for the palette module's edit fields: the typing
//! seam opens one edit, keys build the active field's buffer, and the hex
//! and rgb fields stay linked — a complete value in either field rewrites
//! the other, so the two are always two views of one color. `+` (or Enter,
//! through the host seam) commits the active field's parsed color into the
//! live file palette.

use crate::legacy_indexed_palette::{parse_hex_color, parse_rgb_color, rgb_color_string};

/// Which entry field is currently being typed into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorEntryField {
    Hex,
    Rgb,
}

/// One open color entry session over the palette module's two edit fields.
/// The hex buffer caps at six hex digits; the rgb buffer caps at the
/// canonical `RRR GGG BBB` width. An incomplete entry refuses to commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColorEntryEdit {
    /// The field keys route into right now.
    pub field: ColorEntryField,
    hex: String,
    rgb: String,
    /// The most recent fully-parsed color, so the preview swatch keeps
    /// showing something meaningful while a half-typed entry is invalid.
    last_valid: [u8; 3],
    /// Whether the user has typed into the active field yet. The first
    /// push replaces the seeded buffer wholesale, so typing a new color
    /// never fights a full-width seed character by character.
    touched: bool,
}

impl ColorEntryEdit {
    /// Opens the edit seeded with `rgb` (typically the active hand's color,
    /// so the user can tweak what is already selected rather than typing
    /// from scratch), typing into `field`.
    pub fn begin(rgb: [u8; 3], field: ColorEntryField) -> Self {
        let hex: String = rgb
            .iter()
            .map(|channel| format!("{channel:02X}"))
            .collect();
        Self {
            field,
            hex,
            rgb: rgb_color_string(rgb),
            last_valid: rgb,
            touched: false,
        }
    }

    pub fn hex_text(&self) -> &str {
        &self.hex
    }

    pub fn rgb_text(&self) -> &str {
        &self.rgb
    }

    /// The color the entry currently resolves to (the last fully valid
    /// value across both fields).
    pub fn preview_rgb(&self) -> [u8; 3] {
        self.last_valid
    }

    /// Accepts one character into the active field — hex digits only for
    /// the hex field, digits and separators for the rgb field — then
    /// re-derives the other field whenever the active field parses. The
    /// first push replaces the seeded buffer wholesale.
    pub fn push(&mut self, ch: char) {
        let accepts: fn(char) -> bool = match self.field {
            ColorEntryField::Hex => |ch| ch.is_ascii_hexdigit(),
            ColorEntryField::Rgb => |ch| ch.is_ascii_digit() || ch == ' ' || ch == ',',
        };
        if !accepts(ch) {
            return;
        }
        let (buffer, cap, normalized) = match self.field {
            ColorEntryField::Hex => (&mut self.hex, 6, ch.to_ascii_uppercase()),
            ColorEntryField::Rgb => (&mut self.rgb, 11, ch),
        };
        if !self.touched {
            buffer.clear();
            self.touched = true;
        }
        if buffer.len() >= cap {
            return;
        }
        buffer.push(normalized);
        self.mirror();
    }

    pub fn backspace(&mut self) {
        let buffer = match self.field {
            ColorEntryField::Hex => &mut self.hex,
            ColorEntryField::Rgb => &mut self.rgb,
        };
        buffer.pop();
        self.mirror();
    }

    /// The committed color, when the active field parses.
    pub fn commit(&self) -> Option<[u8; 3]> {
        match self.field {
            ColorEntryField::Hex => parse_hex_color(&self.hex),
            ColorEntryField::Rgb => parse_rgb_color(&self.rgb),
        }
    }

    /// Whenever the active field parses as a complete color, the other
    /// field's text is rewritten to that color's canonical form.
    fn mirror(&mut self) {
        let Some(rgb) = self.commit() else {
            return;
        };
        self.last_valid = rgb;
        let text = rgb_color_string(rgb);
        let hex: String = rgb.iter().map(|channel| format!("{channel:02X}")).collect();
        match self.field {
            ColorEntryField::Hex => self.rgb = text,
            ColorEntryField::Rgb => self.hex = hex,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn begin_seeds_both_fields_from_one_color() {
        let edit = ColorEntryEdit::begin([0xe3, 0x63, 0x25], ColorEntryField::Hex);
        assert_eq!(edit.hex_text(), "E36325");
        assert_eq!(edit.rgb_text(), "227 099 037");
        assert_eq!(edit.preview_rgb(), [0xe3, 0x63, 0x25]);
        assert_eq!(edit.commit(), Some([0xe3, 0x63, 0x25]));
    }

    #[test]
    fn typing_hex_digits_rewrites_the_rgb_field() {
        let mut edit = ColorEntryEdit::begin([0, 0, 0], ColorEntryField::Hex);
        for ch in "ff8000".chars() {
            edit.push(ch);
        }
        assert_eq!(edit.hex_text(), "FF8000");
        assert_eq!(edit.rgb_text(), "255 128 000");
        assert_eq!(edit.preview_rgb(), [0xff, 0x80, 0x00]);
    }

    #[test]
    fn typing_rgb_values_rewrites_the_hex_field() {
        let mut edit = ColorEntryEdit::begin([0, 0, 0], ColorEntryField::Rgb);
        for ch in "0,200,77".chars() {
            edit.push(ch);
        }
        assert_eq!(edit.rgb_text(), "0,200,77");
        assert_eq!(edit.hex_text(), "00C84D");
        assert_eq!(edit.preview_rgb(), [0, 200, 77]);
    }

    #[test]
    fn typing_replaces_the_seeded_buffer_then_appends() {
        let mut edit = ColorEntryEdit::begin([0x12, 0x34, 0x56], ColorEntryField::Rgb);
        for ch in "9 9 9".chars() {
            edit.push(ch);
        }
        assert_eq!(edit.rgb_text(), "9 9 9");
        assert_eq!(edit.preview_rgb(), [9, 9, 9]);
        assert_eq!(edit.hex_text(), "090909");
        // back off to an incomplete two-channel entry: nothing mirrors,
        // the preview holds the last fully valid value.
        edit.backspace();
        edit.backspace();
        edit.push('5');
        assert_eq!(edit.rgb_text(), "9 95");
        assert_eq!(edit.commit(), None);
        assert_eq!(edit.preview_rgb(), [9, 9, 9]);
        assert_eq!(edit.hex_text(), "090909");
    }

    #[test]
    fn the_buffers_cap_at_their_field_widths_and_refuse_noise() {
        let mut edit = ColorEntryEdit::begin([0, 0, 0], ColorEntryField::Hex);
        for ch in "abcdefg7".chars() {
            edit.push(ch);
        }
        assert_eq!(edit.hex_text(), "ABCDEF");

        let mut edit = ColorEntryEdit::begin([0, 0, 0], ColorEntryField::Rgb);
        for ch in "1234567890123".chars() {
            edit.push(ch);
        }
        assert_eq!(edit.rgb_text(), "12345678901");
        // Nine raw digits without separators are read 3+3+3, and this
        // buffer's triples overflow the channel range — no parse.
        assert_eq!(edit.commit(), None);

        let mut edit = ColorEntryEdit::begin([0, 0, 0], ColorEntryField::Rgb);
        for ch in "25,50,7".chars() {
            edit.push(ch);
        }
        assert_eq!(edit.commit(), Some([25, 50, 7]));

        let mut edit = ColorEntryEdit::begin([0, 0, 0], ColorEntryField::Hex);
        edit.push('z');
        assert_eq!(edit.hex_text(), "000000");
    }

    #[test]
    fn an_incomplete_entry_refuses_to_commit() {
        let mut edit = ColorEntryEdit::begin([0, 0, 0], ColorEntryField::Hex);
        edit.backspace();
        assert_eq!(edit.commit(), None);
        let mut edit = ColorEntryEdit::begin([0, 0, 0], ColorEntryField::Rgb);
        for _ in 0..11 {
            edit.backspace();
        }
        assert_eq!(edit.commit(), None);
    }
}
