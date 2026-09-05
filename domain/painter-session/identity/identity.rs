//! Session identity: the painter's stable per-install user identity.
//!
//! `user_id` is random, generated once, persisted, and never regenerated — it
//! keys record ownership in the shared action log (foreign records are skipped
//! by comparing `user_id`), so a rotated id would make your own old records
//! foreign to you. It is never derived from `$USER`, hostname, or IP: two
//! friends with the same username must never collide. `display_name` and
//! `presence_color` are cosmetic data, safe to edit any time.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Presence colors offered at identity creation: the primaries and secondaries
/// hand-picked from the painter's indexed palette that read well as cursor
/// colors on both document backgrounds. Stored on the identity as data (an RGB
/// triple, not a palette index) so the indexed palette can change without
/// breaking existing identities; a user may re-pick any time.
pub const PRESENCE_CANDIDATE_COLORS: [[u8; 3]; 8] = [
    [0xdc, 0x34, 0x26], // red
    [0xe3, 0x63, 0x25], // orange
    [0xff, 0xc6, 0x2f], // yellow
    [0x4f, 0x9d, 0x35], // green
    [0x4d, 0xc6, 0xe4], // cyan
    [0x44, 0x77, 0xff], // blue
    [0xa5, 0x44, 0xff], // purple
    [0xff, 0x26, 0xa8], // magenta
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionIdentity {
    /// Stable globally-unique owner id baked into minted action ids. Permanent
    /// once created.
    pub user_id: String,
    /// Cosmetic label shown in presence UI; never used for ownership.
    pub display_name: String,
    /// Cosmetic presence color as an RGB triple from the indexed palette.
    pub presence_color: [u8; 3],
}

impl SessionIdentity {
    /// Generates a fresh identity. `display_name` seeds the cosmetic label
    /// (e.g. the OS username); ownership never touches it.
    pub fn generate(display_name: Option<&str>) -> Self {
        let random = random_bytes_16();
        Self {
            user_id: format!("u-{}", hex(&random)),
            display_name: display_name
                .map(str::to_string)
                .unwrap_or_else(|| "painter".to_string()),
            presence_color: {
                let pick = (random[0] as usize) % PRESENCE_CANDIDATE_COLORS.len();
                PRESENCE_CANDIDATE_COLORS[pick]
            },
        }
    }

    /// Loads the persisted identity, creating it exactly once if absent. An
    /// existing file is never rewritten or regenerated — identity is permanent.
    /// Missing optional fields (older files) fall back to defaults.
    pub fn load_or_create(path: &Path, display_name: Option<&str>) -> std::io::Result<Self> {
        if let Ok(text) = fs::read_to_string(path) {
            if let Ok(identity) = serde_json::from_str::<SessionIdentity>(&text) {
                return Ok(identity);
            }
        }
        let identity = Self::generate(display_name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, serde_json::to_string_pretty(&identity)?)?;
        Ok(identity)
    }

    /// Test-seam override: swaps in a fixed user_id without persisting. Never
    /// use in normal flow — ids are permanent, this bypasses generation.
    pub fn with_user_id_for_tests(mut self, user_id: impl Into<String>) -> Self {
        self.user_id = user_id.into();
        self
    }
}

/// 128 random bits: /dev/urandom where it exists, otherwise a time+pid mix
/// (uniqueness across installs is what matters; this fallback is best-effort).
fn random_bytes_16() -> [u8; 16] {
    // Bounded read: exactly 16 bytes, never a drain-read (urandom never EOFs).
    if let Ok(mut file) = fs::File::open("/dev/urandom") {
        use std::io::Read;
        let mut out = [0u8; 16];
        if file.read_exact(&mut out).is_ok() {
            return out;
        }
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0) as u64;
    let mut state = nanos ^ (std::process::id() as u64).wrapping_mul(0x9E3779B97F4A7C15);
    let mut out = [0u8; 16];
    for chunk in out.chunks_mut(8) {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        chunk.copy_from_slice(&state.to_le_bytes()[..chunk.len()]);
    }
    out
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::legacy_indexed_palette::legacy_indexed_palette;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "thaum-identity-test-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn user_ids_are_unique_across_generations() {
        let a = SessionIdentity::generate(None);
        let b = SessionIdentity::generate(None);
        assert_ne!(a.user_id, b.user_id);
        assert!(a.user_id.starts_with("u-"));
        assert_eq!(a.user_id.len(), "u-".len() + 32);
    }

    #[test]
    fn identity_persists_and_never_regenerates() {
        let dir = temp_dir("persist");
        let path = dir.join("session-identity.json");
        let first = SessionIdentity::load_or_create(&path, Some("j")).unwrap();
        assert_eq!(first.display_name, "j");
        let second = SessionIdentity::load_or_create(&path, Some("someone-else")).unwrap();
        assert_eq!(first, second);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn presence_color_is_data_from_the_indexed_palette() {
        let palette = legacy_indexed_palette();
        for candidate in PRESENCE_CANDIDATE_COLORS {
            assert!(
                palette.contains(&candidate),
                "presence candidate {candidate:?} must come from the indexed palette"
            );
        }
        let identity = SessionIdentity::generate(None);
        assert!(PRESENCE_CANDIDATE_COLORS.contains(&identity.presence_color));
    }

    #[test]
    fn display_name_defaults_to_a_cosmetic_label() {
        let identity = SessionIdentity::generate(None);
        assert_eq!(identity.display_name, "painter");
        assert_eq!(
            SessionIdentity::generate(Some("alice")).display_name,
            "alice"
        );
    }
}
