use crate::math::hash4;
use crate::renderer::hud::fill_rect;
use crate::renderer::Framebuffer;

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

pub fn text(fb: &mut Framebuffer, value: &str, mut x: i32, y: i32, scale: i32, color: u32) {
    let scale = scale.max(1);
    for ch in value.chars() {
        let glyph = glyph(ch.to_ascii_uppercase());
        for (row, bits) in glyph.iter().enumerate() {
            for col in 0..5 {
                if bits & (1 << (4 - col)) != 0 {
                    fill_rect(
                        fb,
                        x + col * scale,
                        y + row as i32 * scale,
                        scale,
                        scale,
                        color,
                    );
                }
            }
        }
        x += 6 * scale;
    }
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

fn glyph(ch: char) -> [u8; 7] {
    match ch {
        'A' => [14, 17, 17, 31, 17, 17, 17],
        'B' => [30, 17, 17, 30, 17, 17, 30],
        'C' => [14, 17, 16, 16, 16, 17, 14],
        'D' => [30, 17, 17, 17, 17, 17, 30],
        'E' => [31, 16, 16, 30, 16, 16, 31],
        'F' => [31, 16, 16, 30, 16, 16, 16],
        'G' => [14, 17, 16, 23, 17, 17, 15],
        'H' => [17, 17, 17, 31, 17, 17, 17],
        'I' => [14, 4, 4, 4, 4, 4, 14],
        'J' => [7, 2, 2, 2, 2, 18, 12],
        'K' => [17, 18, 20, 24, 20, 18, 17],
        'L' => [16, 16, 16, 16, 16, 16, 31],
        'M' => [17, 27, 21, 21, 17, 17, 17],
        'N' => [17, 25, 21, 19, 17, 17, 17],
        'O' => [14, 17, 17, 17, 17, 17, 14],
        'P' => [30, 17, 17, 30, 16, 16, 16],
        'Q' => [14, 17, 17, 17, 21, 18, 13],
        'R' => [30, 17, 17, 30, 20, 18, 17],
        'S' => [15, 16, 16, 14, 1, 1, 30],
        'T' => [31, 4, 4, 4, 4, 4, 4],
        'U' => [17, 17, 17, 17, 17, 17, 14],
        'V' => [17, 17, 17, 17, 17, 10, 4],
        'W' => [17, 17, 17, 21, 21, 21, 10],
        'X' => [17, 17, 10, 4, 10, 17, 17],
        'Y' => [17, 17, 10, 4, 4, 4, 4],
        'Z' => [31, 1, 2, 4, 8, 16, 31],
        '0' => [14, 17, 19, 21, 25, 17, 14],
        '1' => [4, 12, 4, 4, 4, 4, 14],
        '2' => [14, 17, 1, 2, 4, 8, 31],
        '3' => [30, 1, 1, 14, 1, 1, 30],
        '4' => [2, 6, 10, 18, 31, 2, 2],
        '5' => [31, 16, 16, 30, 1, 1, 30],
        '6' => [14, 16, 16, 30, 17, 17, 14],
        '7' => [31, 1, 2, 4, 8, 8, 8],
        '8' => [14, 17, 17, 14, 17, 17, 14],
        '9' => [14, 17, 17, 15, 1, 1, 14],
        ':' => [0, 4, 4, 0, 4, 4, 0],
        '.' => [0, 0, 0, 0, 0, 6, 6],
        ',' => [0, 0, 0, 0, 6, 4, 8],
        '%' => [17, 2, 4, 8, 16, 17, 0],
        '-' => [0, 0, 0, 31, 0, 0, 0],
        '/' => [1, 2, 2, 4, 8, 8, 16],
        '+' => [0, 4, 4, 31, 4, 4, 0],
        '>' => [16, 8, 4, 2, 4, 8, 16],
        '<' => [1, 2, 4, 8, 4, 2, 1],
        ' ' => [0; 7],
        _ => [31, 17, 5, 4, 4, 0, 4],
    }
}
