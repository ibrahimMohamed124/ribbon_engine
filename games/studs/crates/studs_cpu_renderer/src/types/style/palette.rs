//! Palette and tone mapping data.

use ribbon_math::pack_gray;

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
