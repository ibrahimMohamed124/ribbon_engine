//! CPU-side drawing primitives shared by games and tools.

pub mod font;
pub mod post;
pub mod texture;

pub struct Framebuffer {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u32>,
}

impl Framebuffer {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            pixels: vec![0; width * height],
        }
    }
}

pub fn fill_rect(fb: &mut Framebuffer, x0: i32, y0: i32, w: i32, h: i32, color: u32) {
    let (fw, fh) = (fb.width as i32, fb.height as i32);
    for y in y0.max(0)..(y0 + h).min(fh) {
        for x in x0.max(0)..(x0 + w).min(fw) {
            fb.pixels[(y * fw + x) as usize] = color;
        }
    }
}
