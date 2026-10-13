//! Distance falloff and surface shading interface.

use crate::math::Vect;
use crate::voxel::{Hit, VoxelWorld};

pub const CONE_INNER: f32 = 0.985;
pub const CONE_OUTER: f32 = 0.86;

pub trait SurfaceShader: Sync {
    fn albedo(&self, block: u8, hit: &Hit, direction: Vect) -> f32;
}

pub fn cone_factor(cosang: f32) -> f32 {
    let t = ((cosang - CONE_OUTER) / (CONE_INNER - CONE_OUTER)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub fn light(illumination: f32, distance: f32, ndotl: f32, cone: f32) -> f32 {
    let fog = (1.0 - distance / super::RENDER_DIST).clamp(0.0, 1.0);
    let ambient = 0.035;
    let lamp = 0.12 / (1.0 + 0.7 * distance * distance);
    let beam =
        illumination * cone * (0.3 + 0.7 * ndotl) * (1.7 / (1.0 + 0.06 * distance * distance));
    (ambient + lamp + beam) * fog
}

pub fn shade_surface<S: SurfaceShader + ?Sized>(
    shader: &S,
    world: &VoxelWorld,
    illumination: f32,
    hit: &Hit,
    direction: Vect,
    cone: f32,
    empty_voxel: u8,
) -> f32 {
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
    shader.albedo(block, hit, direction) * light(illumination, hit.dist, ndotl, cone)
}
