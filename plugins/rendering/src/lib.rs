//! Voxel rendering types, CPU/GPU backends, and ECS rendering integration.
//!
//! This crate owns rendering policy and scene extraction; platform and frame contracts are provided by engine crates.

#[cfg(feature = "cpu")]
pub mod cpu;
pub mod types;

#[cfg(feature = "cpu")]
pub use cpu::{canvas, render, renderer};
pub use types::{entities, material, scene, style, texture};

#[cfg(feature = "gpu")]
pub mod gpu {
    //! GPU context and offscreen target supplied by `engine_render`.

    pub use engine_render::gpu::{GpuContext, OffscreenTarget};
    pub use engine_render::graph::RenderError;
}
pub use types::{
    BaseAlbedoRule, Camera, LightingParams, Material, MaterialTable, Palette, PbrTextureSet,
    RenderScene, RenderStyle, RgbaMipChain, RgbaMipLevel, Scene, TextureAtlas, TextureColorSpace,
    TextureId, TextureKind, Tonemap, EMPTY_MATERIAL_TABLE, LEVEL_COUNT, MATERIAL_ID_SPACE,
    MAX_PBR_MAP_DIMENSION, MAX_PBR_TEXTURE_SETS, PBR_MAPS_PER_SET, PBR_MIP_LEVEL_COUNT,
    PBR_MIP_TEXEL_COUNT, RENDER_DIST,
};
