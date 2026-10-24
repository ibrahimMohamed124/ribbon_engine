//! Distance falloff and surface shading interface.

use crate::entities::EntityKind;
use crate::material::MaterialTable;
use crate::style::RenderStyle;
use ribbon_math::Vect;
use ribbon_plugin_voxel::{face_ao, face_uv, Hit, VoxelWorld};

pub const CONE_INNER: f32 = 0.985;
pub const CONE_OUTER: f32 = 0.86;

pub trait SurfaceShader: Sync {
    fn albedo(&self, block: u8, hit: &Hit, direction: Vect, style: &RenderStyle) -> f32;

    fn albedo_with_materials(
        &self,
        block: u8,
        hit: &Hit,
        direction: Vect,
        style: &RenderStyle,
        _materials: &MaterialTable,
    ) -> f32 {
        self.albedo(block, hit, direction, style)
    }

    fn entity_surface(
        &self,
        _kind: EntityKind,
        _point: [f32; 3],
        _axis: usize,
        _sign: f32,
        _part: u8,
    ) -> EntitySurface {
        EntitySurface {
            gray: 0.9,
            emissive: false,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct EntitySurface {
    pub gray: f32,
    pub emissive: bool,
}

pub struct SurfaceContext<'a> {
    pub light_level: f32,
    pub hit: &'a Hit,
    pub direction: Vect,
    pub cone: f32,
    pub empty_voxel: u8,
    pub style: &'a RenderStyle,
    pub materials: &'a MaterialTable,
}

pub fn cone_factor(cosang: f32) -> f32 {
    let t = ((cosang - CONE_OUTER) / (CONE_INNER - CONE_OUTER)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub fn light(light_level: f32, distance: f32, ndotl: f32, cone: f32) -> f32 {
    let fog = (1.0 - distance / crate::RENDER_DIST).clamp(0.0, 1.0);
    let ambient = 0.035;
    let lamp = 0.12 / (1.0 + 0.7 * distance * distance);
    let beam =
        light_level * cone * (0.3 + 0.7 * ndotl) * (1.7 / (1.0 + 0.06 * distance * distance));
    (ambient + lamp + beam) * fog
}

pub fn light_with_style(
    light_level: f32,
    distance: f32,
    ndotl: f32,
    cone: f32,
    style: &RenderStyle,
) -> f32 {
    let fog_distance = if style.fog_distance.is_finite() && style.fog_distance > 0.0 {
        style.fog_distance
    } else {
        crate::RENDER_DIST
    };
    let fog = (1.0 - distance / fog_distance).clamp(0.0, 1.0);
    let ambient = style.ambient;
    let lamp = style.ambient_fill / (1.0 + 0.7 * distance * distance);
    let beam =
        light_level * cone * (0.3 + 0.7 * ndotl) * (1.7 / (1.0 + 0.06 * distance * distance));
    (ambient + lamp + beam) * fog
}

/// Returns zero when a solid voxel blocks the segment back to the directed source.
pub fn spot_light_visibility(
    world: &VoxelWorld,
    point: Vect,
    normal: [i32; 3],
    to_light: Vect,
    max_distance: f32,
    empty_voxel: u8,
) -> f32 {
    let origin = Vect::new(
        point.x + normal[0] as f32 * 0.02,
        point.y + normal[1] as f32 * 0.02,
        point.z + normal[2] as f32 * 0.02,
    );
    if world
        .cast(origin, to_light, max_distance.max(0.0), empty_voxel)
        .is_some()
    {
        0.0
    } else {
        1.0
    }
}

pub fn shade_surface<S: SurfaceShader + ?Sized>(
    shader: &S,
    world: &VoxelWorld,
    context: SurfaceContext<'_>,
) -> f32 {
    let SurfaceContext {
        light_level,
        hit,
        direction,
        cone,
        empty_voxel,
        style,
        materials,
    } = context;
    if hit.normal == [0, 0, 0] {
        return 0.0;
    }
    let block = world
        .get(hit.cell.0, hit.cell.1, hit.cell.2)
        .unwrap_or(empty_voxel);
    let axis = if hit.normal[0] != 0 {
        0
    } else if hit.normal[1] != 0 {
        1
    } else {
        2
    };
    let mut ndotl = direction.axis(axis).abs();
    let pbr = if style.pbr_enabled {
        materials
            .get(block)
            .and_then(|material| {
                material.pbr_texture.and_then(|id| {
                    let (_, _, _, alternate) = crate::material::cell_uv(hit.cell, (0.0, 0.0));
                    let id = if material.cell_variation && alternate && id.0 == 1 {
                        crate::texture::TextureId(2)
                    } else {
                        id
                    };
                    materials.get_pbr_texture(id).map(|set| (material, set))
                })
            })
            .map(|(material, set)| {
                let (mut u, mut v) = face_uv(hit);
                if material.cell_variation {
                    let transformed = crate::material::cell_uv(hit.cell, (u, v));
                    u = transformed.0;
                    v = transformed.1;
                }
                u *= material.uv_scale[0];
                v *= material.uv_scale[1];
                let lod = hit.dist.max(1.0).log2();
                let normal = set.normal.sample(u, v, lod);
                let tangent_normal = Vect::new(
                    normal[0] * 2.0 - 1.0,
                    normal[1] * 2.0 - 1.0,
                    normal[2] * 2.0 - 1.0,
                );
                let face_normal = Vect::new(
                    hit.normal[0] as f32,
                    hit.normal[1] as f32,
                    hit.normal[2] as f32,
                );
                let (tangent, bitangent) = match axis {
                    0 => (
                        Vect::new(0.0, hit.normal[0] as f32, 0.0),
                        Vect::new(0.0, 0.0, 1.0),
                    ),
                    1 => (Vect::new(1.0, 0.0, 0.0), Vect::new(0.0, 0.0, 1.0)),
                    _ => (Vect::new(1.0, 0.0, 0.0), Vect::new(0.0, 1.0, 0.0)),
                };
                let mapped = (tangent.scale(tangent_normal.x)
                    + bitangent.scale(tangent_normal.y)
                    + face_normal.scale(tangent_normal.z))
                .normalize();
                ndotl = mapped.dot(direction.scale(-1.0)).clamp(0.0, 1.0);
                let roughness = set.roughness.sample(u, v, lod)[0].clamp(0.04, 1.0);
                let ao = set.ambient_occlusion.sample(u, v, lod)[0].clamp(0.0, 1.0);
                (roughness, ao)
            })
    } else {
        None
    };
    let ao_strength = if style.ao_strength.is_finite() {
        style.ao_strength.clamp(0.0, 1.0)
    } else {
        0.0
    };
    // The screen-space contribution becomes sub-pixel at range. Skip the
    // neighborhood queries there to avoid spending work on invisible AO.
    let ao_strength = ao_strength * (1.0 - hit.dist / 4.0).clamp(0.0, 1.0);
    let ao = if ao_strength == 0.0 {
        1.0
    } else {
        face_ao(world, hit, empty_voxel)
    };
    let mut occlusion = 1.0 + (ao - 1.0) * ao_strength;
    if let Some((_, material_ao)) = pbr {
        occlusion *= material_ao;
    }
    let shadowed_cone = if style.spot_light_shadows && light_level > 0.0 && cone > 0.0 {
        cone * spot_light_visibility(
            world,
            hit.point,
            hit.normal,
            direction.scale(-1.0),
            hit.dist,
            empty_voxel,
        )
    } else {
        cone
    };
    let diffuse = light_with_style(light_level, hit.dist, ndotl, shadowed_cone, style);
    let specular = pbr.map_or(0.0, |(roughness, _)| {
        let fog = (1.0 - hit.dist / style.fog_distance.max(0.001)).clamp(0.0, 1.0);
        let falloff = light_level * shadowed_cone / (1.0 + 0.06 * hit.dist * hit.dist);
        falloff * (1.0 - roughness) * 0.22 * ndotl.powf(2.0 + 32.0 * (1.0 - roughness)) * fog
    });
    shader.albedo_with_materials(block, hit, direction, style, materials)
        * (diffuse + specular)
        * occlusion
}

#[cfg(test)]
mod shadow_tests {
    use super::{light_with_style, spot_light_visibility};
    use ribbon_math::Vect;
    use ribbon_plugin_voxel::VoxelWorld;

    #[test]
    fn voxel_between_source_and_surface_blocks_the_beam() {
        let mut clear = vec![0; 5 * 3 * 3];
        let eye = Vect::new(0.5, 1.5, 1.5);
        let surface = Vect::new(3.0, 1.5, 1.5);
        let normal = [-1, 0, 0];
        let to_light = Vect::new(-1.0, 0.0, 0.0);
        let open = VoxelWorld::new(5, 3, 3, clear.clone());
        assert_eq!(
            spot_light_visibility(&open, surface, normal, to_light, 2.5, 0),
            1.0
        );
        clear[21] = 1;
        let blocked = VoxelWorld::new(5, 3, 3, clear);
        assert_eq!(
            spot_light_visibility(&blocked, surface, normal, to_light, 2.5, 0),
            0.0
        );
        assert!(eye.x < surface.x);
    }

    #[test]
    fn realistic_light_falloff_is_finite_and_monotonic() {
        let style = crate::RenderStyle::default();
        let values: Vec<_> = (1..=12)
            .map(|distance| light_with_style(1.0, distance as f32, 1.0, 1.0, &style))
            .collect();
        assert!(values.iter().all(|value| value.is_finite()));
        assert!(values.windows(2).all(|pair| pair[0] >= pair[1]));
    }
}
