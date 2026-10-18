//! HUD and full-screen overlays. All coordinates are in framebuffer pixels.

use engine_render::Effect;
pub use voxelworks_render_cpu::fill_rect;
use voxelworks_render_cpu::Canvas;

pub fn draw_hud(
    fb: &mut impl Canvas,
    collected: usize,
    total: usize,
    stamina: f32,
    exhausted: bool,
) {
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
    let y = fb.height() as i32 - 9;
    fill_rect(fb, 6, y, 62, 4, 0x0030_3030);
    let w = (60.0 * stamina.clamp(0.0, 1.0)) as i32;
    fill_rect(
        fb,
        7,
        y + 1,
        w,
        2,
        if exhausted { 0x0060_6060 } else { 0x00c0_c0c0 },
    );
    // tiny crosshair dot
    let (cx, cy) = (fb.width() as i32 / 2, fb.height() as i32 / 2);
    fill_rect(fb, cx - 1, cy - 1, 2, 2, 0x0090_9090);
}

pub fn draw_caught(fb: &mut impl Canvas, t: f32, frame: u32) {
    fb.effect(Effect::CaughtNoise { time: t, frame });
}

pub fn draw_won(fb: &mut impl Canvas, t: f32, frame: u32) {
    fb.effect(Effect::WonNoise { time: t, frame });
}
