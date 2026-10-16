//! Shared voxel rendering algorithms and geometry.

pub mod lighting;
pub mod post;
pub mod raycaster;
#[cfg(test)]
mod style_contract_tests;

pub mod entities;
pub use entities::{EntHit, EntView};
pub use lighting::EntitySurface;
pub use raycaster::{render, render_with_materials, render_with_materials_checked};
pub use voxelworks_scene::scene::{Camera, LightingParams, Scene};
pub use voxelworks_scene::style;
pub use voxelworks_scene::style::{Palette, RenderStyle, Tonemap, LEVEL_COUNT};
pub use voxelworks_scene::RENDER_DIST;
