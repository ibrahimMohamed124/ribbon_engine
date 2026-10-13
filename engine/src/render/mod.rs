//! Shared voxel rendering algorithms and geometry.

pub mod entities;
pub mod lighting;
pub mod post;
pub mod raycaster;

pub use entities::{EntView, EntitySurface};
pub use raycaster::{render, Frame};

pub const RENDER_DIST: f32 = 18.0;
