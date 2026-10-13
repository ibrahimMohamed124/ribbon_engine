use crate::math::hash4;
pub use blocks_engine::gfx::font::text;
use blocks_engine::gfx::{fill_rect, Framebuffer};

pub const INK: u32 = 0x0008_0a0d;
pub const PANEL: u32 = 0x0010_1418;
pub const PANEL_EDGE: u32 = 0x0037_4148;
pub const TEXT: u32 = 0x00d9_dddf;
pub const MUTED: u32 = 0x0078_8289;
pub const ACCENT: u32 = 0x00c2_4545;
pub const SELECTED: u32 = 0x001b_252c;

pub fn fog_background(fb: &mut Framebuffer, frame: u32) {
    let (w, h) = (fb.width, fb.height);
    if w == 0 || h == 0 {
        return;
    }
    for y in 0..h {
        let vertical = y as f32 / h as f32;
        for x in 0..w {
            let nx = (x as f32 / w as f32 - 0.5) * 1.25;
            let ny = (vertical - 0.46) * 1.4;
            let vignette = (1.0 - (nx * nx + ny * ny).sqrt()).clamp(0.0, 1.0);
            let grain = hash4(x as i32 / 3, y as i32 / 3, frame as i32 / 8, 71) * 0.025;
            let glow = (1.0 - ((x as f32 / w as f32 - 0.72).abs() * 4.0)).max(0.0)
                * (1.0 - vertical).max(0.0)
                * 0.035;
            let gray = (0.018 + vignette * 0.045 + grain + glow).clamp(0.0, 0.13);
            let red = (gray + glow * 0.52).clamp(0.0, 0.14);
            fb.pixels[y * w + x] =
                ((red * 255.0) as u32) << 16 | ((gray * 255.0) as u32) << 8 | (gray * 255.0) as u32;
        }
    }

    // A dim, unstable light source gives the flat software-rendered scene depth.
    let pulse = (hash4(frame as i32 / 7, 0, 0, 4) * 4.0) as i32;
    fill_rect(
        fb,
        (w as i32 * 3) / 4 - 18,
        h as i32 / 5,
        36,
        2 + pulse,
        0x0039_2929,
    );
    fill_rect(fb, 0, h as i32 - 12, w as i32, 1, 0x0014_181b);
}

pub fn dim(fb: &mut Framebuffer, amount: f32) {
    let factor = (1.0 - amount.clamp(0.0, 1.0)).clamp(0.0, 1.0);
    for pixel in &mut fb.pixels {
        let r = ((*pixel >> 16) & 0xff) as f32 * factor;
        let g = ((*pixel >> 8) & 0xff) as f32 * factor;
        let b = (*pixel & 0xff) as f32 * factor;
        *pixel = ((r as u32) << 16) | ((g as u32) << 8) | b as u32;
    }
}

pub fn panel(fb: &mut Framebuffer, x: i32, y: i32, w: i32, h: i32) {
    fill_rect(fb, x, y, w, h, PANEL);
    fill_rect(fb, x, y, w, 1, PANEL_EDGE);
    fill_rect(fb, x, y + h - 1, w, 1, PANEL_EDGE);
    fill_rect(fb, x, y, 1, h, PANEL_EDGE);
    fill_rect(fb, x + w - 1, y, 1, h, PANEL_EDGE);
    fill_rect(fb, x + 10, y + 10, 2, 14, ACCENT);
}

pub fn centered_text(fb: &mut Framebuffer, value: &str, y: i32, scale: i32, color: u32) {
    let width = value.chars().count() as i32 * 6 * scale - scale;
    text(fb, value, (fb.width as i32 - width) / 2, y, scale, color);
}

pub fn label_value(fb: &mut Framebuffer, label: &str, value: &str, x: i32, y: i32, width: i32) {
    text(fb, label, x, y, 1, TEXT);
    let value_width = value.chars().count() as i32 * 6 - 1;
    text(fb, value, x + width - value_width, y, 1, MUTED);
}
