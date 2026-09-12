use thaum_renderer_domain::{CellColor, CellColorSlot, CellMaterialId, ColorBand};

/// Painter-owned portable cell color. Asset-backed values retain their relative
/// filenames; renderer-specific material ids are resolved only at handoff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaintColor {
    FlatRgb(u8, u8, u8),
    Material {
        asset_file: String,
    },
    Slots {
        a: PaintColorSlot,
        b: PaintColorSlot,
        c: PaintColorSlot,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaintColorSlot {
    FlatRgb(u8, u8, u8),
    Material { asset_file: String },
}

impl PaintColor {
    pub const GRAY_SCALE_ASSET_FILE: &'static str = "materials/gray-scale.json";

    pub const fn flat_rgb(red: u8, green: u8, blue: u8) -> Self {
        Self::FlatRgb(red, green, blue)
    }

    /// Transitional convenience for the currently available built-in material.
    /// New authored values should use [`Self::material_asset`], which preserves
    /// the portable asset filename directly.
    pub fn material(material: CellMaterialId) -> Self {
        Self::material_asset(material_asset_file(material))
    }

    pub fn material_asset(asset_file: impl Into<String>) -> Self {
        Self::Material {
            asset_file: asset_file.into(),
        }
    }

    pub const fn flat_slot(red: u8, green: u8, blue: u8) -> PaintColorSlot {
        PaintColorSlot::FlatRgb(red, green, blue)
    }

    pub fn material_slot(asset_file: impl Into<String>) -> PaintColorSlot {
        PaintColorSlot::Material {
            asset_file: asset_file.into(),
        }
    }

    pub const fn slots(a: PaintColorSlot, b: PaintColorSlot, c: PaintColorSlot) -> Self {
        Self::Slots { a, b, c }
    }

    /// Temporary built-in-only preview mapping. The authored value stays a
    /// filename even if the current renderer cannot resolve it yet.
    pub fn to_cell_color(&self) -> CellColor {
        match self {
            Self::FlatRgb(red, green, blue) => flat_renderer_color(*red, *green, *blue),
            Self::Material { asset_file } => CellColor::Material(resolve_material(asset_file)),
            Self::Slots { a, b, c } => CellColor::slots(
                a.to_cell_color_slot(),
                b.to_cell_color_slot(),
                c.to_cell_color_slot(),
            ),
        }
    }

    pub fn preview_rgb(&self) -> (u8, u8, u8) {
        match self {
            Self::FlatRgb(red, green, blue) => (*red, *green, *blue),
            Self::Material { asset_file } => {
                rgba_to_rgb_tuple(resolve_material(asset_file).resolve_band(ColorBand::MediumLight))
            }
            Self::Slots { a, .. } => a.preview_rgb(),
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::FlatRgb(red, green, blue) => format!("#{red:02X}{green:02X}{blue:02X}"),
            Self::Material { asset_file } => asset_file.clone(),
            Self::Slots { .. } => "color slots".to_string(),
        }
    }
}

impl PaintColorSlot {
    fn to_cell_color_slot(&self) -> CellColorSlot {
        match self {
            Self::FlatRgb(red, green, blue) => CellColorSlot::Flat(flat_rgba(*red, *green, *blue)),
            Self::Material { asset_file } => CellColorSlot::Material(resolve_material(asset_file)),
        }
    }

    fn preview_rgb(&self) -> (u8, u8, u8) {
        match self {
            Self::FlatRgb(red, green, blue) => (*red, *green, *blue),
            Self::Material { asset_file } => {
                rgba_to_rgb_tuple(resolve_material(asset_file).resolve_band(ColorBand::MediumLight))
            }
        }
    }
}

impl Default for PaintColor {
    fn default() -> Self {
        Self::flat_rgb(235, 235, 235)
    }
}

fn material_asset_file(material: CellMaterialId) -> &'static str {
    match material {
        CellMaterialId::GrayScale => PaintColor::GRAY_SCALE_ASSET_FILE,
    }
}

fn resolve_material(asset_file: &str) -> CellMaterialId {
    match asset_file {
        PaintColor::GRAY_SCALE_ASSET_FILE | "gray-scale" => CellMaterialId::GrayScale,
        // The renderer registry is the next step. Keep unknown source values
        // intact here; this temporary preview falls back to the built-in.
        _ => CellMaterialId::GrayScale,
    }
}

fn flat_renderer_color(red: u8, green: u8, blue: u8) -> CellColor {
    CellColor::Flat(flat_rgba(red, green, blue))
}

fn flat_rgba(red: u8, green: u8, blue: u8) -> [f32; 4] {
    [
        red as f32 / 255.0,
        green as f32 / 255.0,
        blue as f32 / 255.0,
        1.0,
    ]
}

fn rgba_to_rgb_tuple(rgba: [f32; 4]) -> (u8, u8, u8) {
    (
        (rgba[0] * 255.0).round().clamp(0.0, 255.0) as u8,
        (rgba[1] * 255.0).round().clamp(0.0, 255.0) as u8,
        (rgba[2] * 255.0).round().clamp(0.0, 255.0) as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_rgb_round_trips_into_a_renderer_flat_color() {
        assert_eq!(
            PaintColor::flat_rgb(1, 2, 3).to_cell_color(),
            CellColor::Flat([1.0 / 255.0, 2.0 / 255.0, 3.0 / 255.0, 1.0])
        );
    }

    #[test]
    fn material_assets_and_slots_keep_their_portable_filenames() {
        let color = PaintColor::slots(
            PaintColor::material_slot("materials/fire.json"),
            PaintColor::flat_slot(1, 2, 3),
            PaintColor::material_slot("materials/smoke.json"),
        );
        assert_eq!(
            color,
            PaintColor::Slots {
                a: PaintColorSlot::Material {
                    asset_file: "materials/fire.json".to_string()
                },
                b: PaintColorSlot::FlatRgb(1, 2, 3),
                c: PaintColorSlot::Material {
                    asset_file: "materials/smoke.json".to_string()
                },
            }
        );
    }

    #[test]
    fn material_preview_uses_the_medium_light_band() {
        assert_eq!(
            PaintColor::material(CellMaterialId::GrayScale).preview_rgb(),
            (0xa8, 0xa8, 0xa8)
        );
    }
}
