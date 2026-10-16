use std::time::Instant;

use voxelworks_api::Framebuffer;
use voxelworks_platform::present::blit_scaled;

fn main() {
    const WIDTH: u32 = 1280;
    const HEIGHT: u32 = 720;
    const FRAMES: usize = 100;
    let mut fb = Framebuffer::new(320, 180);
    for (index, pixel) in fb.pixels.iter_mut().enumerate() {
        *pixel = index as u32 & 0x00ff_ffff;
    }
    let mut dst = vec![0; (WIDTH * HEIGHT) as usize];
    for _ in 0..10 {
        blit_scaled(&fb, &mut dst, WIDTH, HEIGHT);
    }
    let start = Instant::now();
    for _ in 0..FRAMES {
        blit_scaled(&fb, &mut dst, WIDTH, HEIGHT);
    }
    println!(
        "blit_scaled 320x180 -> {WIDTH}x{HEIGHT}: {:.3} ms/frame ({FRAMES} frames)",
        start.elapsed().as_secs_f64() * 1000.0 / FRAMES as f64
    );
}
