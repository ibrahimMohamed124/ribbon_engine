use crate::texture::Texture;

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
}

#[derive(Clone, Copy, Debug)]
pub struct MaterialTable {
    pub materials: &'static [Material],
}

impl MaterialTable {
    pub const fn new(materials: &'static [Material]) -> Self {
        Self { materials }
    }

    pub fn get(&self, block_id: u8) -> Option<&Material> {
        self.materials
            .iter()
            .find(|material| material.block_id == block_id)
    }
}

#[derive(Clone, Debug)]
pub struct TextureAtlas {
    pub bricks: Texture,
    pub noise: Texture,
}
