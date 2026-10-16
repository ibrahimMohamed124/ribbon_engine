//! CPU-side drawing primitives shared by games and tools.

pub mod font;
pub mod post;

/// Full-screen operations supported by both immediate and deferred canvases.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Effect {
    FogBackground { frame: u32 },
    CaughtNoise { time: f32, frame: u32 },
    WonNoise { time: f32, frame: u32 },
    SolidWhite,
}

/// Small drawing interface used by game UI and renderer command recording.
pub trait Canvas {
    fn width(&self) -> usize;
    fn height(&self) -> usize;
    fn rect(&mut self, x: i32, y: i32, width: i32, height: i32, color: u32);
    fn text(&mut self, value: &str, x: i32, y: i32, scale: i32, color: u32);
    fn dim(&mut self, amount: f32);
    fn effect(&mut self, effect: Effect);
}

/// Ordered immediate-mode drawing commands for GPU or CPU backends.
#[derive(Clone, Debug, PartialEq)]
pub enum DrawCommand {
    Rect {
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        color: u32,
    },
    Text {
        value: String,
        x: i32,
        y: i32,
        scale: i32,
        color: u32,
    },
    Dim(f32),
    Effect(Effect),
}

#[derive(Clone, Debug, PartialEq)]
pub struct DrawList {
    pub width: usize,
    pub height: usize,
    pub commands: Vec<DrawCommand>,
}

impl DrawList {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            commands: Vec::new(),
        }
    }
}

impl Canvas for DrawList {
    fn width(&self) -> usize {
        self.width
    }
    fn height(&self) -> usize {
        self.height
    }
    fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, color: u32) {
        self.commands.push(DrawCommand::Rect { x, y, w, h, color });
    }
    fn text(&mut self, value: &str, x: i32, y: i32, scale: i32, color: u32) {
        self.commands.push(DrawCommand::Text {
            value: value.to_owned(),
            x,
            y,
            scale,
            color,
        });
    }
    fn dim(&mut self, amount: f32) {
        self.commands.push(DrawCommand::Dim(amount));
    }
    fn effect(&mut self, effect: Effect) {
        self.commands.push(DrawCommand::Effect(effect));
    }
}

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

impl Canvas for Framebuffer {
    fn width(&self) -> usize {
        self.width
    }
    fn height(&self) -> usize {
        self.height
    }
    fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, color: u32) {
        let (fw, fh) = (self.width as i32, self.height as i32);
        for py in y.max(0)..(y + h).min(fh) {
            for px in x.max(0)..(x + w).min(fw) {
                self.pixels[(py * fw + px) as usize] = color;
            }
        }
    }
    fn text(&mut self, value: &str, x: i32, y: i32, scale: i32, color: u32) {
        font::text(self, value, x, y, scale, color);
    }
    fn dim(&mut self, amount: f32) {
        let factor = (1.0 - amount.clamp(0.0, 1.0)).clamp(0.0, 1.0);
        for pixel in &mut self.pixels {
            let r = (((*pixel >> 16) & 0xff) as f32 * factor) as u32;
            let g = (((*pixel >> 8) & 0xff) as f32 * factor) as u32;
            let b = ((*pixel & 0xff) as f32 * factor) as u32;
            *pixel = (r << 16) | (g << 8) | b;
        }
    }
    fn effect(&mut self, effect: Effect) {
        match effect {
            Effect::FogBackground { frame } => fog_background(self, frame),
            Effect::CaughtNoise { time, frame } => caught_noise(self, time, frame),
            Effect::WonNoise { time, frame } => won_noise(self, time, frame),
            Effect::SolidWhite => self.pixels.fill(0x00ff_ffff),
        }
    }
}

fn hash4(x: i32, y: i32, z: i32, w: i32) -> f32 {
    let mut h = (x as u32).wrapping_mul(73_856_093)
        ^ (y as u32).wrapping_mul(19_349_663)
        ^ (z as u32).wrapping_mul(83_492_791)
        ^ (w as u32).wrapping_mul(2_654_435_761);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h & 0xffff) as f32 / 32767.5 - 1.0
}

fn pack_gray(value: f32) -> u32 {
    let v = (value.clamp(0.0, 1.0) * 255.0) as u32;
    (v << 16) | (v << 8) | v
}

fn fog_background(fb: &mut Framebuffer, frame: u32) {
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

fn caught_noise(fb: &mut Framebuffer, t: f32, frame: u32) {
    let (w, h) = (fb.width, fb.height);
    for y in 0..h {
        for x in 0..w {
            let n = (hash4(x as i32, y as i32, frame as i32, 9) + 1.0) * 0.5;
            fb.pixels[y * w + x] = pack_gray(0.04 + 0.22 * n);
        }
    }
    if t < 0.12 {
        fb.pixels.fill(0x00ff_ffff);
        return;
    }
    if hash4((frame / 2) as i32, 1, 2, 3) > 0.85 {
        return;
    }
    let hw = (14.0 + t * 40.0).min(64.0) as i32;
    let hh = (8.0 + t * 22.0).min(34.0) as i32;
    let jx = (hash4(frame as i32, 4, 4, 4) * 3.0) as i32;
    let (cx, cy) = (w as i32 / 2, h as i32 / 2 - 22);
    fill_rect(fb, cx - 62 - hw / 2 + jx, cy - hh / 2, hw, hh, 0x00ff_ffff);
    fill_rect(fb, cx + 62 - hw / 2 + jx, cy - hh / 2, hw, hh, 0x00ff_ffff);
    fill_rect(
        fb,
        cx - 58 + jx,
        cy + 62,
        116,
        4 + (t * 3.0) as i32,
        0x00ff_ffff,
    );
}

fn won_noise(fb: &mut Framebuffer, t: f32, frame: u32) {
    let base = (t / 3.0).min(1.0) * 0.9;
    let (w, h) = (fb.width, fb.height);
    for y in 0..h {
        for x in 0..w {
            let n = hash4(x as i32, y as i32, frame as i32, 11) * 0.04;
            fb.pixels[y * w + x] = pack_gray(base + n);
        }
    }
}

pub fn fill_rect<C: Canvas + ?Sized>(fb: &mut C, x0: i32, y0: i32, w: i32, h: i32, color: u32) {
    fb.rect(x0, y0, w, h, color);
}

#[cfg(test)]
mod canvas_tests {
    use super::{Canvas, DrawCommand, DrawList, Effect, Framebuffer};

    #[test]
    fn draw_list_keeps_order_and_text_as_a_single_command() {
        let mut list = DrawList::new(20, 10);
        list.rect(1, 2, 3, 4, 0x123456);
        list.text("A", 5, 6, 1, 0xffffff);
        list.dim(0.25);
        assert!(matches!(list.commands[0], DrawCommand::Rect { .. }));
        assert!(matches!(list.commands[1], DrawCommand::Text { .. }));
        assert_eq!(list.commands[2], DrawCommand::Dim(0.25));
    }

    #[test]
    fn immediate_rect_keeps_clipping_and_effects_are_repeatable() {
        let mut first = Framebuffer::new(3, 2);
        first.rect(-1, 0, 3, 1, 0x123456);
        first.effect(Effect::CaughtNoise {
            time: 0.5,
            frame: 13,
        });
        let mut second = Framebuffer::new(3, 2);
        second.rect(-1, 0, 3, 1, 0x123456);
        second.effect(Effect::CaughtNoise {
            time: 0.5,
            frame: 13,
        });
        assert_eq!(first.pixels, second.pixels);
        for point in [(-1, 0, 0, 0), (0, 1, 13, 71), (1280, -720, 8, 6)] {
            assert_eq!(
                super::hash4(point.0, point.1, point.2, point.3),
                crate::math::hash4(point.0, point.1, point.2, point.3)
            );
        }
    }
}
