//! HUD and full-screen overlays. All coordinates are in framebuffer pixels.

use super::Framebuffer;
use crate::math::{hash4, pack_gray};

pub fn fill_rect(fb: &mut Framebuffer, x0: i32, y0: i32, w: i32, h: i32, color: u32) {
    let (fw, fh) = (fb.width as i32, fb.height as i32);
    for y in y0.max(0)..(y0 + h).min(fh) {
        for x in x0.max(0)..(x0 + w).min(fw) {
            fb.pixels[(y * fw + x) as usize] = color;
        }
    }
}

pub fn draw_hud(fb: &mut Framebuffer, collected: usize, total: usize, stamina: f32, exhausted: bool) {
    // page counters
    for i in 0..total {
        let x = 6 + i as i32 * 10;
        if i < collected {
            fill_rect(fb, x, 6, 7, 9, 0x00dc_dcdc);
        } else {
            fill_rect(fb, x, 6, 7, 9, 0x0030_3030);
            fill_rect(fb, x + 1, 7, 5, 7, 0x0010_1010);
        }
    }
    // stamina bar
    let y = fb.height as i32 - 9;
    fill_rect(fb, 6, y, 62, 4, 0x0030_3030);
    let w = (60.0 * stamina.clamp(0.0, 1.0)) as i32;
    fill_rect(fb, 7, y + 1, w, 2, if exhausted { 0x0060_6060 } else { 0x00c0_c0c0 });
    // tiny crosshair dot
    let (cx, cy) = (fb.width as i32 / 2, fb.height as i32 / 2);
    fill_rect(fb, cx - 1, cy - 1, 2, 2, 0x0090_9090);
}

pub fn draw_caught(fb: &mut Framebuffer, t: f32, frame: u32) {
    let (w, h) = (fb.width, fb.height);
    for y in 0..h {
        for x in 0..w {
            let n = (hash4(x as i32, y as i32, frame as i32, 9) + 1.0) * 0.5;
            fb.pixels[y * w + x] = pack_gray(0.04 + 0.22 * n);
        }
    }
    if t < 0.12 {
        for p in fb.pixels.iter_mut() {
            *p = 0x00ff_ffff;
        }
        return;
    }
    let blink = hash4((frame / 2) as i32, 1, 2, 3) > 0.85;
    if !blink {
        let hw = (14.0 + t * 40.0).min(64.0) as i32;
        let hh = (8.0 + t * 22.0).min(34.0) as i32;
        let jx = (hash4(frame as i32, 4, 4, 4) * 3.0) as i32;
        let cx = w as i32 / 2;
        let cy = h as i32 / 2 - 22;
        fill_rect(fb, cx - 62 - hw / 2 + jx, cy - hh / 2, hw, hh, 0x00ff_ffff);
        fill_rect(fb, cx + 62 - hw / 2 + jx, cy - hh / 2, hw, hh, 0x00ff_ffff);
        fill_rect(fb, cx - 58 + jx, cy + 62, 116, 4 + (t * 3.0) as i32, 0x00ff_ffff);
    }
}

pub fn draw_won(fb: &mut Framebuffer, t: f32, frame: u32) {
    let (w, h) = (fb.width, fb.height);
    let base = (t / 3.0).min(1.0) * 0.9;
    for y in 0..h {
        for x in 0..w {
            let n = hash4(x as i32, y as i32, frame as i32, 11) * 0.04;
            fb.pixels[y * w + x] = pack_gray(base + n);
        }
    }
}
