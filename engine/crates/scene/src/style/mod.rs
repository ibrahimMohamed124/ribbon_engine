//! Color and post-lighting controls, split by data responsibility.

mod effects;
mod palette;
mod render_style;

pub use effects::screen_warp;
pub use palette::{Palette, Tonemap, LEVEL_COUNT};
pub use render_style::{LevelLut, RenderStyle};

#[cfg(test)]
mod tests {
    use super::{Palette, RenderStyle, Tonemap, LEVEL_COUNT};
    use voxelworks_core::math::pack_gray;

    fn nonlinear_style() -> RenderStyle {
        RenderStyle {
            tonemap: Tonemap::Exp { exposure: 2.0 },
            gamma: 1.6,
            palette: Palette::ramp(0x05_08_06, 0xcf_e3_d4),
            texture_strength: 0.6,
            ao_strength: 0.45,
            ambient: 0.010,
            ..RenderStyle::default()
        }
    }
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
    fn default_style_is_well_formed() {
        assert_eq!(RenderStyle::default().gamma, 1.0);
    }

    #[test]
    fn screen_warp_is_bounded_finite_and_accessibility_aware() {
        for intensity in [-5.0, 0.0, 0.25, 1.0, 5.0] {
            let offset = super::screen_warp(1.0, -1.0, 100.0, intensity);
            assert!(offset
                .iter()
                .all(|value| value.is_finite() && value.abs() <= 0.003));
            assert_eq!(super::screen_warp(1.0, -1.0, 100.0, 0.0), [0.0; 2]);
        }
        assert_eq!(super::screen_warp(f32::NAN, 0.0, 0.0, 1.0), [0.0; 2]);
    }

    #[test]
    fn level_lut_tracks_exact_levels_and_default_bypasses_it() {
        let style = nonlinear_style();
        let lut = style.level_lut();
        for i in 0..=1000 {
            let gray = i as f32 * 1.9 / 1000.0;
            let exact = style.level_for(gray);
            let sampled = lut.level(lut.tone(&style, gray));
            assert!((sampled - exact).abs() < 0.02);
        }
        assert!(RenderStyle::default().level_lut().bypass);
    }

    #[test]
    fn equal_styles_share_tables_but_gamma_changes_do_not() {
        let first = nonlinear_style().level_lut();
        let second = nonlinear_style().level_lut();
        assert!(Arc::ptr_eq(
            first.tables.as_ref().unwrap(),
            second.tables.as_ref().unwrap()
        ));

        let mut changed_gamma = nonlinear_style();
        changed_gamma.gamma += 0.1;
        let changed = changed_gamma.level_lut();
        assert!(!Arc::ptr_eq(
            first.tables.as_ref().unwrap(),
            changed.tables.as_ref().unwrap()
        ));
    }

    #[test]
    fn level_for_is_monotonic_safe_and_identity_for_default() {
        let default_style = RenderStyle::default();
        for gray in [0.0, 0.1, 0.55, 1.0, 1.7] {
            assert!((default_style.level_for(gray) / 13.0 - gray.clamp(0.0, 1.0)).abs() < 1e-6);
        }
        let nonlinear = nonlinear_style();
        let levels: Vec<_> = (0..=1000)
            .map(|i| nonlinear.level_for(i as f32 / 500.0))
            .collect();
        assert!(levels.windows(2).all(|pair| pair[0] <= pair[1]));
        assert_eq!(nonlinear.level_for(f32::NAN), 0.0);
        assert_eq!(nonlinear.level_for(f32::INFINITY), 0.0);
    }
}
