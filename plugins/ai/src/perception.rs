//! Budgeted physics-raycast sight perception.
use glam::Vec3;
use ribbon_app::World;
use ribbon_ecs::Entity;
use ribbon_plugin_physics::{raycast, Collider, CollisionLayers, RaycastQuery};
use ribbon_scene::{GlobalTransform3D, Transform3D};
#[derive(Clone, Copy, Debug)]
pub struct Sight {
    pub range: f32,
    pub fov_deg: f32,
    pub eye_offset: Vec3,
    pub target_layers: CollisionLayers,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct SightTarget;
#[derive(Clone, Debug, Default)]
pub struct Perceived {
    pub visible: Vec<Entity>,
}
#[derive(Clone, Copy, Debug)]
pub struct AiSettings {
    pub max_sight_checks_per_update: usize,
}
impl Default for AiSettings {
    fn default() -> Self {
        Self {
            max_sight_checks_per_update: 32,
        }
    }
}
#[derive(Default)]
pub(crate) struct SightCursor {
    pub(crate) observer: usize,
    pub(crate) target: usize,
    pub(crate) pending_observer: Option<Entity>,
    pub(crate) pending_visible: Vec<Entity>,
    observer_snapshot: Vec<Entity>,
    target_snapshot: Vec<Entity>,
}
pub(crate) fn perception_system(world: &mut World) {
    let settings = world
        .try_resource::<AiSettings>()
        .copied()
        .unwrap_or_default();
    let mut observers = world
        .query::<&Sight>()
        .iter()
        .map(|(e, _)| e)
        .collect::<Vec<_>>();
    observers.sort_by_key(|e| e.to_bits());
    if observers.is_empty() {
        world.insert_resource(SightCursor::default());
        return;
    }
    let mut targets = world
        .query::<&SightTarget>()
        .iter()
        .map(|(e, _)| e)
        .collect::<Vec<_>>();
    targets.sort_by_key(|e| e.to_bits());

    if targets.is_empty() {
        for entity in observers {
            let existing = if let Some(mut perceived) = world.get_mut::<Perceived>(entity) {
                perceived.visible.clear();
                true
            } else {
                false
            };
            if !existing {
                let _ = world.insert(entity, Perceived::default());
            }
        }
        world.insert_resource(SightCursor::default());
        return;
    }
    let Some(mut cursor) = world.remove_resource::<SightCursor>() else {
        world.insert_resource(SightCursor::default());
        return;
    };
    // A sliced scan must never publish results from an older entity set. Scene
    // streaming can add/remove observers or targets between updates, so restart
    // the current pass whenever either sorted snapshot changes.
    if cursor.observer_snapshot != observers || cursor.target_snapshot != targets {
        cursor.observer = 0;
        cursor.target = 0;
        cursor.pending_observer = observers.first().copied();
        cursor.pending_visible.clear();
        cursor.observer_snapshot.clone_from(&observers);
        cursor.target_snapshot.clone_from(&targets);
    }
    cursor.observer %= observers.len();
    cursor.target %= targets.len();
    if cursor.pending_observer != Some(observers[cursor.observer]) {
        cursor.pending_observer = Some(observers[cursor.observer]);
        cursor.pending_visible.clear();
    }

    // The budget counts every observer-target pair, including pairs rejected by
    // range/FOV, not merely the raycasts that happen to pass the cheap tests.
    let total_pairs = observers.len().saturating_mul(targets.len());
    let checks = settings.max_sight_checks_per_update.min(total_pairs);
    for _ in 0..checks {
        let entity = observers[cursor.observer];
        let target = targets[cursor.target];
        if entity != target {
            let sight = world.get::<Sight>(entity).map(|s| *s);
            let transform = world
                .get::<GlobalTransform3D>(entity)
                .map(|t| (t.translation, t.rotation))
                .or_else(|| {
                    world
                        .get::<Transform3D>(entity)
                        .map(|t| (t.translation, t.rotation))
                })
                .unwrap_or((Vec3::ZERO, glam::Quat::IDENTITY));
            if let Some(sight) = sight {
                let (base, rotation) = transform;
                let eye = base + rotation * sight.eye_offset;
                let forward = rotation * Vec3::NEG_Z;
                let cos = (sight.fov_deg.to_radians() * 0.5).cos();
                let pos = world
                    .get::<GlobalTransform3D>(target)
                    .map(|t| t.translation)
                    .or_else(|| world.get::<Transform3D>(target).map(|t| t.translation));
                if let Some(pos) = pos {
                    let delta = pos - eye;
                    let distance = delta.length();
                    if distance.is_finite()
                        && distance > 1e-5
                        && distance <= sight.range
                        && forward.dot(delta / distance) >= cos
                    {
                        let mut query = RaycastQuery::new(eye, delta / distance, distance);
                        query.exclude_entity = Some(entity);
                        query.layers = sight.target_layers;
                        query.include_sensors = false;
                        match raycast(world, query) {
                            Some(hit) if hit.entity == target => {
                                cursor.pending_visible.push(target);
                            }
                            None if world.get::<Collider>(target).is_none() => {
                                cursor.pending_visible.push(target);
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        cursor.target += 1;
        if cursor.target == targets.len() {
            if world.get::<Perceived>(entity).is_some() {
                if let Some(mut out) = world.get_mut::<Perceived>(entity) {
                    out.visible.clear();
                    out.visible.append(&mut cursor.pending_visible);
                }
            } else {
                let visible = std::mem::take(&mut cursor.pending_visible);
                let _ = world.insert(entity, Perceived { visible });
            }
            cursor.target = 0;
            cursor.observer = (cursor.observer + 1) % observers.len();
            cursor.pending_observer = Some(observers[cursor.observer]);
        }
    }
    world.insert_resource(cursor);
}
