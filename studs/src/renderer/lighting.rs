//! Lighting model and procedural block textures.

use super::RENDER_DIST;
use crate::math::{hash4, Vect};
use crate::world::{Hit, World, CEIL, WALL};

pub const SPOT_INNER: f32 = 0.985;
pub const SPOT_OUTER: f32 = 0.86;

/// Flashlight cone falloff (0..1) from cos(angle between pixel ray and view axis).
pub fn spot_factor(cosang: f32) -> f32 {
    let t = ((cosang - SPOT_OUTER) / (SPOT_INNER - SPOT_OUTER)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Total light at distance `d`. `flash` is the flashlight multiplier (0 = off).
pub fn light(flash: f32, d: f32, ndotl: f32, spot: f32) -> f32 {
    let fog = (1.0 - d / RENDER_DIST).clamp(0.0, 1.0);
    let ambient = 0.035;
    let lamp = 0.12 / (1.0 + 0.7 * d * d); // faint glow right around you
    let beam = flash * spot * (0.3 + 0.7 * ndotl) * (1.7 / (1.0 + 0.06 * d * d));
    (ambient + lamp + beam) * fog
}

/// Gray level of a voxel hit: procedural texture times lighting.
pub fn shade_world(world: &World, flash: f32, hit: &Hit, dir: Vect, spot: f32) -> f32 {
    if hit.normal == [0, 0, 0] {
        return 0.0;
    }
    let block = world
        .get_checked(hit.cell.2, hit.cell.1, hit.cell.0)
        .unwrap_or(WALL);
    let p = hit.point;
    let axis = if hit.normal[0] != 0 {
        0
    } else if hit.normal[1] != 0 {
        1
    } else {
        2
    };
    let ndotl = dir.axis(axis).abs();

    let albedo = if axis == 2 {
        // Roblox baseplate: studs on a plate with seams
        let fx = p.x - p.x.floor();
        let fy = p.y - p.y.floor();
        let base = if block == CEIL { 0.30 } else { 0.40 };
        let d = ((fx - 0.5) * (fx - 0.5) + (fy - 0.5) * (fy - 0.5)).sqrt();
        let mut a = base;
        if d < 0.27 {
            a *= 1.25;
        } else if d < 0.34 {
            a *= 0.6;
        }
        if fx < 0.025 || fx > 0.975 || fy < 0.025 || fy > 0.975 {
            a *= 0.65;
        }
        a * (1.0 + 0.05 * hash4(hit.cell.0, hit.cell.1, hit.cell.2, 7))
    } else {
        // brick wall
        let h = if axis == 0 { p.y } else { p.x };
        let rowf = p.z * 2.0;
        let row = rowf.floor() as i32;
        let off = if row & 1 == 1 { 0.5 } else { 0.0 };
        let hh = h + off;
        if rowf.fract() < 0.10 || hh.fract() < 0.06 {
            0.20
        } else {
            let id = hash4(
                hh.floor() as i32,
                row,
                axis as i32 + 3 * hit.normal[axis],
                hit.cell.0 + hit.cell.1 * 31,
            );
            0.52 + 0.10 * id
        }
    };
    albedo * light(flash, hit.dist, ndotl, spot)
}
