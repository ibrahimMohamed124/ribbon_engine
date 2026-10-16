//! The hunter: pathfinding and the "moves only when you look away" rule.

use std::collections::VecDeque;

use crate::world::{World, FLOOR_TOP, X_BLOCKS, Y_BLOCKS};
use engine_core::math::{forward_vec, Angles, Vect};

pub const ENEMY_AWAKE_AFTER: f32 = 12.0; // seconds
pub const ENEMY_BASE_SPEED: f32 = 3.6;
pub const ENEMY_SPEED_PER_PAGE: f32 = 0.55;
pub const SEEN_SPEED_FACTOR: f32 = 0.10;
pub const CATCH_DIST: f32 = 0.85;
pub const ENEMY_SIGHT_RANGE: f32 = 12.0;
pub const PROXIMITY_RANGE: f32 = 10.0;
const SEARCH_DURATION: f32 = 7.0;

pub fn proximity_intensity(distance: f32, awake: bool) -> f32 {
    if !awake || !distance.is_finite() {
        return 0.0;
    }
    ((PROXIMITY_RANGE - distance.max(0.0)) / PROXIMITY_RANGE).clamp(0.0, 1.0)
}

pub fn flashlight_pulse_factor(intensity: f32, pulse: bool, limit_flicker: bool) -> f32 {
    let intensity = if intensity.is_finite() {
        intensity.clamp(0.0, 1.0)
    } else {
        0.0
    };
    if limit_flicker {
        1.0 - intensity * 0.12
    } else if pulse {
        1.0 - intensity * 0.55
    } else {
        1.0
    }
}

pub fn footstep_crossings(previous_phase: f32, current_phase: f32) -> u32 {
    if !previous_phase.is_finite() || !current_phase.is_finite() || current_phase <= previous_phase
    {
        return 0;
    }
    let period = std::f32::consts::PI;
    ((current_phase / period).floor() - (previous_phase / period).floor()) as u32
}

/// Minimum dot(view direction, direction to enemy) for the player to "see" it.
const VIEW_CONE_DOT: f32 = 0.82;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum EnemyAnimation {
    Idle = 0,
    Walk = 1,
    RunLunge = 2,
    Attack = 3,
    Frozen = 4,
}

pub fn animation_state(
    awake: bool,
    seen: bool,
    distance: f32,
    speed: f32,
    moving: bool,
) -> EnemyAnimation {
    if !awake {
        EnemyAnimation::Idle
    } else if distance <= CATCH_DIST {
        EnemyAnimation::Attack
    } else if seen {
        EnemyAnimation::Frozen
    } else if distance <= CATCH_DIST * 3.0 || speed >= 4.8 {
        EnemyAnimation::RunLunge
    } else if moving {
        EnemyAnimation::Walk
    } else {
        EnemyAnimation::Idle
    }
}

pub struct Enemy {
    pub pos: Vect, // feet position
    pub yaw: f32,
    pub phase: f32, // walk-cycle phase, drives leg animation
    pub target: Option<(i32, i32)>,
    pub moving: bool,
    pub last_known_player: Option<Vect>,
    pub has_seen_player: bool,
    search_elapsed: f32,
    search_target: Option<Vect>,
    search_target_index: usize,
}

impl Enemy {
    pub fn new(x: f32, y: f32) -> Enemy {
        Enemy {
            pos: Vect::new(x, y, FLOOR_TOP),
            yaw: 0.0,
            phase: 0.0,
            target: None,
            moving: false,
            last_known_player: None,
            has_seen_player: false,
            search_elapsed: 0.0,
            search_target: None,
            search_target_index: 0,
        }
    }

    /// Does the enemy hunt yet? It sleeps for a while, or until the first page is taken.
    pub fn awake(time: f32, pages_collected: usize) -> bool {
        time > ENEMY_AWAKE_AFTER || pages_collected > 0
    }

    /// Chase speed: faster with every page, nearly frozen while being looked at.
    pub fn speed(pages_collected: usize, seen: bool) -> f32 {
        Self::speed_with_tuning(pages_collected, seen, 1.0, SEEN_SPEED_FACTOR)
    }

    /// Chase speed with per-run tuning; the default `speed` retains normal behavior.
    pub fn speed_with_tuning(
        pages_collected: usize,
        seen: bool,
        multiplier: f32,
        seen_factor: f32,
    ) -> f32 {
        let mut speed = ENEMY_BASE_SPEED + ENEMY_SPEED_PER_PAGE * pages_collected as f32;
        speed *= multiplier;
        if seen {
            speed *= seen_factor;
        }
        speed
    }

    pub fn has_caught(&self, player: Vect) -> bool {
        self.pos.dist_xy(player) < CATCH_DIST
    }

    pub fn update(&mut self, world: &World, player: Vect, speed: f32, dt: f32) {
        self.update_hunt(world, player, true, speed, dt);
    }

    /// Hunt the player's last visible position; search nearby briefly after losing sight.
    /// Returns the distance travelled this frame for footstep timing.
    pub fn update_hunt(
        &mut self,
        world: &World,
        player: Vect,
        player_visible: bool,
        speed: f32,
        dt: f32,
    ) -> f32 {
        if player_visible {
            self.last_known_player = Some(player);
            self.has_seen_player = true;
            self.search_elapsed = 0.0;
            self.search_target = None;
        }

        let destination = if player_visible {
            player
        } else if let Some(last_known) = self.last_known_player {
            if self.pos.dist_xy(last_known) > 0.85 {
                last_known
            } else {
                self.search_elapsed += dt;
                if self.search_elapsed >= SEARCH_DURATION {
                    self.last_known_player = None;
                    self.search_target = None;
                    self.target = None;
                    self.moving = false;
                    return 0.0;
                }
                if self
                    .search_target
                    .is_none_or(|target| self.pos.dist_xy(target) < 0.3)
                {
                    self.search_target = self.next_search_target(world, last_known);
                }
                match self.search_target {
                    Some(target) => target,
                    None => last_known,
                }
            }
        } else {
            self.moving = false;
            if self.has_seen_player {
                self.yaw = (self.yaw + dt * 0.8).rem_euclid(std::f32::consts::TAU);
            }
            return 0.0;
        };

        self.move_toward(world, destination, speed, dt)
    }

    pub fn can_see_player(&self, player: Vect, world: &World) -> bool {
        let eye = Vect::new(self.pos.x, self.pos.y, self.pos.z + 1.5);
        let chest = Vect::new(player.x, player.y, player.z + 1.2);
        let to = Vect::new(chest.x - eye.x, chest.y - eye.y, chest.z - eye.z);
        let dist = (to.x * to.x + to.y * to.y + to.z * to.z).sqrt();
        if dist > ENEMY_SIGHT_RANGE || dist < 1e-3 {
            return false;
        }
        let direction = to.scale(1.0 / dist);
        let facing = forward_vec(Angles {
            psi: 0.0,
            phi: self.yaw,
        });
        if direction.dot(facing) < 0.2 {
            return false;
        }
        world
            .cast(eye, direction, dist)
            .is_none_or(|hit| hit.dist > dist - 0.4)
    }

    fn next_search_target(&mut self, world: &World, around: Vect) -> Option<Vect> {
        const OFFSETS: [(i32, i32); 8] = [
            (1, 0),
            (0, 1),
            (-1, 0),
            (0, -1),
            (1, 1),
            (-1, 1),
            (-1, -1),
            (1, -1),
        ];
        let center = (around.x.floor() as i32, around.y.floor() as i32);
        for _ in 0..OFFSETS.len() {
            let (dx, dy) = OFFSETS[self.search_target_index % OFFSETS.len()];
            self.search_target_index += 1;
            let tile = (center.0 + dx, center.1 + dy);
            if world.tile_open(tile.0, tile.1) {
                return Some(Vect::new(
                    tile.0 as f32 + 0.5,
                    tile.1 as f32 + 0.5,
                    self.pos.z,
                ));
            }
        }
        None
    }

    fn move_toward(&mut self, world: &World, destination: Vect, speed: f32, dt: f32) -> f32 {
        self.moving = false;
        if speed <= 0.0 {
            return 0.0;
        }
        let tile = (self.pos.x.floor() as i32, self.pos.y.floor() as i32);
        let ptile = (destination.x.floor() as i32, destination.y.floor() as i32);

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
            _ => (destination.x, destination.y),
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
            return step;
        }
        0.0
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
