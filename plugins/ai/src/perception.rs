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
pub(crate) struct SightCursor(pub usize);
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
        return;
    }
    let mut targets = world
        .query::<&SightTarget>()
        .iter()
        .map(|(e, _)| e)
        .collect::<Vec<_>>();
    targets.sort_by_key(|e| e.to_bits());
    let start = world
        .try_resource::<SightCursor>()
        .map_or(0, |c| c.0 % observers.len());
    let count = settings.max_sight_checks_per_update.min(observers.len());
    for i in 0..count {
        let entity = observers[(start + i) % observers.len()];
        let Some(sight) = world.get::<Sight>(entity).map(|s| *s) else {
            continue;
        };
        let transform = world
            .get::<GlobalTransform3D>(entity)
            .map(|t| (t.translation, t.rotation))
            .or_else(|| {
                world
                    .get::<Transform3D>(entity)
                    .map(|t| (t.translation, t.rotation))
            });
        let Some((base, rotation)) = transform else {
            continue;
        };
        let eye = base + rotation * sight.eye_offset;
        let forward = rotation * Vec3::NEG_Z;
        let cos = (sight.fov_deg.to_radians() * 0.5).cos();
        let mut visible = Vec::new();
        for target in &targets {
            if *target == entity {
                continue;
            }
            let pos = world
                .get::<GlobalTransform3D>(*target)
                .map(|t| t.translation)
                .or_else(|| world.get::<Transform3D>(*target).map(|t| t.translation));
            let Some(pos) = pos else { continue };
            let delta = pos - eye;
            let distance = delta.length();
            if !distance.is_finite() || distance > sight.range || distance <= 1e-5 {
                continue;
            }
            if forward.dot(delta / distance) < cos {
                continue;
            }
            let mut query = RaycastQuery::new(eye, delta / distance, distance);
            query.exclude_entity = Some(entity);
            query.layers = sight.target_layers;
            query.include_sensors = false;
            match raycast(world, query) {
                Some(hit) if hit.entity == *target => visible.push(*target),
                Some(_) => {}
                None if world.get::<Collider>(*target).is_none() => visible.push(*target),
                None => {}
            }
        }
        if world.get::<Perceived>(entity).is_some() {
            if let Some(mut out) = world.get_mut::<Perceived>(entity) {
                out.visible.clear();
                out.visible.extend(visible);
            }
        } else {
            let _ = world.insert(entity, Perceived { visible });
        }
    }
    if world.try_resource::<SightCursor>().is_some() {
        world.resource_mut::<SightCursor>().0 = (start + count) % observers.len();
    }
}
