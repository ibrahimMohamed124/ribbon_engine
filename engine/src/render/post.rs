//! Screen-space vignette, grain, and ordered dithering.

use crate::math::{hash4, pack_gray};

pub const LEVELS: f32 = 14.0;
pub const BAYER: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];

pub fn finish_pixel(g: f32, u: f32, v: f32, x: usize, y: usize, frame: u32, intensity: f32) -> u32 {
    let r2 = u * u + v * v;
    let vig = 1.0 - r2 * (0.45 + 0.5 * intensity);
    let grain = hash4(x as i32, y as i32, frame as i32, 5) * 0.025 * (1.0 + 3.0 * intensity);
    let gv = (g * vig + grain).clamp(0.0, 1.0);
    let b = (BAYER[y & 3][x & 3] as f32 + 0.5) / 16.0;
    let q = ((gv * (LEVELS - 1.0) + b).floor() / (LEVELS - 1.0)).clamp(0.0, 1.0);
    pack_gray(q)
}
