use crate::texture::{PbrTextureSet, Texture, TextureId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BaseAlbedoRule {
    Constant,
    StudsClassic,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextureKind {
    None,
    Bricks { seed: i32 },
    Noise { seed: i32 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Material {
    pub block_id: u8,
    pub base_rule: BaseAlbedoRule,
    pub constant_albedo: f32,
    pub texture: TextureKind,
    /// Optional stable PBR texture-set reference; `None` keeps legacy shading.
    pub pbr_texture: Option<TextureId>,
    /// Per-face repeat scale applied before sampling the referenced set.
    pub uv_scale: [f32; 2],
    /// Enables deterministic cell-level rotation/flip/tint in realistic rendering.
    pub cell_variation: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct MaterialTable {
    pub materials: &'static [Material],
    pub pbr_textures: &'static [PbrTextureSet],
}

impl MaterialTable {
    pub const fn new(materials: &'static [Material]) -> Self {
        Self {
            materials,
            pbr_textures: &[],
        }
    }

    pub const fn with_pbr_textures(
        materials: &'static [Material],
        pbr_textures: &'static [PbrTextureSet],
    ) -> Self {
        Self {
            materials,
            pbr_textures,
        }
    }

    pub fn get(&self, block_id: u8) -> Option<&Material> {
        self.materials
            .iter()
            .find(|material| material.block_id == block_id)
    }

    pub fn get_pbr_texture(&self, id: TextureId) -> Option<&PbrTextureSet> {
        self.pbr_textures.iter().find(|set| set.id == id)
    }
}

#[derive(Clone, Debug)]
pub struct TextureAtlas {
    pub bricks: Texture,
    pub noise: Texture,
}
