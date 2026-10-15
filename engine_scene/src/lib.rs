//! Renderer-independent descriptions of worlds, cameras, lighting, and entities.

pub mod entities;
pub mod scene;
pub mod style;
pub mod texture;

pub use scene::{Camera, LightingParams, Scene};
pub use style::{Palette, RenderStyle, Tonemap, LEVEL_COUNT};

pub const RENDER_DIST: f32 = 18.0;
