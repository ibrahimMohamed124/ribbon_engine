//! Small deterministic grayscale textures for software-rendered materials.

use crate::math::hash4;

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
            // This texture represents the interior of one classic brick. The
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
    use super::Texture;

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
}
