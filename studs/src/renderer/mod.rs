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
pub use engine_core::gfx::Framebuffer;
pub use engine_scene::scene::Scene;
pub use engine_scene::style::{Palette, RenderStyle, Tonemap, LEVEL_COUNT};
pub use engine_scene::RENDER_DIST;
pub use render_cpu::render::entities::{ray_box, EntView};
pub use render_cpu::render::raycaster::{render, render_with_materials};

pub const DEFAULT_WIDTH: usize = 320;
pub const DEFAULT_HEIGHT: usize = 180;
pub const DEFAULT_HFOV: f32 = 1.35;
