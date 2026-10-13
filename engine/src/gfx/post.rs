//! Shared image post-processing helpers.

use super::Framebuffer;

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
