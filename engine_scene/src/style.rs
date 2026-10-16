//! Color management and materials applied after lighting.

use crate::RENDER_DIST;
use engine_core::math::pack_gray;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

pub const LEVEL_COUNT: usize = 14;
const LEVEL_LUT_SIZE: usize = 1024;

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
    /// Constant world illumination with the flashlight off.
    pub ambient: f32,
    /// Secondary point-like illumination around the player.
    pub lamp: f32,
    /// Distance where fog has faded lighting to black.
    pub fog_distance: f32,
    /// Enables sampling material PBR maps where a material supplies a set ID.
    pub pbr_enabled: bool,
    /// Enables realistic-only assets and effects independently of quality.
    pub realistic_assets: bool,
    /// Replaces abrupt visual pulses with a small steady illumination change.
    pub reduce_flashing: bool,
    /// Cast a voxel DDA ray from lit surfaces back toward the flashlight.
    pub flashlight_shadows: bool,
}

impl RenderStyle {
    pub fn classic() -> Self {
        Self {
            tonemap: Tonemap::None,
            gamma: 1.0,
            palette: Palette::gray(),
            texture_strength: 0.0,
            ao_strength: 0.0,
            ambient: 0.035,
            lamp: 0.12,
            fog_distance: RENDER_DIST,
            pbr_enabled: false,
            realistic_assets: false,
            reduce_flashing: false,
            flashlight_shadows: false,
        }
    }

    pub fn moody() -> Self {
        Self {
            tonemap: Tonemap::Exp { exposure: 2.0 },
            gamma: 1.6,
            palette: Palette::ramp(0x05_08_06, 0xcf_e3_d4),
            texture_strength: 0.6,
            ao_strength: 0.45,
            ambient: 0.010,
            lamp: 0.12,
            fog_distance: RENDER_DIST,
            pbr_enabled: false,
            realistic_assets: false,
            reduce_flashing: false,
            flashlight_shadows: false,
        }
    }

    /// Color-graded preset for the new PBR path. Legacy styles remain byte-identical.
    pub fn realistic() -> Self {
        Self {
            tonemap: Tonemap::Exp { exposure: 1.45 },
            gamma: 1.0,
            palette: Palette::ramp(0x02_0b_0b, 0xb9_c1_b8),
            texture_strength: 0.0,
            ao_strength: 0.72,
            ambient: 0.006,
            lamp: 0.16,
            fog_distance: RENDER_DIST * 0.82,
            pbr_enabled: true,
            realistic_assets: true,
            reduce_flashing: false,
            flashlight_shadows: true,
        }
    }

    /// Continuous palette level after tone mapping and display gamma.
    /// Vignette, grain, and dithering are deliberately excluded.
    pub fn level_for(&self, gray: f32) -> f32 {
        if !gray.is_finite() {
            return 0.0;
        }
        let mapped = self.tonemap.apply(gray).clamp(0.0, 1.0);
        let gamma = if self.gamma.is_finite() && self.gamma > 0.0 {
            self.gamma
        } else {
            1.0
        };
        let encoded = if gamma == 1.0 {
            mapped
        } else {
            mapped.powf(1.0 / gamma)
        };
        encoded * (LEVEL_COUNT - 1) as f32
    }

    #[doc(hidden)]
    pub fn level_lut(&self) -> LevelLut {
        let bypass = matches!(self.tonemap, Tonemap::None) && self.gamma == 1.0;
        if bypass {
            return LevelLut {
                tables: None,
                bypass: true,
            };
        }
        let key = LutKey {
            tonemap: match self.tonemap {
                Tonemap::None => (0, 0),
                Tonemap::Exp { exposure } => (1, exposure.to_bits()),
            },
            gamma_bits: self.gamma.to_bits(),
        };
        static LUTS: OnceLock<Mutex<HashMap<LutKey, Arc<LutTables>>>> = OnceLock::new();
        let cache = LUTS.get_or_init(|| Mutex::new(HashMap::new()));
        let tables = {
            let mut cache = cache
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(tables) = cache.get(&key) {
                Arc::clone(tables)
            } else {
                let tables = Arc::new(LutTables {
                    tone: self.build_tone_table(),
                    gamma: self.build_gamma_table(),
                });
                // Styles are user-configurable; cap retained entries to avoid unbounded growth.
                if cache.len() >= 8 {
                    if let Some(oldest) = cache.keys().next().copied() {
                        cache.remove(&oldest);
                    }
                }
                cache.insert(key, Arc::clone(&tables));
                tables
            }
        };
        LevelLut {
            tables: Some(tables),
            bypass: false,
        }
    }

    fn build_tone_table(&self) -> [f32; LEVEL_LUT_SIZE] {
        std::array::from_fn(|i| {
            self.tonemap
                .apply(i as f32 * 2.0 / (LEVEL_LUT_SIZE - 1) as f32)
                .clamp(0.0, 1.0)
        })
    }

    fn build_gamma_table(&self) -> [f32; LEVEL_LUT_SIZE] {
        let gamma = if self.gamma.is_finite() && self.gamma > 0.0 {
            self.gamma
        } else {
            1.0
        };
        std::array::from_fn(|i| {
            let mapped = i as f32 / (LEVEL_LUT_SIZE - 1) as f32;
            let encoded = if gamma == 1.0 {
                mapped
            } else {
                mapped.powf(1.0 / gamma)
            };
            encoded * (LEVEL_COUNT - 1) as f32
        })
    }
}

#[doc(hidden)]
pub struct LevelLut {
    tables: Option<Arc<LutTables>>,
    bypass: bool,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct LutKey {
    tonemap: (u8, u32),
    gamma_bits: u32,
}

struct LutTables {
    tone: [f32; LEVEL_LUT_SIZE],
    gamma: [f32; LEVEL_LUT_SIZE],
}

impl LevelLut {
    fn sample(table: &[f32; LEVEL_LUT_SIZE], value: f32, max: f32) -> f32 {
        let position = value.clamp(0.0, max) * (LEVEL_LUT_SIZE - 1) as f32 / max;
        let low = position.floor() as usize;
        let high = (low + 1).min(LEVEL_LUT_SIZE - 1);
        let fraction = position - low as f32;
        table[low] * (1.0 - fraction) + table[high] * fraction
    }

    pub fn tone(&self, style: &RenderStyle, gray: f32) -> f32 {
        if !gray.is_finite() {
            return 0.0;
        }
        if self.bypass {
            return style.tonemap.apply(gray);
        }
        Self::sample(&self.tables.as_ref().expect("tone table").tone, gray, 2.0)
    }

    pub fn level(&self, gray: f32) -> f32 {
        if !gray.is_finite() {
            return 0.0;
        }
        if self.bypass {
            return gray.clamp(0.0, 1.0) * (LEVEL_COUNT - 1) as f32;
        }
        Self::sample(&self.tables.as_ref().expect("gamma table").gamma, gray, 1.0)
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
    use engine_core::math::pack_gray;
    use std::sync::Arc;

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

    #[test]
    fn level_lut_tracks_exact_levels_and_classic_bypasses_it() {
        let style = RenderStyle::moody();
        let lut = style.level_lut();
        for i in 0..=1000 {
            let gray = i as f32 * 1.9 / 1000.0;
            let exact = style.level_for(gray);
            let sampled = lut.level(lut.tone(&style, gray));
            assert!((sampled - exact).abs() < 0.02);
        }
        assert!(RenderStyle::classic().level_lut().bypass);
    }

    #[test]
    fn equal_styles_share_tables_but_gamma_changes_do_not() {
        let first = RenderStyle::moody().level_lut();
        let second = RenderStyle::moody().level_lut();
        assert!(Arc::ptr_eq(
            first.tables.as_ref().unwrap(),
            second.tables.as_ref().unwrap()
        ));

        let mut changed_gamma = RenderStyle::moody();
        changed_gamma.gamma += 0.1;
        let changed = changed_gamma.level_lut();
        assert!(!Arc::ptr_eq(
            first.tables.as_ref().unwrap(),
            changed.tables.as_ref().unwrap()
        ));
    }

    #[test]
    fn level_for_is_monotonic_safe_and_identity_for_classic() {
        let classic = RenderStyle::classic();
        for gray in [0.0, 0.1, 0.55, 1.0, 1.7] {
            assert!((classic.level_for(gray) / 13.0 - gray.clamp(0.0, 1.0)).abs() < 1e-6);
        }
        let moody = RenderStyle::moody();
        let levels: Vec<_> = (0..=1000)
            .map(|i| moody.level_for(i as f32 / 500.0))
            .collect();
        assert!(levels.windows(2).all(|pair| pair[0] <= pair[1]));
        assert_eq!(moody.level_for(f32::NAN), 0.0);
        assert_eq!(moody.level_for(f32::INFINITY), 0.0);
    }
}
