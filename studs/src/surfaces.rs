//! Game-specific voxel materials and renderable shapes.

use std::sync::OnceLock;

pub use crate::entities::ENEMY_BOUNDS;
use crate::entities::{PAGE, WARDEN};
use engine_core::math::{hash4, Vect};
use engine_core::voxel::{face_uv, Hit};
use engine_scene::entities::{EntityInstance, EntityKind};
use engine_scene::texture::{
    PbrTextureSet, RgbaMipChain, RgbaMipLevel, SpriteAtlas, Texture, TextureColorSpace, TextureId,
};
use engine_scene::RenderStyle;
use engine_scene::{BaseAlbedoRule, Material, MaterialTable, TextureKind};
use render_cpu::render::entities::EntitySurface;
use render_cpu::render::lighting::SurfaceShader;

use crate::level::{CEIL, FLOOR, WALL};

pub struct StudsSurface;

const STUDS_MATERIALS: [Material; 3] = [
    Material {
        block_id: FLOOR,
        base_rule: BaseAlbedoRule::StudsClassic,
        constant_albedo: 0.0,
        texture: TextureKind::Noise { seed: 41 },
        pbr_texture: Some(TextureId(3)),
        uv_scale: [1.0, 1.0],
        cell_variation: false,
    },
    Material {
        block_id: WALL,
        base_rule: BaseAlbedoRule::StudsClassic,
        constant_albedo: 0.0,
        texture: TextureKind::Bricks { seed: 17 },
        pbr_texture: Some(TextureId(1)),
        uv_scale: [1.0, 1.0],
        cell_variation: true,
    },
    Material {
        block_id: CEIL,
        base_rule: BaseAlbedoRule::StudsClassic,
        constant_albedo: 0.0,
        texture: TextureKind::Noise { seed: 41 },
        pbr_texture: Some(TextureId(4)),
        uv_scale: [1.0, 1.0],
        cell_variation: false,
    },
];

pub fn studs_material_table() -> &'static MaterialTable {
    static TABLE: OnceLock<MaterialTable> = OnceLock::new();
    TABLE.get_or_init(|| {
        let loaded = [
            (1, "concrete_wall_004"),
            (2, "t_concrete_wall_002"),
            (3, "concrete_floor_worn_001"),
            (4, "ceiling_interior"),
        ]
        .into_iter()
        .filter_map(|(id, name)| load_pbr_set(id, name))
        .collect::<Vec<_>>();
        let textures: &'static [PbrTextureSet] = Box::leak(loaded.into_boxed_slice());
        let sprites: Option<&'static SpriteAtlas> =
            load_sprite_atlas().map(|atlas| Box::leak(Box::new(atlas)) as &SpriteAtlas);
        MaterialTable::with_pbr_and_sprite(&STUDS_MATERIALS, textures, sprites)
    })
}

fn load_sprite_atlas() -> Option<SpriteAtlas> {
    let directory = asset_roots()
        .into_iter()
        .map(|root| root.join("monster"))
        .find(|path| path.join("albedo.png").is_file());
    let Some(directory) = directory else {
        eprintln!("warning: baked monster sprite atlas is missing; using the legacy box monster");
        return None;
    };
    let load = |name: &str| -> Option<image::RgbaImage> {
        image::open(directory.join(name))
            .ok()
            .map(|image| image.to_rgba8())
    };
    let albedo_image = load("albedo.png")?;
    let normal_image = load("normal.png")?;
    let emissive_image = load("emissive.png")?;
    let (width, height) = (
        albedo_image.width() as usize,
        albedo_image.height() as usize,
    );
    if normal_image.dimensions() != albedo_image.dimensions()
        || emissive_image.dimensions() != albedo_image.dimensions()
        || width % 16 != 0
        || height % 40 != 0
    {
        eprintln!(
            "warning: baked monster atlas dimensions are invalid; using the legacy box monster"
        );
        return None;
    }
    let atlas = SpriteAtlas {
        width,
        height,
        cell_width: width / 16,
        cell_height: height / 40,
        views: 16,
        states: 5,
        frames_per_state: 8,
        albedo: albedo_image.pixels().map(|p| p.0).collect(),
        normal: normal_image.pixels().map(|p| p.0).collect(),
        emissive: emissive_image.pixels().map(|p| p.0[0]).collect(),
    };
    atlas.is_valid().then_some(atlas)
}

fn load_pbr_set(id: u32, name: &str) -> Option<PbrTextureSet> {
    const SIZE: usize = 256;
    const BYTES: usize = SIZE * SIZE * 4;
    let directory = asset_directories()
        .into_iter()
        .map(|root| root.join(name))
        .find(|path| path.join("albedo.rgba").is_file());
    let Some(directory) = directory else {
        eprintln!("warning: PBR texture set '{name}' is missing; using the procedural material");
        return None;
    };
    let read_map = |filename: &str| -> Option<Vec<[u8; 4]>> {
        let bytes = std::fs::read(directory.join(filename)).ok()?;
        if bytes.len() != BYTES {
            eprintln!(
                "warning: invalid map size for '{}'; using the procedural material",
                directory.join(filename).display()
            );
            return None;
        }
        Some(
            bytes
                .chunks(4)
                .map(|chunk| [chunk[0], chunk[1], chunk[2], chunk[3]])
                .collect(),
        )
    };
    let chain = |filename: &str, color_space| {
        Some(RgbaMipChain::new(
            RgbaMipLevel::new(SIZE, SIZE, read_map(filename)?),
            color_space,
        ))
    };
    let Some(albedo) = chain("albedo.rgba", TextureColorSpace::Srgb) else {
        eprintln!("warning: failed to load albedo for '{name}'; using the procedural material");
        return None;
    };
    let Some(normal) = chain("normal.rgba", TextureColorSpace::Linear) else {
        eprintln!("warning: failed to load normal for '{name}'; using the procedural material");
        return None;
    };
    let Some(roughness) = chain("roughness.rgba", TextureColorSpace::Linear) else {
        eprintln!("warning: failed to load roughness for '{name}'; using the procedural material");
        return None;
    };
    let Some(ambient_occlusion) = chain("ao.rgba", TextureColorSpace::Linear) else {
        eprintln!("warning: failed to load AO for '{name}'; using the procedural material");
        return None;
    };
    Some(PbrTextureSet {
        id: TextureId(id),
        albedo,
        normal,
        roughness,
        ambient_occlusion,
    })
}

fn asset_directories() -> Vec<std::path::PathBuf> {
    asset_roots()
        .into_iter()
        .map(|root| root.join("materials"))
        .collect()
}

fn asset_roots() -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();
    if let Ok(executable) = std::env::current_exe() {
        if let Some(parent) = executable.parent() {
            paths.push(parent.join("assets"));
        }
    }
    if let Ok(current) = std::env::current_dir() {
        paths.push(current.join("assets"));
        paths.push(current.join("studs/assets"));
    }
    paths
}

impl StudsSurface {
    pub fn materials(&self) -> &'static MaterialTable {
        studs_material_table()
    }
}

impl SurfaceShader for StudsSurface {
    fn albedo(&self, block: u8, hit: &Hit, _direction: Vect, style: &RenderStyle) -> f32 {
        let base = classic_albedo(block, hit);
        let strength = if style.texture_strength.is_finite() {
            style.texture_strength.clamp(0.0, 1.0)
        } else {
            0.0
        };
        if strength == 0.0 {
            return base;
        }

        if block == WALL {
            static BRICKS: OnceLock<Texture> = OnceLock::new();
            let bricks = BRICKS.get_or_init(|| Texture::bricks(64, 32, 17));
            return base * wall_texture_modulation(hit, bricks, strength);
        }

        if block == FLOOR || block == CEIL {
            static NOISE: OnceLock<Texture> = OnceLock::new();
            let noise = NOISE.get_or_init(|| Texture::noise(64, 64, 41));
            let (u, v) = face_uv(hit);
            let texel = noise.sample_faded(u, v, hit.dist, 5.0, 14.0);
            return apply_texture_modulation(base, texel / noise.average(), strength);
        }
        base
    }

    fn albedo_with_materials(
        &self,
        block: u8,
        hit: &Hit,
        direction: Vect,
        style: &RenderStyle,
        materials: &MaterialTable,
    ) -> f32 {
        let Some(material) = materials.get(block) else {
            return self.albedo(block, hit, direction, style);
        };
        if style.pbr_enabled {
            if let Some(set_id) = material.pbr_texture {
                let (_, _, tint, alternate) = engine_scene::material::cell_uv(hit.cell, (0.0, 0.0));
                let selected_id = if material.cell_variation && alternate && set_id.0 == 1 {
                    TextureId(2)
                } else {
                    set_id
                };
                if let Some(set) = materials.get_pbr_texture(selected_id) {
                    let (mut u, mut v) = face_uv(hit);
                    if material.cell_variation {
                        let transformed = engine_scene::material::cell_uv(hit.cell, (u, v));
                        u = transformed.0;
                        v = transformed.1;
                    }
                    let color = set.albedo.sample(
                        u * material.uv_scale[0],
                        v * material.uv_scale[1],
                        hit.dist.max(1.0).log2(),
                    );
                    let texture_luminance =
                        0.2126 * color[0] + 0.7152 * color[1] + 0.0722 * color[2];
                    return classic_albedo(block, hit)
                        * (0.35 + texture_luminance * 1.3).clamp(0.35, 1.3)
                        * if material.cell_variation { tint } else { 1.0 };
                }
            }
        }
        match material.base_rule {
            BaseAlbedoRule::Constant => material.constant_albedo,
            BaseAlbedoRule::StudsClassic if material.texture == TextureKind::None => {
                classic_albedo(block, hit)
            }
            BaseAlbedoRule::StudsClassic => self.albedo(block, hit, direction, style),
        }
    }

    fn entity_surface(
        &self,
        kind: EntityKind,
        point: [f32; 3],
        axis: usize,
        sign: f32,
        part: u8,
    ) -> EntitySurface {
        match kind {
            WARDEN => warden_surface(point, axis, sign, part),
            PAGE => page_surface(point, axis),
            _ => EntitySurface {
                gray: 0.10,
                emissive: false,
            },
        }
    }
}

#[cfg(test)]
mod pbr_asset_tests {
    use super::studs_material_table;

    #[test]
    fn material_asset_loading_is_repeatable_and_pbr_sets_are_optional() {
        let first = studs_material_table();
        let second = studs_material_table();
        assert!(std::ptr::eq(first, second));
        // Every loaded set is still referenced only through stable scene data.
        assert!(first.pbr_textures.len() <= 4);
    }
}

fn apply_texture_modulation(base: f32, normalized_texel: f32, strength: f32) -> f32 {
    base * modulation_factor(normalized_texel, strength)
}

fn modulation_factor(normalized_texel: f32, strength: f32) -> f32 {
    (1.0 + (normalized_texel - 1.0) * strength).clamp(0.7, 1.3)
}

fn wall_texture_modulation(hit: &Hit, bricks: &Texture, strength: f32) -> f32 {
    let axis = if hit.normal[0] != 0 { 0 } else { 1 };
    let height = if axis == 0 { hit.point.y } else { hit.point.x };
    let row_position = hit.point.z * 2.0;
    let row = row_position.floor() as i32;
    let offset = if row & 1 == 1 { 0.5 } else { 0.0 };
    let horizontal = height + offset;
    let texel = bricks.sample_faded(
        horizontal.fract(),
        row_position.fract(),
        hit.dist,
        7.0,
        17.0,
    );
    let brick_variation = 0.92
        + 0.16
            * hash4(
                horizontal.floor() as i32,
                row,
                axis as i32 + hit.normal[axis],
                17,
            );
    modulation_factor(texel / bricks.average() * brick_variation, strength)
}

fn classic_albedo(block: u8, hit: &Hit) -> f32 {
    if hit.normal == [0, 0, 0] {
        return 0.0;
    }
    let point = hit.point;
    let axis = if hit.normal[0] != 0 {
        0
    } else if hit.normal[1] != 0 {
        1
    } else {
        2
    };

    if axis == 2 {
        let fx = point.x - point.x.floor();
        let fy = point.y - point.y.floor();
        let base = if block == CEIL { 0.30 } else { 0.40 };
        let distance = ((fx - 0.5) * (fx - 0.5) + (fy - 0.5) * (fy - 0.5)).sqrt();
        let mut albedo = base;
        if distance < 0.27 {
            albedo *= 1.25;
        } else if distance < 0.34 {
            albedo *= 0.6;
        }
        if fx < 0.025 || fx > 0.975 || fy < 0.025 || fy > 0.975 {
            albedo *= 0.65;
        }
        albedo * (1.0 + 0.05 * hash4(hit.cell.0, hit.cell.1, hit.cell.2, 7))
    } else {
        let height = if axis == 0 { point.y } else { point.x };
        let row_fraction = point.z * 2.0;
        let row = row_fraction.floor() as i32;
        let offset = if row & 1 == 1 { 0.5 } else { 0.0 };
        let horizontal = height + offset;
        if row_fraction.fract() < 0.10 || horizontal.fract() < 0.06 {
            0.20
        } else {
            let id = hash4(
                horizontal.floor() as i32,
                row,
                axis as i32 + 3 * hit.normal[axis],
                hit.cell.0 + hit.cell.1 * 31,
            );
            0.52 + 0.10 * id
        }
    }
}

fn warden_surface(point: [f32; 3], axis: usize, sign: f32, part: u8) -> EntitySurface {
    if part == 1 && axis == 0 && sign > 0.0 {
        let u = point[1].abs();
        let v = point[2];
        let eyes = u > 0.05 && u < 0.17 && v > 1.93 && v < 2.06;
        let mouth = u < 0.13 && v > 1.78 && v < 1.82;
        if eyes || mouth {
            return EntitySurface {
                gray: 1.0,
                emissive: true,
            };
        }
    }
    EntitySurface {
        gray: 0.10,
        emissive: false,
    }
}

fn page_surface(point: [f32; 3], axis: usize) -> EntitySurface {
    let mut gray = 0.92;
    if axis == 1 {
        let row = (point[2] + 0.25) * 9.0;
        if row.fract() < 0.45 && point[0].abs() < 0.15 && row > 1.0 && row < 7.5 {
            gray = 0.35;
        }
    }
    EntitySurface {
        gray,
        emissive: true,
    }
}

pub fn enemy_view(
    pos: Vect,
    yaw: f32,
    phase: f32,
    animation_state: usize,
    _eye: Vect,
) -> EntityInstance {
    EntityInstance {
        kind: WARDEN,
        pos,
        yaw,
        params: [phase, animation_state.min(4) as f32, 0.0, 0.0],
    }
}

pub fn page_view(pos: Vect, time: f32, index: f32) -> EntityInstance {
    EntityInstance {
        kind: PAGE,
        pos,
        yaw: 0.0,
        params: [time, index, 0.0, 0.0],
    }
}
#[cfg(test)]
mod tests {
    use super::{apply_texture_modulation, wall_texture_modulation};
    use engine_core::math::Vect;
    use engine_core::voxel::Hit;
    use engine_scene::texture::Texture;

    #[test]
    fn texture_modulation_preserves_classic_at_zero_and_stays_moderate() {
        let base = 0.52;
        for texel in [0.38, 0.46, 0.5, 0.62] {
            let normalized = texel / 0.5;
            assert_eq!(apply_texture_modulation(base, normalized, 0.0), base);
            let modulated = apply_texture_modulation(base, normalized, 1.0);
            let factor = modulated / base;
            assert!((0.7..=1.3).contains(&factor));
        }
    }

    #[test]
    fn wall_modulation_is_continuous_across_cell_boundaries() {
        let texture = Texture::bricks(64, 32, 17);
        let left = Hit {
            cell: (2, 4, 1),
            normal: [1, 0, 0],
            dist: 3.0,
            point: Vect::new(3.0, 4.375, 1.125),
        };
        let right = Hit {
            cell: (3, 4, 1),
            ..left
        };
        assert_eq!(
            wall_texture_modulation(&left, &texture, 1.0),
            wall_texture_modulation(&right, &texture, 1.0)
        );
    }

    #[test]
    fn procedural_wall_and_floor_modulation_stays_in_range() {
        let bricks = Texture::bricks(64, 32, 17);
        let noise = Texture::noise(64, 64, 41);
        for row in 0..4 {
            for step in 0..32 {
                let height = step as f32 / 32.0;
                let hit = Hit {
                    cell: (2, 4, 1),
                    normal: [1, 0, 0],
                    dist: 1.0,
                    point: Vect::new(3.0, height, row as f32 * 0.5 + 0.13),
                };
                assert!((0.7..=1.3).contains(&wall_texture_modulation(&hit, &bricks, 1.0)));
                let floor_factor = noise.sample_nearest(height, row as f32 / 4.0) / noise.average();
                assert!((0.7..=1.3).contains(&floor_factor));
            }
        }
    }
}
