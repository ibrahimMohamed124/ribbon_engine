//! Backend-neutral drawing data shared by games and renderers.

/// Full-screen operations recorded by the application and interpreted by renderers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Effect {
    FogBackground { frame: u32 },
    CaughtNoise { time: f32, frame: u32 },
    WonNoise { time: f32, frame: u32 },
    SolidWhite,
}

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

/// Plain 24-bit RGB pixel storage. Raster operations belong to renderer crates.
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
