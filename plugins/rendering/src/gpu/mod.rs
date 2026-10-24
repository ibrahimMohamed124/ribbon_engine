//! GPU backends. The context and offscreen target come from `ribbon_render`; the
//! voxel raymarching renderer lives in [`voxel`].

pub mod voxel;

pub use ribbon_render::gpu::{GpuContext, OffscreenTarget};
pub use ribbon_render::graph::RenderError;
pub use voxel::{GpuPresenter, GpuRenderer};
