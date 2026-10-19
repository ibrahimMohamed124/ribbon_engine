//! Temporary compatibility re-exports for legacy render-scene imports.
pub use plugin_rendering::{
    entities, material, scene, style, texture, BaseAlbedoRule, Camera, LightingParams, Material,
    MaterialTable, Palette, PbrTextureSet, RenderScene, RenderStyle, RgbaMipChain, RgbaMipLevel,
    Scene, TextureAtlas, TextureColorSpace, TextureId, TextureKind, Tonemap, EMPTY_MATERIAL_TABLE,
    LEVEL_COUNT, MATERIAL_ID_SPACE, MAX_PBR_MAP_DIMENSION, MAX_PBR_TEXTURE_SETS, PBR_MAPS_PER_SET,
    PBR_MIP_LEVEL_COUNT, PBR_MIP_TEXEL_COUNT, RENDER_DIST,
};
