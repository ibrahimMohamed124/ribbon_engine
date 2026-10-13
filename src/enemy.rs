//! The hunter: pathfinding and the "moves only when you look away" rule.

use std::collections::VecDeque;

use crate::math::{forward_vec, Angles, Vect};
use crate::world::{World, FLOOR_TOP, X_BLOCKS, Y_BLOCKS};

pub const ENEMY_AWAKE_AFTER: f32 = 12.0; // seconds
pub const ENEMY_BASE_SPEED: f32 = 3.6;
pub const ENEMY_SPEED_PER_PAGE: f32 = 0.55;
pub const SEEN_SPEED_FACTOR: f32 = 0.10;
pub const CATCH_DIST: f32 = 0.85;

/// Minimum dot(view direction, direction to enemy) for the player to "see" it.
const VIEW_CONE_DOT: f32 = 0.82;

pub struct Enemy {
    pub pos: Vect, // feet position
    pub yaw: f32,
    pub phase: f32, // walk-cycle phase, drives leg animation
    pub target: Option<(i32, i32)>,
    pub moving: bool,
}

impl Enemy {
    pub fn new(x: f32, y: f32) -> Enemy {
        Enemy {
            pos: Vect::new(x, y, FLOOR_TOP),
            yaw: 0.0,
            phase: 0.0,
            target: None,
            moving: false,
        }
    }

    /// Does the enemy hunt yet? It sleeps for a while, or until the first page is taken.
    pub fn awake(time: f32, pages_collected: usize) -> bool {
        time > ENEMY_AWAKE_AFTER || pages_collected > 0
    }

    /// Chase speed: faster with every page, nearly frozen while being looked at.
    pub fn speed(pages_collected: usize, seen: bool) -> f32 {
        let mut speed = ENEMY_BASE_SPEED + ENEMY_SPEED_PER_PAGE * pages_collected as f32;
        if seen {
            speed *= SEEN_SPEED_FACTOR;
        }
        speed
    }

    pub fn has_caught(&self, player: Vect) -> bool {
        self.pos.dist_xy(player) < CATCH_DIST
    }

    pub fn update(&mut self, world: &World, player: Vect, speed: f32, dt: f32) {
        self.moving = false;
        if speed <= 0.0 {
            return;
        }
        let tile = (self.pos.x.floor() as i32, self.pos.y.floor() as i32);
        let ptile = (player.x.floor() as i32, player.y.floor() as i32);

        if let Some(t) = self.target {
            let dx = t.0 as f32 + 0.5 - self.pos.x;
            let dy = t.1 as f32 + 0.5 - self.pos.y;
            if dx * dx + dy * dy < 0.08 * 0.08 {
                self.target = None;
            }
        }
        if self.target.is_none() && tile != ptile {
            self.target = bfs_next(world, tile, ptile);
        }

        let (gx, gy) = match self.target {
            Some(t) if tile != ptile => (t.0 as f32 + 0.5, t.1 as f32 + 0.5),
            _ => (player.x, player.y),
        };
        let dx = gx - self.pos.x;
        let dy = gy - self.pos.y;
        let dist = (dx * dx + dy * dy).sqrt();
        if dist > 1e-4 {
            let step = (speed * dt).min(dist);
            self.pos.x += dx / dist * step;
            self.pos.y += dy / dist * step;
            self.yaw = dy.atan2(dx);
            self.phase += step * 2.4;
            self.moving = true;
        }
    }

    /// Can a viewer at `eye` looking along `view` see the enemy right now
    /// (inside the view cone, with a clear line, within `max_dist`)?
    pub fn is_seen_from(&self, eye: Vect, view: Angles, world: &World, max_dist: f32) -> bool {
        let chest = Vect::new(self.pos.x, self.pos.y, self.pos.z + 1.4);
        let to = Vect::new(chest.x - eye.x, chest.y - eye.y, chest.z - eye.z);
        let dist = (to.x * to.x + to.y * to.y + to.z * to.z).sqrt();
        if dist > max_dist || dist < 1e-3 {
            return false;
        }
        let dirn = to.scale(1.0 / dist);
        if dirn.dot(forward_vec(view)) < VIEW_CONE_DOT {
            return false;
        }
        match world.cast(eye, dirn, dist) {
            Some(h) => h.dist > dist - 0.4,
            None => true,
        }
    }
}

/// Next tile on the shortest path from `from` to `to` (BFS over open tiles).
pub fn bfs_next(world: &World, from: (i32, i32), to: (i32, i32)) -> Option<(i32, i32)> {
    if !world.tile_open(from.0, from.1) || !world.tile_open(to.0, to.1) {
        return None;
    }
    let idx = |x: i32, y: i32| y as usize * X_BLOCKS + x as usize;
    let mut dist = vec![i32::MAX; X_BLOCKS * Y_BLOCKS];
    let mut q = VecDeque::new();
    dist[idx(to.0, to.1)] = 0;
    q.push_back(to);
    const DIRS: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
    while let Some((x, y)) = q.pop_front() {
        let dcur = dist[idx(x, y)];
        for (dx, dy) in DIRS {
            let (nx, ny) = (x + dx, y + dy);
            if world.tile_open(nx, ny) && dist[idx(nx, ny)] == i32::MAX {
                dist[idx(nx, ny)] = dcur + 1;
                q.push_back((nx, ny));
            }
        }
    }
    let d0 = dist[idx(from.0, from.1)];
    if d0 == i32::MAX || d0 == 0 {
        return None;
    }
    for (dx, dy) in DIRS {
        let (nx, ny) = (from.0 + dx, from.1 + dy);
        if world.tile_open(nx, ny) && dist[idx(nx, ny)] == d0 - 1 {
            return Some((nx, ny));
        }
    }
    None
}
