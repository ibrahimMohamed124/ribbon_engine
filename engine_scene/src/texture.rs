//! Small deterministic grayscale textures for software-rendered materials.

use engine_core::math::hash4;

/// Stable renderer-independent identifier for a decoded PBR texture set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TextureId(pub u32);

/// How the RGB channels of an RGBA mip chain should be interpreted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextureColorSpace {
    /// Base color is stored in sRGB bytes and converted to linear values on sample.
    Srgb,
    /// Normal, roughness, AO, and other data maps are linear byte values.
    Linear,
}

/// One tightly packed RGBA8 texture level.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbaMipLevel {
    pub width: usize,
    pub height: usize,
    pub texels: Vec<[u8; 4]>,
}

impl RgbaMipLevel {
    pub fn new(width: usize, height: usize, texels: Vec<[u8; 4]>) -> Self {
        let (width, height) = (width.max(1), height.max(1));
        assert_eq!(texels.len(), width * height);
        Self {
            width,
            height,
            texels,
        }
    }
}

/// Plain-data RGBA8 mip chain, suitable for either renderer backend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbaMipChain {
    pub color_space: TextureColorSpace,
    pub levels: Vec<RgbaMipLevel>,
}

impl RgbaMipChain {
    pub fn new(base: RgbaMipLevel, color_space: TextureColorSpace) -> Self {
        let mut levels = vec![base];
        while levels
            .last()
            .is_some_and(|level| level.width > 1 || level.height > 1)
        {
            let previous = levels.last().expect("base mip exists");
            let width = (previous.width / 2).max(1);
            let height = (previous.height / 2).max(1);
            let mut texels = Vec::with_capacity(width * height);
            for y in 0..height {
                for x in 0..width {
                    let samples = [
                        previous.texels[(y * 2).min(previous.height - 1) * previous.width
                            + (x * 2).min(previous.width - 1)],
                        previous.texels[(y * 2).min(previous.height - 1) * previous.width
                            + (x * 2 + 1).min(previous.width - 1)],
                        previous.texels[((y * 2 + 1).min(previous.height - 1)) * previous.width
                            + (x * 2).min(previous.width - 1)],
                        previous.texels[((y * 2 + 1).min(previous.height - 1)) * previous.width
                            + (x * 2 + 1).min(previous.width - 1)],
                    ];
                    let mut out = [0u8; 4];
                    for channel in 0..3 {
                        let values = samples.map(|sample| sample[channel] as f32 / 255.0);
                        let average = match color_space {
                            TextureColorSpace::Srgb => {
                                values.map(srgb_to_linear).iter().sum::<f32>() / 4.0
                            }
                            TextureColorSpace::Linear => values.iter().sum::<f32>() / 4.0,
                        };
                        out[channel] = (match color_space {
                            TextureColorSpace::Srgb => linear_to_srgb(average),
                            TextureColorSpace::Linear => average,
                        } * 255.0)
                            .round()
                            .clamp(0.0, 255.0) as u8;
                    }
                    out[3] = ((samples.iter().map(|sample| sample[3] as u32).sum::<u32>() + 2) / 4)
                        as u8;
                    texels.push(out);
                }
            }
            levels.push(RgbaMipLevel::new(width, height, texels));
        }
        Self {
            color_space,
            levels,
        }
    }

    /// Wrapped bilinear sampling with interpolation between neighboring mip levels.
    pub fn sample(&self, u: f32, v: f32, lod: f32) -> [f32; 4] {
        let max_lod = self.levels.len().saturating_sub(1) as f32;
        let lod = if lod.is_finite() {
            lod.clamp(0.0, max_lod)
        } else {
            max_lod
        };
        let low = lod.floor() as usize;
        let high = (low + 1).min(self.levels.len() - 1);
        let t = lod - low as f32;
        let a = self.sample_level(low, u, v);
        let b = self.sample_level(high, u, v);
        std::array::from_fn(|i| a[i] * (1.0 - t) + b[i] * t)
    }

    fn sample_level(&self, level: usize, u: f32, v: f32) -> [f32; 4] {
        let mip = &self.levels[level];
        if !u.is_finite() || !v.is_finite() {
            return [0.0; 4];
        }
        let x = u.rem_euclid(1.0) * mip.width as f32 - 0.5;
        let y = v.rem_euclid(1.0) * mip.height as f32 - 0.5;
        let x0 = x.floor() as isize;
        let y0 = y.floor() as isize;
        let tx = x - x.floor();
        let ty = y - y.floor();
        let fetch = |x: isize, y: isize| {
            let ix = x.rem_euclid(mip.width as isize) as usize;
            let iy = y.rem_euclid(mip.height as isize) as usize;
            let texel = mip.texels[iy * mip.width + ix];
            let mut rgba = [0.0; 4];
            for i in 0..4 {
                let value = texel[i] as f32 / 255.0;
                rgba[i] = if i < 3 && self.color_space == TextureColorSpace::Srgb {
                    srgb_to_linear(value)
                } else {
                    value
                };
            }
            rgba
        };
        let p00 = fetch(x0, y0);
        let p10 = fetch(x0 + 1, y0);
        let p01 = fetch(x0, y0 + 1);
        let p11 = fetch(x0 + 1, y0 + 1);
        std::array::from_fn(|i| {
            let top = p00[i] * (1.0 - tx) + p10[i] * tx;
            let bottom = p01[i] * (1.0 - tx) + p11[i] * tx;
            top * (1.0 - ty) + bottom * ty
        })
    }
}

/// Albedo and data maps for one material, kept as CPU-owned plain data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PbrTextureSet {
    pub id: TextureId,
    pub albedo: RgbaMipChain,
    pub normal: RgbaMipChain,
    pub roughness: RgbaMipChain,
    pub ambient_occlusion: RgbaMipChain,
}

/// Compact baked billboard atlas shared by both rendering backends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpriteAtlas {
    pub width: usize,
    pub height: usize,
    pub cell_width: usize,
    pub cell_height: usize,
    pub views: usize,
    pub states: usize,
    pub frames_per_state: usize,
    pub albedo: Vec<[u8; 4]>,
    pub normal: Vec<[u8; 4]>,
    /// One byte per atlas texel; white is emissive.
    pub emissive: Vec<u8>,
}

impl SpriteAtlas {
    pub fn is_valid(&self) -> bool {
        self.width == self.cell_width * self.views
            && self.height == self.cell_height * self.states * self.frames_per_state
            && self.albedo.len() == self.width * self.height
            && self.normal.len() == self.width * self.height
            && self.emissive.len() == self.width * self.height
            && self.views > 0
            && self.states > 0
            && self.frames_per_state > 0
    }

    pub fn texel(
        &self,
        view: usize,
        state: usize,
        frame: usize,
        u: f32,
        v: f32,
    ) -> Option<([u8; 4], [u8; 4], u8)> {
        if !self.is_valid() || !u.is_finite() || !v.is_finite() {
            return None;
        }
        let view = view % self.views;
        let state = state.min(self.states - 1);
        let frame = frame % self.frames_per_state;
        let x = (u.clamp(0.0, 0.999_999) * self.cell_width as f32) as usize;
        let y = (v.clamp(0.0, 0.999_999) * self.cell_height as f32) as usize;
        let index =
            (state * self.frames_per_state * self.cell_height + frame * self.cell_height + y)
                * self.width
                + view * self.cell_width
                + x;
        Some((self.albedo[index], self.normal[index], self.emissive[index]))
    }
}

fn srgb_to_linear(value: f32) -> f32 {
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(value: f32) -> f32 {
    if value <= 0.0031308 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Texture {
    pub w: usize,
    pub h: usize,
    pub texels: Vec<f32>,
    mean: f32,
}

impl Texture {
    pub fn new(w: usize, h: usize, texels: Vec<f32>) -> Self {
        let (w, h) = (w.max(1), h.max(1));
        assert_eq!(texels.len(), w * h);
        let mean = texels.iter().sum::<f32>() / texels.len() as f32;
        Self { w, h, texels, mean }
    }

    pub fn bricks(w: usize, h: usize, seed: i32) -> Self {
        Self::generate(w, h, |x, y, _, _| {
            // This texture represents the interior of one representative brick. The
            // game maps it to 1 unit horizontally by 0.5 units vertically,
            // while mortar and half-offset rows remain in the material shader.
            let grain = hash4(x as i32, y as i32, seed, 1);
            let chip = (grain > 0.985) as u8 as f32 * 0.08;
            (0.50 + grain * 0.08 - chip).clamp(0.38, 0.62)
        })
    }

    pub fn tiles(w: usize, h: usize, seed: i32) -> Self {
        Self::generate(w, h, |x, y, w, h| {
            let cols = 4usize;
            let rows = 4usize;
            let tile_x = (x * cols / w) as i32;
            let tile_y = (y * rows / h) as i32;
            let local_x = (x * cols % w) as f32 / w as f32;
            let local_y = (y * rows % h) as f32 / h as f32;
            if !(0.035..=0.965).contains(&local_x) || !(0.035..=0.965).contains(&local_y) {
                0.24
            } else {
                let variation = hash4(tile_x, tile_y, seed, 2) * 0.09;
                (0.47 + variation).clamp(0.30, 0.64)
            }
        })
    }

    pub fn noise(w: usize, h: usize, seed: i32) -> Self {
        let (w, h) = (w.max(2), h.max(2));
        let period_w = (w - 1) as i32;
        let period_h = (h - 1) as i32;
        let mut texels = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                let px = (x as i32).rem_euclid(period_w);
                let py = (y as i32).rem_euclid(period_h);
                texels.push(0.46 + hash4(px, py, seed, 3) * 0.12);
            }
        }
        Self::new_seamless(w, h, texels)
    }

    pub fn sample_nearest(&self, u: f32, v: f32) -> f32 {
        if !u.is_finite() || !v.is_finite() {
            return self.average();
        }
        let u = u - u.floor();
        let v = v - v.floor();
        let x = ((u * self.w as f32) as usize).min(self.w - 1);
        let y = ((v * self.h as f32) as usize).min(self.h - 1);
        self.texels[y * self.w + x]
    }

    pub fn average(&self) -> f32 {
        self.mean
    }

    /// Fade detail toward the texture mean at long range to limit shimmer.
    pub fn sample_faded(
        &self,
        u: f32,
        v: f32,
        distance: f32,
        fade_start: f32,
        fade_end: f32,
    ) -> f32 {
        let sample = self.sample_nearest(u, v);
        let mean = self.average();
        let t = if !distance.is_finite() {
            1.0
        } else if fade_end <= fade_start {
            f32::from(distance >= fade_end)
        } else {
            ((distance - fade_start) / (fade_end - fade_start)).clamp(0.0, 1.0)
        };
        sample * (1.0 - t) + mean * t
    }

    fn generate(
        w: usize,
        h: usize,
        mut texel: impl FnMut(usize, usize, usize, usize) -> f32,
    ) -> Self {
        let (w, h) = (w.max(1), h.max(1));
        let texels = (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .map(|(x, y)| texel(x, y, w, h))
            .collect();
        Self::new_seamless(w, h, texels)
    }

    fn new_seamless(w: usize, h: usize, mut texels: Vec<f32>) -> Self {
        if w > 1 {
            for y in 0..h {
                let row = y * w;
                texels[row + w - 1] = texels[row];
            }
        }
        if h > 1 {
            let last = (h - 1) * w;
            for x in 0..w {
                texels[last + x] = texels[x];
            }
        }
        Self::new(w, h, texels)
    }
}

#[cfg(test)]
mod tests {
    use super::{RgbaMipChain, RgbaMipLevel, Texture, TextureColorSpace};

    #[test]
    fn nearest_sampling_wraps_negative_and_overflowing_uvs() {
        let texture = Texture::new(2, 2, vec![0.1, 0.2, 0.3, 0.4]);
        assert_eq!(
            texture.sample_nearest(0.0, 0.0),
            texture.sample_nearest(1.0, 1.0)
        );
        assert_eq!(
            texture.sample_nearest(-0.5, 0.0),
            texture.sample_nearest(0.5, 0.0)
        );
        assert_eq!(
            texture.sample_nearest(1.5, 0.5),
            texture.sample_nearest(0.5, 0.5)
        );
    }

    #[test]
    fn fade_converges_to_average() {
        let texture = Texture::noise(8, 8, 42);
        assert_eq!(
            texture.sample_faded(0.17, 0.28, 20.0, 5.0, 10.0),
            texture.average()
        );
        assert_eq!(
            texture.sample_faded(0.17, 0.28, 5.0, 5.0, 10.0),
            texture.sample_nearest(0.17, 0.28)
        );
    }

    #[test]
    fn procedural_textures_are_seeded_and_repeatable() {
        assert_eq!(Texture::bricks(32, 32, 7), Texture::bricks(32, 32, 7));
        assert_eq!(Texture::tiles(32, 32, 7), Texture::tiles(32, 32, 7));
        assert_eq!(Texture::noise(32, 32, 7), Texture::noise(32, 32, 7));
    }

    #[test]
    fn procedural_textures_match_at_wrapping_edges() {
        for texture in [
            Texture::bricks(32, 32, 3),
            Texture::tiles(32, 32, 3),
            Texture::noise(32, 32, 3),
        ] {
            for t in [0.0, 0.25, 0.5, 0.75] {
                assert_eq!(
                    texture.sample_nearest(0.0, t),
                    texture.sample_nearest(0.999, t)
                );
                assert_eq!(
                    texture.sample_nearest(t, 0.0),
                    texture.sample_nearest(t, 0.999)
                );
            }
        }
    }

    #[test]
    fn rgba_mips_reach_one_texel_and_keep_linear_data() {
        let chain = RgbaMipChain::new(
            RgbaMipLevel::new(2, 2, vec![[0, 64, 128, 255]; 4]),
            TextureColorSpace::Linear,
        );
        assert_eq!(chain.levels.len(), 2);
        assert_eq!(chain.levels[1].texels, vec![[0, 64, 128, 255]]);
        assert_eq!(
            chain.sample(-0.25, 2.25, 0.0),
            chain.sample(0.75, 0.25, 0.0)
        );
    }

    #[test]
    fn srgb_mips_average_in_linear_light_and_samples_decode_to_linear() {
        let chain = RgbaMipChain::new(
            RgbaMipLevel::new(
                2,
                2,
                vec![
                    [0, 0, 0, 255],
                    [255, 255, 255, 255],
                    [0, 0, 0, 255],
                    [255, 255, 255, 255],
                ],
            ),
            TextureColorSpace::Srgb,
        );
        assert!((chain.sample(0.25, 0.25, 1.0)[0] - 0.5).abs() < 0.01);
        assert!(chain.sample(0.25, 0.25, 0.0)[0] < 0.01);
        assert!((chain.sample(0.75, 0.25, 0.0)[0] - 1.0).abs() < 0.01);
    }
}
