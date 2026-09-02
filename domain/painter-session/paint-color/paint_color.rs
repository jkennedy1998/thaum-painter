use thaum_renderer_domain::{CellColor, CellMaterialId, ColorBand};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaintColor {
    FlatRgb(u8, u8, u8),
    Material(CellMaterialId),
}

impl PaintColor {
    pub const fn flat_rgb(red: u8, green: u8, blue: u8) -> Self {
        Self::FlatRgb(red, green, blue)
    }

    pub const fn material(material: CellMaterialId) -> Self {
        Self::Material(material)
    }

    pub fn to_cell_color(self) -> CellColor {
        match self {
            Self::FlatRgb(red, green, blue) => CellColor::Flat([
                red as f32 / 255.0,
                green as f32 / 255.0,
                blue as f32 / 255.0,
                1.0,
            ]),
            Self::Material(material) => CellColor::Material(material),
        }
    }

    pub fn preview_rgb(self) -> (u8, u8, u8) {
        match self {
            Self::FlatRgb(red, green, blue) => (red, green, blue),
            Self::Material(material) => {
                rgba_to_rgb_tuple(material.resolve_band(ColorBand::MediumLight))
            }
        }
    }

    pub fn label(self) -> String {
        match self {
            Self::FlatRgb(red, green, blue) => format!("#{red:02X}{green:02X}{blue:02X}"),
            Self::Material(material) => material.label().to_string(),
        }
    }
}

impl Default for PaintColor {
    fn default() -> Self {
        Self::flat_rgb(235, 235, 235)
    }
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
    fn material_preview_uses_the_medium_light_band() {
        assert_eq!(
            PaintColor::material(CellMaterialId::GrayScale).preview_rgb(),
            (0xa8, 0xa8, 0xa8)
        );
    }
}
