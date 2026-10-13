//! Per-pixel post-processing: vignette, film grain, ordered dithering.

use super::Framebuffer;
use crate::math::{hash4, pack_gray};

/// Number of gray shades (dithered).
pub const LEVELS: f32 = 14.0;

pub const BAYER: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];

/// Apply a linear brightness multiplier to an already rendered frame.
pub fn adjust_brightness(fb: &mut Framebuffer, brightness: f32) {
    let brightness = if brightness.is_finite() {
        brightness.clamp(0.5, 1.5)
    } else {
        1.0
    };
    for pixel in &mut fb.pixels {
        let r = (((*pixel >> 16) & 0xff) as f32 * brightness).clamp(0.0, 255.0) as u32;
        let g = (((*pixel >> 8) & 0xff) as f32 * brightness).clamp(0.0, 255.0) as u32;
        let b = ((*pixel & 0xff) as f32 * brightness).clamp(0.0, 255.0) as u32;
        *pixel = (r << 16) | (g << 8) | b;
    }
}

/// Turn a linear gray `g` into a final packed pixel.
/// `u`, `v` are the pixel's screen coordinates in [-1, 1]; `prox` (0..1) is how
/// close the enemy is, which darkens the edges and makes the grain heavier.
pub fn finish_pixel(g: f32, u: f32, v: f32, x: usize, y: usize, frame: u32, prox: f32) -> u32 {
    let r2 = u * u + v * v;
    let vig = 1.0 - r2 * (0.45 + 0.5 * prox);
    let grain = hash4(x as i32, y as i32, frame as i32, 5) * 0.025 * (1.0 + 3.0 * prox);
    let gv = (g * vig + grain).clamp(0.0, 1.0);
    let b = (BAYER[y & 3][x & 3] as f32 + 0.5) / 16.0;
    let q = ((gv * (LEVELS - 1.0) + b).floor() / (LEVELS - 1.0)).clamp(0.0, 1.0);
    pack_gray(q)
}
