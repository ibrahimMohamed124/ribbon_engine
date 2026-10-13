//! Software renderer.
//!
//! * `raycaster` - camera, per-pixel pipeline, parallel frame loop
//! * `lighting`  - flashlight / fog / procedural block textures
//! * `entities`  - box-based models (enemy, pages) and their ray tests
//! * `post`      - vignette, film grain, ordered dithering
//! * `hud`       - HUD and full-screen game-over / win screens

pub mod entities;
pub mod hud;
pub mod lighting;
pub mod post;
pub mod raycaster;

pub use entities::{EntKind, EntView};
pub use raycaster::{render, Frame};

/// How far rays travel before the world fades to black.
pub const RENDER_DIST: f32 = 18.0;

pub const DEFAULT_WIDTH: usize = 320;
pub const DEFAULT_HEIGHT: usize = 180;
pub const DEFAULT_HFOV: f32 = 1.35;

/// A CPU-side 0x00RRGGBB pixel buffer.
pub struct Framebuffer {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u32>,
}

impl Framebuffer {
    pub fn new(width: usize, height: usize) -> Self {
        Framebuffer {
            width,
            height,
            pixels: vec![0u32; width * height],
        }
    }
}
