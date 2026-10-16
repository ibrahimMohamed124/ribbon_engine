//! Render style parameters and cached tone/gamma tables.

use super::{Palette, Tonemap, LEVEL_COUNT};
use crate::RENDER_DIST;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

const LEVEL_LUT_SIZE: usize = 1024;

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
    /// Constant light when no directed source contributes.
    pub ambient: f32,
    /// Secondary point-like light_level around the player.
    pub ambient_fill: f32,
    /// Distance where fog has faded lighting to black.
    pub fog_distance: f32,
    /// Enables sampling material PBR maps where a material supplies a set ID.
    pub pbr_enabled: bool,
    /// Enables optional render assets and effects independently of quality.
    pub billboards_enabled: bool,
    /// Replaces abrupt visual pulses with a small steady light_level change.
    pub limit_flicker: bool,
    /// Cast a voxel DDA ray from lit surfaces back toward the directed source.
    pub spot_light_shadows: bool,
}

impl RenderStyle {
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
    pub(super) tables: Option<Arc<LutTables>>,
    pub(super) bypass: bool,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct LutKey {
    tonemap: (u8, u32),
    gamma_bits: u32,
}

pub(super) struct LutTables {
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
        Self {
            tonemap: Tonemap::None,
            gamma: 1.0,
            palette: Palette::gray(),
            texture_strength: 0.0,
            ao_strength: 0.0,
            ambient: 0.035,
            ambient_fill: 0.12,
            fog_distance: RENDER_DIST,
            pbr_enabled: false,
            billboards_enabled: false,
            limit_flicker: false,
            spot_light_shadows: false,
        }
    }
}
