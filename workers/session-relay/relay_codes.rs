//! Invite codes for relay rooms: `<room6>-<token10>` in one paste-able field.
//!
//! The code IS the room auth — knowing it is the invite — so both halves are
//! minted from OS entropy in an alphabet without ambiguous glyphs (no 0/O/1/I).
//! The room id names the room; the room token authorizes joining it. The relay
//! mints both when a host claims a room; the host app displays the code
//! verbatim and joiners paste it into the same JOIN field they already use.

use getrandom::getrandom;

/// Alphabet: Crockford base32 minus the vowels-adjacent ambiguity — no 0/O/1/I/L.
const CODE_ALPHABET: &[u8] = b"23456789ABCDEFGHJKMNPQRSTUVWXYZ";

pub const ROOM_ID_LEN: usize = 6;
pub const ROOM_TOKEN_LEN: usize = 10;
pub const CODE_SEPARATOR: char = '-';

/// The full invite code a user pastes: `ROOM6-TOKEN10` (17 chars).
pub fn generate_code() -> (String, String, String) {
    let room = code_part(ROOM_ID_LEN);
    let token = code_part(ROOM_TOKEN_LEN);
    let code = format!("{room}{CODE_SEPARATOR}{token}");
    (room, token, code)
}

/// Splits a pasted code back into `(room, token)`. Accepts lowercase input and
/// stray whitespace at the edges; anything else (empty, wrong lengths, foreign
/// glyphs, missing separator) is None — the JOIN field routes by shape, so a
/// code-shaped string that does not parse is just not a code.
pub fn parse_code(raw: &str) -> Option<(String, String)> {
    let trimmed = raw.trim().to_ascii_uppercase();
    let (room, token) = trimmed.split_once(CODE_SEPARATOR)?;
    if room.len() != ROOM_ID_LEN || token.len() != ROOM_TOKEN_LEN {
        return None;
    }
    for byte in room.bytes().chain(token.bytes()) {
        if !CODE_ALPHABET.contains(&byte) {
            return None;
        }
    }
    Some((room.to_string(), token.to_string()))
}

fn code_part(length: usize) -> String {
    let mut bytes = vec![0u8; length];
    getrandom(&mut bytes).expect("OS entropy unavailable — cannot mint invite codes");
    bytes
        .iter()
        .map(|byte| CODE_ALPHABET[*byte as usize % CODE_ALPHABET.len()] as char)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_round_trip_through_parse() {
        for _ in 0..64 {
            let (room, token, code) = generate_code();
            assert_eq!(code.len(), ROOM_ID_LEN + ROOM_TOKEN_LEN + 1);
            assert_eq!(code, format!("{room}-{token}"));
            let (parsed_room, parsed_token) = parse_code(&code).expect("generated codes parse");
            assert_eq!(parsed_room, room);
            assert_eq!(parsed_token, token);
        }
    }

    #[test]
    fn codes_stay_inside_the_unambiguous_alphabet() {
        for _ in 0..64 {
            let (_, _, code) = generate_code();
            for glyph in code.chars() {
                if glyph == CODE_SEPARATOR {
                    continue;
                }
                assert_ne!(glyph, '0');
                assert_ne!(glyph, 'O');
                assert_ne!(glyph, '1');
                assert_ne!(glyph, 'I');
                assert_ne!(glyph, 'L');
                assert!(glyph.is_ascii_alphanumeric());
            }
        }
    }

    #[test]
    fn parse_accepts_lowercase_and_edge_whitespace() {
        let (room, token, _) = generate_code();
        let sloppy = format!("  {}-{}  ", room.to_lowercase(), token.to_lowercase());
        let (parsed_room, parsed_token) = parse_code(&sloppy).expect("sloppy paste parses");
        assert_eq!(parsed_room, room);
        assert_eq!(parsed_token, token);
    }

    #[test]
    fn parse_rejects_non_code_shapes() {
        assert_eq!(parse_code(""), None);
        assert_eq!(parse_code("127.0.0.1:4747"), None);
        assert_eq!(parse_code("ABC"), None);
        assert_eq!(parse_code("ABCDE-FGHIJK"), None); // wrong lengths
        assert_eq!(parse_code("ABCDEF-GHIJKLM"), None);
        assert_eq!(parse_code("ABCDE0-GHIJKLMN"), None); // 0 not in alphabet
        assert_eq!(parse_code("ABCDEF-GHIJKLMN-EXTRA"), None);
    }
}
