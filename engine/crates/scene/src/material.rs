use crate::texture::{PbrTextureSet, SpriteAtlas, Texture, TextureId};

/// Number of distinct block ids representable by the core voxel format.
pub const MATERIAL_ID_SPACE: usize = u8::MAX as usize + 1;
use voxelworks_core::math::hash4;

/// Stable per-voxel UV transform and tint shared by both render backends.
pub fn cell_uv(cell: (i32, i32, i32), uv: (f32, f32)) -> (f32, f32, f32, bool) {
    let choice = ((hash4(cell.0, cell.1, cell.2, 97) + 1.0) * 8.0) as u32;
    let (mut u, mut v) = (uv.0, uv.1);
    if choice & 4 != 0 {
        u = 1.0 - u;
    }
    if choice & 8 != 0 {
        v = 1.0 - v;
    }
    for _ in 0..(choice & 3) {
        (u, v) = (v, 1.0 - u);
    }
    let tint = 0.92 + hash4(cell.0, cell.1, cell.2, 103) * 0.08;
    (u, v, tint, choice & 3 == 0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BaseAlbedoRule {
    Constant,
    Procedural(u32),
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
    /// Enables deterministic cell-level rotation/flip/tint in PBR shading.
    pub cell_variation: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct MaterialTable {
    pub materials: &'static [Material],
    pub pbr_textures: &'static [PbrTextureSet],
    pub sprite_atlas: Option<&'static SpriteAtlas>,
}

impl MaterialTable {
    pub const fn new(materials: &'static [Material]) -> Self {
        Self {
            materials,
            pbr_textures: &[],
            sprite_atlas: None,
        }
    }

    pub const fn with_pbr_textures(
        materials: &'static [Material],
        pbr_textures: &'static [PbrTextureSet],
    ) -> Self {
        Self {
            materials,
            pbr_textures,
            sprite_atlas: None,
        }
    }

    pub const fn with_pbr_and_sprite(
        materials: &'static [Material],
        pbr_textures: &'static [PbrTextureSet],
        sprite_atlas: Option<&'static SpriteAtlas>,
    ) -> Self {
        Self {
            materials,
            pbr_textures,
            sprite_atlas,
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

#[cfg(test)]
mod cell_variation_tests {
    use super::cell_uv;

    #[test]
    fn cell_transform_and_tint_are_repeatable_and_cell_specific() {
        let first = cell_uv((12, 3, 8), (0.2, 0.7));
        assert_eq!(first, cell_uv((12, 3, 8), (0.2, 0.7)));
        let neighbor = cell_uv((13, 3, 8), (0.2, 0.7));
        assert_ne!(first, neighbor);
        assert!((0.84..=1.0).contains(&first.2));
        assert!(first.0.is_finite() && first.1.is_finite());
    }
}

#[derive(Clone, Debug)]
pub struct TextureAtlas {
    pub bricks: Texture,
    pub noise: Texture,
}
