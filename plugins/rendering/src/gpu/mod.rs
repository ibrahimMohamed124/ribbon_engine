//! GPU backends. The context and offscreen target come from `engine_render`; the
//! voxel raymarching renderer lives in [`voxel`].

pub mod voxel;

pub use engine_render::gpu::{GpuContext, OffscreenTarget};
pub use engine_render::graph::RenderError;
pub use voxel::{GpuPresenter, GpuRenderer};
