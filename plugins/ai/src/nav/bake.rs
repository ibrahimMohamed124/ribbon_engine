//! Physics-raycast nav-grid baking from a world-space AABB.
use super::NavGrid;
use glam::{Vec2, Vec3};
use ribbon_ecs::World;
use ribbon_plugin_physics::{raycast, RaycastQuery};
#[derive(Clone, Copy, Debug)]
pub struct NavBakeConfig {
    pub min: Vec3,
    pub max: Vec3,
    pub cell_size: f32,
    pub agent_radius: f32,
    pub agent_height: f32,
    pub max_step_height: f32,
    pub max_slope_deg: f32,
    pub probe_height_above_ground: f32,
}
pub fn bake_nav_grid(world: &World, cfg: &NavBakeConfig) -> NavGrid {
    let cell = cfg.cell_size.max(0.01);
    let width = ((cfg.max.x - cfg.min.x) / cell).ceil().max(0.0) as u32;
    let height = ((cfg.max.z - cfg.min.z) / cell).ceil().max(0.0) as u32;
    let mut grid = NavGrid::new(Vec2::new(cfg.min.x, cfg.min.z), cell, width, height);
    let slope = cfg.max_slope_deg.to_radians().cos();
    for y in 0..height {
        for x in 0..width {
            let p = grid.cell_center(x, y);
            let origin = Vec3::new(p.x, cfg.min.y + cfg.probe_height_above_ground, p.z);
            let distance = cfg.probe_height_above_ground + (cfg.max.y - cfg.min.y);
            let mut query = RaycastQuery::new(origin, Vec3::NEG_Y, distance.max(0.0));
            query.include_sensors = false;
            if let Some(hit) = raycast(world, query) {
                if hit.distance <= 0.001 || hit.normal.y < slope || hit.position.y > cfg.max.y {
                    grid.set_walkable(x, y, false);
                    continue;
                }
                grid.set_ground(x, y, hit.position.y);
                let mut clear = RaycastQuery::new(
                    hit.position + Vec3::Y * 0.05,
                    Vec3::Y,
                    cfg.agent_height.max(0.0),
                );
                clear.include_sensors = false;
                if raycast(world, clear).is_some() {
                    grid.set_walkable(x, y, false);
                }
            } else {
                grid.set_walkable(x, y, false);
            }
        }
    }
    let erode = (cfg.agent_radius / cell).ceil().max(0.0) as i32;
    let mut snapshot = Vec::with_capacity((width as usize).saturating_mul(height as usize));
    for y in 0..height {
        for x in 0..width {
            snapshot.push(grid.cell(x, y).is_some_and(|c| c.walkable));
        }
    }
    for y in 0..height {
        for x in 0..width {
            if !grid.cell(x, y).is_some_and(|c| c.walkable) {
                continue;
            }
            let mut clear = true;
            for dy in -erode..=erode {
                for dx in -erode..=erode {
                    let nx = x as i32 + dx;
                    let ny = y as i32 + dy;
                    if nx < 0
                        || ny < 0
                        || nx >= width as i32
                        || ny >= height as i32
                        || !snapshot[(ny as u32 * width + nx as u32) as usize]
                    {
                        clear = false;
                    }
                }
            }
            if !clear {
                grid.set_walkable(x, y, false);
            }
        }
    }
    let mut heights = Vec::with_capacity((width as usize).saturating_mul(height as usize));
    for y in 0..height {
        for x in 0..width {
            heights.push(grid.cell(x, y).map_or(0.0, |c| c.ground_y));
        }
    }
    for y in 0..height {
        for x in 0..width {
            if !grid.cell(x, y).is_some_and(|c| c.walkable) {
                continue;
            }
            let h = heights[(y * width + x) as usize];
            for (nx, ny) in [(x.saturating_add(1), y), (x, y.saturating_add(1))] {
                if nx < width && ny < height && grid.cell(nx, ny).is_some_and(|c| c.walkable) {
                    let nh = heights[(ny * width + nx) as usize];
                    let diff = (h - nh).abs();
                    if diff > cfg.max_step_height * 2.0 {
                        grid.set_walkable(x, y, false);
                        grid.set_walkable(nx, ny, false);
                    } else if diff > cfg.max_step_height {
                        grid.set_cost(
                            x,
                            y,
                            grid.cell(x, y).map_or(1, |c| c.cost.saturating_mul(3)),
                        );
                        grid.set_cost(
                            nx,
                            ny,
                            grid.cell(nx, ny).map_or(1, |c| c.cost.saturating_mul(3)),
                        );
                    }
                }
            }
        }
    }
    grid
}
