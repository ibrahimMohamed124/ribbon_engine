//! Game-specific voxel materials and renderable shapes.

use std::sync::OnceLock;

use blocks_engine::gfx::texture::Texture;
use blocks_engine::math::{hash4, Vect};
use blocks_engine::render::entities::{Bounds3, Box3, EntView, EntitySurface};
use blocks_engine::render::lighting::SurfaceShader;
use blocks_engine::render::RenderStyle;
use blocks_engine::voxel::{face_uv, Hit};

use crate::level::{CEIL, FLOOR, WALL};

pub struct StudsSurface;

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

pub const ENEMY_BOUNDS: Bounds3 = ([-0.5, -0.75, 0.0], [1.0, 0.75, 2.25]);

fn enemy_boxes(phase: f32) -> Vec<Box3> {
    let swing = phase.sin() * 0.28;
    vec![
        ([-0.15 + swing, -0.35, 0.0], [0.15 + swing, -0.05, 0.9], 0),
        ([-0.15 - swing, 0.05, 0.0], [0.15 - swing, 0.35, 0.9], 0),
        ([-0.2, -0.4, 0.9], [0.2, 0.4, 1.7], 0),
        ([-0.1, -0.65, 1.35], [0.85, -0.4, 1.7], 0),
        ([-0.1, 0.4, 1.35], [0.85, 0.65, 1.7], 0),
        ([-0.25, -0.25, 1.7], [0.25, 0.25, 2.2], 1),
    ]
}

fn page_boxes() -> Vec<Box3> {
    vec![([-0.2, -0.015, -0.25], [0.2, 0.015, 0.25], 0)]
}

fn enemy_surface(point: [f32; 3], axis: usize, sign: f32, part: u8) -> EntitySurface {
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

fn page_surface(point: [f32; 3], axis: usize, _sign: f32, _part: u8) -> EntitySurface {
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

pub fn enemy_view(pos: Vect, yaw: f32, phase: f32, eye: Vect) -> EntView {
    EntView::new(
        pos,
        yaw,
        eye,
        enemy_boxes(phase),
        Some(ENEMY_BOUNDS),
        enemy_surface,
    )
}

pub fn page_view(pos: Vect, yaw: f32, eye: Vect) -> EntView {
    EntView::new(pos, yaw, eye, page_boxes(), None, page_surface)
}
#[cfg(test)]
mod tests {
    use super::{apply_texture_modulation, wall_texture_modulation};
    use blocks_engine::gfx::texture::Texture;
    use blocks_engine::math::Vect;
    use blocks_engine::voxel::Hit;

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
