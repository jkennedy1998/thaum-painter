use crate::storage::SharedDocumentDisplayProfile;

/// One named bundle of the four display-profile axes (cell width, cell
/// height, typeface id, spriteset id), applied atomically by the
/// display-profile module's preset switcher. Deliberately a small built-in
/// Rust list rather than a discoverable asset format: with a handful of
/// entries, an asset-discovery seam would be premature machinery for what
/// is still a short, hand-curated list (project 04n3fq3e, J 2026-11-06 —
/// "ideally we can have presets which bake in the typeface, sprite set,
/// cell x and cell y").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayProfilePreset {
    pub label: &'static str,
    pub cell_width: u32,
    pub cell_height: u32,
    pub typeface_id: &'static str,
    pub spriteset_id: &'static str,
}

impl DisplayProfilePreset {
    fn matches(&self, profile: &SharedDocumentDisplayProfile) -> bool {
        self.cell_width == profile.cell_width
            && self.cell_height == profile.cell_height
            && self.typeface_id == profile.typeface_id
            && self.spriteset_id == profile.spriteset_id
    }

    pub fn apply(&self, profile: &mut SharedDocumentDisplayProfile) {
        profile.cell_width = self.cell_width;
        profile.cell_height = self.cell_height;
        profile.typeface_id = self.typeface_id.to_owned();
        profile.spriteset_id = self.spriteset_id.to_owned();
    }
}

/// The index of the preset exactly matching `profile`'s four fields, if any.
pub fn matching_preset_index(
    presets: &[DisplayProfilePreset],
    profile: &SharedDocumentDisplayProfile,
) -> Option<usize> {
    presets.iter().position(|preset| preset.matches(profile))
}

/// The built-in presets: the legacy ThaumMono default, and JetBrains Mono
/// tuned to its own measured natural ratio (5:11, measured 2026-11-06
/// through fontdue's `horizontal_line_metrics`/monospace advance width — the
/// same metrics the renderer's own font-fit step uses). JetBrains Mono
/// defaults to no spriteset override: the bundled monothaum sprites are
/// drawn for ThaumMono's 12:16 box, and stretching them into JetBrains
/// Mono's much taller 5:11 box would distort them rather than complement
/// the typeface.
pub fn built_in_display_profile_presets() -> &'static [DisplayProfilePreset] {
    &[
        DisplayProfilePreset {
            label: "thaum-mono",
            cell_width: 12,
            cell_height: 16,
            typeface_id: "thaum-mono",
            spriteset_id: "monothaum-atlas-v3",
        },
        DisplayProfilePreset {
            label: "jetbrains-mono",
            cell_width: 10,
            cell_height: 22,
            typeface_id: "jetbrains-mono",
            spriteset_id: "none",
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(
        cell_width: u32,
        cell_height: u32,
        typeface_id: &str,
        spriteset_id: &str,
    ) -> SharedDocumentDisplayProfile {
        SharedDocumentDisplayProfile {
            cell_width,
            cell_height,
            typeface_id: typeface_id.to_owned(),
            spriteset_id: spriteset_id.to_owned(),
        }
    }

    #[test]
    fn applying_a_preset_sets_all_four_fields_atomically() {
        let preset = built_in_display_profile_presets()[1];
        let mut live = profile(12, 16, "thaum-mono", "monothaum-atlas-v3");

        preset.apply(&mut live);

        assert_eq!(live, profile(10, 22, "jetbrains-mono", "none"));
    }

    #[test]
    fn matching_preset_index_finds_an_exact_four_field_match() {
        let presets = built_in_display_profile_presets();
        let live = profile(10, 22, "jetbrains-mono", "none");

        assert_eq!(matching_preset_index(presets, &live), Some(1));
    }

    #[test]
    fn matching_preset_index_is_none_for_a_hand_edited_combination() {
        let presets = built_in_display_profile_presets();
        let live = profile(10, 22, "jetbrains-mono", "monothaum-atlas-v3");

        assert_eq!(matching_preset_index(presets, &live), None);
    }
}
