//! PBR and billboard texture data shared by renderer backends.

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
