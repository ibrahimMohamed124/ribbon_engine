//! Shared voxel rendering algorithms and geometry.

pub mod lighting;
pub mod post;
pub mod raycaster;
#[cfg(test)]
mod style_contract_tests;

pub mod entities;
pub use crate::scene::{Camera, LightingParams, Scene};
pub use crate::style;
pub use crate::style::{Palette, RenderStyle, Tonemap, LEVEL_COUNT};
pub use crate::RENDER_DIST;
pub use entities::{EntHit, EntView};
pub use lighting::EntitySurface;
pub use raycaster::{render, render_with_materials, render_with_materials_checked};
