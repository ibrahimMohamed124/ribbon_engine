//! Game-specific voxel materials and renderable shapes.

use blocks_engine::math::{hash4, Vect};
use blocks_engine::render::entities::{Bounds3, Box3, EntView, EntitySurface};
use blocks_engine::render::lighting::SurfaceShader;
use blocks_engine::voxel::Hit;

use crate::level::CEIL;

pub struct StudsSurface;

impl SurfaceShader for StudsSurface {
    fn albedo(&self, block: u8, hit: &Hit, _direction: Vect) -> f32 {
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
