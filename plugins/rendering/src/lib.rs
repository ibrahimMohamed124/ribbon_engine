//! Legacy CPU voxel-scene rendering and ECS integration.
//!
//! GPU 3D mesh rendering is owned by `ribbon_plugin_render3d`; this crate retains the CPU
//! voxel raycaster for supported software rendering and reference use.

#[cfg(feature = "cpu")]
pub mod cpu;
#[cfg(feature = "cpu")]
pub mod ecs;
pub mod types;

#[cfg(feature = "cpu")]
pub use cpu::{canvas, render, renderer};
pub use ribbon_plugin_voxel::VoxelWorldRes;
pub use types::{entities, material, scene, style, texture};

pub use types::{
    BaseAlbedoRule, Camera, LightingParams, Material, MaterialTable, Palette, PbrTextureSet,
    RenderScene, RenderStyle, RgbaMipChain, RgbaMipLevel, Scene, TextureAtlas, TextureColorSpace,
    TextureId, TextureKind, Tonemap, EMPTY_MATERIAL_TABLE, LEVEL_COUNT, MATERIAL_ID_SPACE,
    MAX_PBR_MAP_DIMENSION, MAX_PBR_TEXTURE_SETS, PBR_MAPS_PER_SET, PBR_MIP_LEVEL_COUNT,
    PBR_MIP_TEXEL_COUNT, RENDER_DIST,
};
