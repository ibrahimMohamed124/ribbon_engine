//! Navigation agent steering output.
use crate::{NavPath, NavWorld};
use glam::Vec3;
use ribbon_app::{Time, World};
use ribbon_ecs::Entity;
use ribbon_scene::{GlobalTransform3D, Transform3D};
#[derive(Clone, Copy, Debug)]
pub struct NavAgent {
    pub speed: f32,
    pub arrive_radius: f32,
    pub repath_interval: f32,
    pub max_path_age: f32,
}
impl Default for NavAgent {
    fn default() -> Self {
        Self {
            speed: 3.0,
            arrive_radius: 0.3,
            repath_interval: 1.0,
            max_path_age: 5.0,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct NavDestination(pub Vec3);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavStatus {
    Idle,
    Moving,
    Arrived,
    NoPath,
}
#[derive(Clone, Copy, Debug)]
pub struct NavSteering {
    pub desired_velocity: Vec3,
    pub remaining_distance: f32,
    pub status: NavStatus,
}
#[derive(Clone, Copy, Debug)]
pub struct NavSettings {
    pub max_paths_per_update: usize,
    pub max_expansions: usize,
}
impl Default for NavSettings {
    fn default() -> Self {
        Self {
            max_paths_per_update: 4,
            max_expansions: 200_000,
        }
    }
}
#[derive(Default)]
pub(crate) struct NavAgentState {
    path: Option<NavPath>,
    next: usize,
    age: f32,
    last: Vec3,
}
#[derive(Default)]
pub(crate) struct PathCursor(pub usize);
pub(crate) fn nav_agent_system(world: &mut World) {
    let Some(nav) = world.try_resource::<NavWorld>().cloned() else {
        return;
    };
    let dt = world
        .try_resource::<Time>()
        .map_or(0.0, |t| t.delta.as_secs_f32().max(0.0));
    let settings = world
        .try_resource::<NavSettings>()
        .copied()
        .unwrap_or_default();
    let mut ids = world
        .query::<(&NavAgent, &NavDestination)>()
        .iter()
        .map(|(e, _)| e)
        .collect::<Vec<Entity>>();
    ids.sort_by_key(|e| e.to_bits());
    if ids.is_empty() {
        return;
    }
    let entity_count = ids.len();
    let start = world
        .try_resource::<PathCursor>()
        .map_or(0, |c| c.0 % entity_count);
    ids.rotate_left(start);
    let mut paths = 0;
    for e in ids {
        let Some(dest) = world.get::<NavDestination>(e).map(|d| d.0) else {
            continue;
        };
        let pos = world
            .get::<GlobalTransform3D>(e)
            .map(|t| t.translation)
            .or_else(|| world.get::<Transform3D>(e).map(|t| t.translation));
        let Some(pos) = pos else { continue };
        if world.get::<NavAgentState>(e).is_none() {
            let _ = world.insert(e, NavAgentState::default());
        }
        let Some(agent) = world.get::<NavAgent>(e).map(|a| *a) else {
            continue;
        };
        let Some(mut state) = world.get_mut::<NavAgentState>(e) else {
            continue;
        };
        state.age += dt;
        let changed = state.last.distance(dest) > nav.grid.cell_size;
        let stale = state.age >= agent.max_path_age.max(agent.repath_interval);
        let needs = state.path.is_none() || changed || stale;
        let mut status = NavStatus::Idle;
        let mut velocity = Vec3::ZERO;
        let mut remaining = 0.0;
        if needs && paths < settings.max_paths_per_update {
            paths += 1;
            state.last = dest;
            state.age = 0.0;
            state.path = nav
                .grid
                .find_path_with_limit(pos, dest, settings.max_expansions)
                .ok();
            state.next = 1;
        }
        if let Some(path) = state.path.clone() {
            let finalp = path.waypoints.last().copied().unwrap_or(dest);
            remaining = pos.distance(finalp);
            if remaining <= agent.arrive_radius {
                status = NavStatus::Arrived;
            } else if let Some(target) = path.waypoints.get(state.next).copied() {
                let delta = Vec3::new(target.x - pos.x, 0.0, target.z - pos.z);
                if delta.length() <= agent.arrive_radius {
                    state.next = (state.next + 1).min(path.waypoints.len().saturating_sub(1));
                }
                let to = path.waypoints.get(state.next).copied().unwrap_or(target);
                let direction = Vec3::new(to.x - pos.x, 0.0, to.z - pos.z);
                let frame_speed = if dt > 0.0 {
                    direction.length() / dt
                } else {
                    0.0
                };
                velocity = direction.normalize_or_zero() * agent.speed.max(0.0).min(frame_speed);
                status = NavStatus::Moving;
            }
        } else if needs {
            status = NavStatus::NoPath;
        }
        drop(state);
        if world.get::<NavSteering>(e).is_some() {
            if let Some(mut output) = world.get_mut::<NavSteering>(e) {
                output.desired_velocity = velocity;
                output.remaining_distance = remaining;
                output.status = status;
            }
        } else {
            let _ = world.insert(
                e,
                NavSteering {
                    desired_velocity: velocity,
                    remaining_distance: remaining,
                    status,
                },
            );
        }
    }
    if world.try_resource::<PathCursor>().is_some() {
        world.resource_mut::<PathCursor>().0 = (start + 1) % entity_count;
    }
}
