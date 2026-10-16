//! Renderer-independent descriptions of worlds, cameras, lighting, and entities.

pub mod entities;
pub mod material;
pub mod scene;
pub mod style;
pub mod texture;

pub use material::{
    BaseAlbedoRule, Material, MaterialTable, TextureAtlas, TextureKind, MATERIAL_ID_SPACE,
};
pub use scene::{Camera, LightingParams, Scene};
pub use style::{Palette, RenderStyle, Tonemap, LEVEL_COUNT};
pub use texture::{
    PbrTextureSet, RgbaMipChain, RgbaMipLevel, TextureColorSpace, TextureId, MAX_PBR_MAP_DIMENSION,
    MAX_PBR_TEXTURE_SETS, PBR_MAPS_PER_SET, PBR_MIP_LEVEL_COUNT, PBR_MIP_TEXEL_COUNT,
};

pub const RENDER_DIST: f32 = 18.0;

pub static EMPTY_MATERIAL_TABLE: MaterialTable = MaterialTable::new(&[]);
