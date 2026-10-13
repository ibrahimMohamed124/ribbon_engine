//! Color management and materials applied after lighting.

use crate::math::pack_gray;

pub const LEVEL_COUNT: usize = 14;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tonemap {
    None,
    Exp { exposure: f32 },
}

impl Tonemap {
    pub fn apply(self, value: f32) -> f32 {
        if !value.is_finite() {
            return 0.0;
        }
        match self {
            Self::None => value,
            Self::Exp { exposure } => {
                let exposure = if exposure.is_finite() {
                    exposure.max(0.0)
                } else {
                    0.0
                };
                1.0 - (-value.max(0.0) * exposure).exp()
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    colors: [u32; LEVEL_COUNT],
}

impl Palette {
    pub fn gray() -> Self {
        Self {
            colors: std::array::from_fn(|i| pack_gray(i as f32 / (LEVEL_COUNT - 1) as f32)),
        }
    }

    /// Build a 14-step ramp by linearly interpolating each RGB channel.
    pub fn ramp(dark_rgb: u32, light_rgb: u32) -> Self {
        let channel = |rgb: u32, shift: u32| ((rgb >> shift) & 0xff) as f32;
        let colors = std::array::from_fn(|i| {
            let t = i as f32 / (LEVEL_COUNT - 1) as f32;
            let lerp = |shift| {
                (channel(dark_rgb, shift) * (1.0 - t) + channel(light_rgb, shift) * t) as u32
            };
            (lerp(16) << 16) | (lerp(8) << 8) | lerp(0)
        });
        Self { colors }
    }

    pub fn colors(&self) -> &[u32; LEVEL_COUNT] {
        &self.colors
    }

    pub fn color(&self, index: usize) -> u32 {
        self.colors[index.min(LEVEL_COUNT - 1)]
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderStyle {
    pub tonemap: Tonemap,
    /// Display gamma. 1.0 leaves linear values unchanged.
    pub gamma: f32,
    pub palette: Palette,
    /// Blend procedural surface detail into the game's base materials.
    pub texture_strength: f32,
    /// Blend voxel face occlusion into world surfaces.
    pub ao_strength: f32,
}

impl RenderStyle {
    pub fn classic() -> Self {
        Self {
            tonemap: Tonemap::None,
            gamma: 1.0,
            palette: Palette::gray(),
            texture_strength: 0.0,
            ao_strength: 0.0,
        }
    }

    pub fn moody() -> Self {
        Self {
            tonemap: Tonemap::Exp { exposure: 1.15 },
            gamma: 2.2,
            palette: Palette::ramp(0x05_08_06, 0xcf_e3_d4),
            texture_strength: 0.55,
            ao_strength: 0.45,
        }
    }
}

impl Default for RenderStyle {
    fn default() -> Self {
        Self::classic()
    }
}

#[cfg(test)]
mod tests {
    use super::{Palette, RenderStyle, Tonemap, LEVEL_COUNT};
    use crate::math::pack_gray;

    #[test]
    fn gray_palette_matches_the_legacy_packing() {
        let palette = Palette::gray();
        for (i, &color) in palette.colors().iter().enumerate() {
            assert_eq!(color, pack_gray(i as f32 / 13.0));
        }
        assert_eq!(palette.colors().len(), LEVEL_COUNT);
    }

    #[test]
    fn ramp_has_monotonic_luminance() {
        let colors = Palette::ramp(0x05_08_06, 0xcf_e3_d4);
        let luminance = |color: u32| {
            0.2126 * ((color >> 16) & 0xff) as f32
                + 0.7152 * ((color >> 8) & 0xff) as f32
                + 0.0722 * (color & 0xff) as f32
        };
        assert!(colors
            .colors()
            .windows(2)
            .all(|pair| luminance(pair[0]) <= luminance(pair[1])));
    }

    #[test]
    fn exponential_tonemap_is_monotonic_and_bounded() {
        let tone = Tonemap::Exp { exposure: 1.2 };
        assert_eq!(tone.apply(0.0), 0.0);
        let values: Vec<_> = (0..=100).map(|i| tone.apply(i as f32 / 10.0)).collect();
        assert!(values.windows(2).all(|pair| pair[0] <= pair[1]));
        assert!(values.iter().all(|value| (0.0..=1.0).contains(value)));
        assert_eq!(tone.apply(f32::NAN), 0.0);
        assert_eq!(tone.apply(f32::INFINITY), 0.0);
    }

    #[test]
    fn classic_is_the_default_style() {
        assert_eq!(RenderStyle::default(), RenderStyle::classic());
    }
}
