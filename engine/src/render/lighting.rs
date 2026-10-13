//! Distance falloff and surface shading interface.

use crate::math::Vect;
use crate::render::style::RenderStyle;
use crate::voxel::{face_ao, Hit, VoxelWorld};

pub const CONE_INNER: f32 = 0.985;
pub const CONE_OUTER: f32 = 0.86;

pub trait SurfaceShader: Sync {
    fn albedo(&self, block: u8, hit: &Hit, direction: Vect, style: &RenderStyle) -> f32;
}

pub struct SurfaceContext<'a> {
    pub illumination: f32,
    pub hit: &'a Hit,
    pub direction: Vect,
    pub cone: f32,
    pub empty_voxel: u8,
    pub style: &'a RenderStyle,
}

pub fn cone_factor(cosang: f32) -> f32 {
    let t = ((cosang - CONE_OUTER) / (CONE_INNER - CONE_OUTER)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub fn light(illumination: f32, distance: f32, ndotl: f32, cone: f32) -> f32 {
    light_with_style(illumination, distance, ndotl, cone, &RenderStyle::classic())
}

pub fn light_with_style(
    illumination: f32,
    distance: f32,
    ndotl: f32,
    cone: f32,
    style: &RenderStyle,
) -> f32 {
    let fog_distance = if style.fog_distance.is_finite() && style.fog_distance > 0.0 {
        style.fog_distance
    } else {
        super::RENDER_DIST
    };
    let fog = (1.0 - distance / fog_distance).clamp(0.0, 1.0);
    let ambient = style.ambient;
    let lamp = style.lamp / (1.0 + 0.7 * distance * distance);
    let beam =
        illumination * cone * (0.3 + 0.7 * ndotl) * (1.7 / (1.0 + 0.06 * distance * distance));
    (ambient + lamp + beam) * fog
}

pub fn shade_surface<S: SurfaceShader + ?Sized>(
    shader: &S,
    world: &VoxelWorld,
    context: SurfaceContext<'_>,
) -> f32 {
    let SurfaceContext {
        illumination,
        hit,
        direction,
        cone,
        empty_voxel,
        style,
    } = context;
    if hit.normal == [0, 0, 0] {
        return 0.0;
    }
    let block = world
        .get_checked(hit.cell.2, hit.cell.1, hit.cell.0)
        .unwrap_or(empty_voxel);
    let axis = if hit.normal[0] != 0 {
        0
    } else if hit.normal[1] != 0 {
        1
    } else {
        2
    };
    let ndotl = direction.axis(axis).abs();
    let ao_strength = if style.ao_strength.is_finite() {
        style.ao_strength.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let ao = if ao_strength == 0.0 {
        1.0
    } else {
        face_ao(world, hit, empty_voxel)
    };
    let occlusion = 1.0 + (ao - 1.0) * ao_strength;
    shader.albedo(block, hit, direction, style)
        * light_with_style(illumination, hit.dist, ndotl, cone, style)
        * occlusion
}
