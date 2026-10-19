//! Compatibility shim: the GPU voxel renderer now lives in
//! `plugin_rendering::gpu::voxel`. Removed together with the legacy tree
//! (see `docs/PHASE9.md`).

pub use plugin_rendering::gpu::voxel::{GpuPresenter, GpuRenderer};
