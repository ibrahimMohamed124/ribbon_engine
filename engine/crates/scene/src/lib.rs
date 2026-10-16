//! Renderer-independent descriptions of worlds, cameras, lighting, and entities.

pub mod app;
pub mod entities;
pub mod material;
pub mod renderer;
pub mod scene;
pub mod style;
pub mod texture;

pub use material::{BaseAlbedoRule, Material, MaterialTable, TextureAtlas, TextureKind};
pub use scene::{Camera, LightingParams, Scene};
pub use style::{Palette, RenderStyle, Tonemap, LEVEL_COUNT};
pub use texture::{PbrTextureSet, RgbaMipChain, RgbaMipLevel, TextureColorSpace, TextureId};

pub const RENDER_DIST: f32 = 18.0;

pub static EMPTY_MATERIAL_TABLE: MaterialTable = MaterialTable::new(&[]);
