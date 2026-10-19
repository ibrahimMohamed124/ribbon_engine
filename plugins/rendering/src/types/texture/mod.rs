//! Renderer-neutral texture data, split into PBR and procedural responsibilities.

mod pbr;
mod procedural;

pub use pbr::{
    PbrTextureSet, RgbaMipChain, RgbaMipLevel, SpriteAtlas, TextureColorSpace, TextureId,
    MAX_PBR_MAP_DIMENSION, MAX_PBR_TEXTURE_SETS, PBR_MAPS_PER_SET, PBR_MIP_LEVEL_COUNT,
    PBR_MIP_TEXEL_COUNT,
};
pub use procedural::Texture;

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
