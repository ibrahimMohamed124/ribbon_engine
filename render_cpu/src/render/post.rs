//! Screen-space vignette, grain, and ordered dithering.

use crate::render::style::{LevelLut, RenderStyle, LEVEL_COUNT};
use engine_core::math::hash4;

pub const LEVELS: f32 = LEVEL_COUNT as f32;
pub const BAYER: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];

#[derive(Clone, Copy, Debug)]
pub struct PixelSample {
    pub u: f32,
    pub v: f32,
    pub x: usize,
    pub y: usize,
    pub frame: u32,
    pub intensity: f32,
}

pub fn finish_pixel(g: f32, u: f32, v: f32, x: usize, y: usize, frame: u32, intensity: f32) -> u32 {
    finish_pixel_styled(
        g,
        PixelSample {
            u,
            v,
            x,
            y,
            frame,
            intensity,
        },
        &RenderStyle::classic(),
    )
}

pub fn finish_pixel_styled(g: f32, sample: PixelSample, style: &RenderStyle) -> u32 {
    let lut = style.level_lut();
    finish_pixel_styled_with_lut(g, sample, style, &lut)
}

pub(crate) fn finish_pixel_styled_with_lut(
    g: f32,
    sample: PixelSample,
    style: &RenderStyle,
    lut: &LevelLut,
) -> u32 {
    if !g.is_finite()
        || !sample.u.is_finite()
        || !sample.v.is_finite()
        || !sample.intensity.is_finite()
    {
        return style.palette.color(0);
    }

    let mapped = lut.tone(style, g);
    let r2 = sample.u * sample.u + sample.v * sample.v;
    let vig = 1.0 - r2 * (0.45 + 0.5 * sample.intensity);
    let grain = hash4(sample.x as i32, sample.y as i32, sample.frame as i32, 5)
        * 0.025
        * (1.0 + 3.0 * sample.intensity);
    let gv = (mapped * vig + grain).clamp(0.0, 1.0);
    let b = (BAYER[sample.y & 3][sample.x & 3] as f32 + 0.5) / 16.0;
    let level = (lut.level(gv) + b).floor().clamp(0.0, LEVELS - 1.0) as usize;
    style.palette.color(level)
}

#[cfg(test)]
mod tests {
    use super::{finish_pixel, finish_pixel_styled};
    use crate::render::style::RenderStyle;

    #[test]
    fn classic_wrapper_matches_styled_classic() {
        let style = RenderStyle::classic();
        for gray in [0.0, 0.01, 0.1, 0.5, 0.99, 1.0, 1.4] {
            for (x, y) in [(0, 0), (7, 3), (31, 15)] {
                assert_eq!(
                    finish_pixel(gray, -0.3, 0.6, x, y, 12, 0.75),
                    finish_pixel_styled(
                        gray,
                        super::PixelSample {
                            u: -0.3,
                            v: 0.6,
                            x,
                            y,
                            frame: 12,
                            intensity: 0.75,
                        },
                        &style
                    )
                );
            }
        }
    }

    #[test]
    fn gamma_one_is_a_noop_and_non_finite_input_is_black() {
        let mut style = RenderStyle::classic();
        style.gamma = 1.0;
        for gray in [0.0, 0.25, 0.75, 1.2] {
            assert_eq!(
                finish_pixel_styled(
                    gray,
                    super::PixelSample {
                        u: 0.2,
                        v: -0.1,
                        x: 3,
                        y: 7,
                        frame: 42,
                        intensity: 0.3,
                    },
                    &style,
                ),
                finish_pixel(gray, 0.2, -0.1, 3, 7, 42, 0.3)
            );
        }
        for gray in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(
                finish_pixel_styled(
                    gray,
                    super::PixelSample {
                        u: 0.0,
                        v: 0.0,
                        x: 0,
                        y: 0,
                        frame: 0,
                        intensity: 0.0,
                    },
                    &style,
                ),
                style.palette.color(0)
            );
        }
    }
}
