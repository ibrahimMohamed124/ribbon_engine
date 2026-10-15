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
pub use blocks_engine::gfx::Framebuffer;
pub use blocks_engine::render::entities::{ray_box, EntView};
pub use blocks_engine::render::raycaster::{render, render_with_materials};
pub use blocks_engine::render::style::{Palette, RenderStyle, Tonemap, LEVEL_COUNT};
pub use blocks_engine::render::RENDER_DIST;
pub use engine_scene::scene::Scene;

pub const DEFAULT_WIDTH: usize = 320;
pub const DEFAULT_HEIGHT: usize = 180;
pub const DEFAULT_HFOV: f32 = 1.35;
