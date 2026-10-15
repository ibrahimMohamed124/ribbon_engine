//! Temporary compatibility facade for the split engine crates.

pub use engine_core::{app, audio, input, math, physics, time, voxel};
pub mod gfx {
    pub use engine_core::gfx::{fill_rect, font, post, Framebuffer};
    pub use engine_scene::texture;
}
pub use engine_scene;
pub use render_cpu::render;
