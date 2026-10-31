//! Reusable mesh-based rain simulation.
//!
//! The plugin owns rain-particle lifetime and movement. Games only toggle [`RainSettings`]
//! and may tune its parameters when entering a weather region.

use glam::{Quat, Vec3};
use ribbon_app::{App, Stage, Time, World};
use ribbon_ecs::Entity;
use ribbon_plugins::Plugin;
use ribbon_render_contracts::{BuiltinMesh, Camera3D, MeshInstance, Transform3D};
use std::time::Instant;

const DEFAULT_RAIN_SEED: u32 = 0x51A7_2026;
const DROP_STREAK_LENGTH: f32 = 0.42;

/// Configuration for camera-relative rain. Defaults retain ATHAR's existing visual density,
/// palette, speed range, and local coverage.
#[derive(Clone, Copy, Debug)]
pub struct RainSettings {
    pub enabled: bool,
    pub particle_count: usize,
    pub radius: f32,
    pub minimum_speed: f32,
    pub maximum_speed: f32,
    pub wind: Vec3,
    pub color: [f32; 3],
    pub ground_offset: f32,
}

impl Default for RainSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            particle_count: 220,
            radius: 21.0,
            minimum_speed: 11.0,
            maximum_speed: 20.0,
            wind: Vec3::ZERO,
            color: [0.47, 0.56, 0.67],
            ground_offset: 2.0,
        }
    }
}

impl RainSettings {
    /// Number of particles configured to be active for the current weather state.
    pub fn active_particles(&self) -> usize {
        if self.enabled {
            self.particle_count
        } else {
            0
        }
    }
}

/// State attached to each engine-owned rain mesh.
#[derive(Clone, Copy, Debug)]
pub struct RainParticle {
    offset_x: f32,
    offset_z: f32,
    speed: f32,
    seed: u32,
}

#[derive(Default)]
struct RainEntities(Vec<Entity>);

/// Measurement of synchronous rain-particle batch creation.
///
/// The timer is sampled only when the system actually creates particles, not during steady-state
/// movement/recycling, so it does not add a per-frame clock read to normal weather updates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RainDiagnostics {
    /// Number of particle-creation batches observed since the plugin was installed.
    pub particle_batches_created: u64,
    /// Number of entities allocated by the most recently created batch.
    pub last_batch_particle_count: usize,
    /// Wall-clock CPU time spent creating the most recent particle batch, in nanoseconds.
    pub last_batch_creation_time_ns: u64,
}

fn next_unit(seed: &mut u32) -> f32 {
    *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    (*seed >> 8) as f32 / 16_777_216.0
}

fn between(seed: &mut u32, low: f32, high: f32) -> f32 {
    let (low, high) = if low <= high {
        (low, high)
    } else {
        (high, low)
    };
    low + next_unit(seed) * (high - low)
}

fn spawn_particle(
    world: &mut World,
    center: Vec3,
    settings: RainSettings,
    seed: &mut u32,
    index: usize,
) -> Entity {
    let radius = settings.radius.max(1.0);
    let offset_x = next_unit(seed) * radius * 2.0 - radius;
    let offset_z = next_unit(seed) * radius * 2.0 - radius;
    let height = 5.0 + next_unit(seed) * 17.0;
    let speed = between(seed, settings.minimum_speed, settings.maximum_speed);
    let particle = RainParticle {
        offset_x,
        offset_z,
        speed,
        seed: *seed ^ (index as u32).wrapping_mul(0x9E37_79B9),
    };
    world.spawn((
        particle,
        Transform3D {
            translation: Vec3::new(center.x + offset_x, center.y + height, center.z + offset_z),
            rotation: Quat::from_rotation_z(0.10),
            scale: Vec3::new(0.018, DROP_STREAK_LENGTH, 0.018),
        },
        MeshInstance {
            mesh: BuiltinMesh::Cube,
            color: settings.color,
        },
    ))
}

/// Keep rain around the active camera and recycle entities rather than allocating per frame.
pub fn rain_system(world: &mut World) {
    let settings = *world.resource::<RainSettings>();
    let target_count = settings.active_particles();

    if target_count == 0 {
        world.resource_scope::<RainEntities, _>(|world, entities| {
            for entity in entities.0.drain(..) {
                let _ = world.despawn(entity);
            }
        });
        return;
    }

    let Some(camera_position) = world
        .query::<(&Transform3D, &Camera3D)>()
        .iter()
        .next()
        .map(|(_, (transform, _))| transform.translation)
    else {
        return;
    };

    let (created_count, creation_time_ns) =
        world.resource_scope::<RainEntities, _>(|world, entities| {
            while entities.0.len() > target_count {
                if let Some(entity) = entities.0.pop() {
                    let _ = world.despawn(entity);
                }
            }
            let created_count = target_count.saturating_sub(entities.0.len());
            if created_count == 0 {
                return (0, 0);
            }

            let started = Instant::now();
            let mut seed = DEFAULT_RAIN_SEED.wrapping_add(entities.0.len() as u32);
            while entities.0.len() < target_count {
                let entity = spawn_particle(
                    world,
                    camera_position,
                    settings,
                    &mut seed,
                    entities.0.len(),
                );
                entities.0.push(entity);
            }
            let elapsed_ns = started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
            (created_count, elapsed_ns)
        });
    if created_count > 0 && world.try_resource::<RainDiagnostics>().is_some() {
        let diagnostics = world.resource_mut::<RainDiagnostics>();
        diagnostics.particle_batches_created =
            diagnostics.particle_batches_created.saturating_add(1);
        diagnostics.last_batch_particle_count = created_count;
        diagnostics.last_batch_creation_time_ns = creation_time_ns;
    }

    let dt = world
        .resource::<Time>()
        .delta
        .as_secs_f32()
        .clamp(0.0, 0.05);
    let radius = settings.radius.max(1.0);
    for (_, (rain, transform)) in world
        .query_mut::<(&mut RainParticle, &mut Transform3D)>()
        .into_iter()
    {
        rain.offset_x += settings.wind.x * dt;
        rain.offset_z += settings.wind.z * dt;
        if rain.offset_x > radius {
            rain.offset_x -= radius * 2.0;
        }
        if rain.offset_x < -radius {
            rain.offset_x += radius * 2.0;
        }
        if rain.offset_z > radius {
            rain.offset_z -= radius * 2.0;
        }
        if rain.offset_z < -radius {
            rain.offset_z += radius * 2.0;
        }
        transform.translation.x = camera_position.x + rain.offset_x;
        transform.translation.z = camera_position.z + rain.offset_z;
        transform.translation.y -= rain.speed * dt;
        if transform.translation.y < camera_position.y - settings.ground_offset.max(0.0) {
            transform.translation.y = camera_position.y + 7.0 + next_unit(&mut rain.seed) * 16.0;
            rain.offset_x = next_unit(&mut rain.seed) * radius * 2.0 - radius;
            rain.offset_z = next_unit(&mut rain.seed) * radius * 2.0 - radius;
            rain.speed = between(
                &mut rain.seed,
                settings.minimum_speed,
                settings.maximum_speed,
            );
        }
    }
}

/// Registers engine-owned rain settings and the particle recycling system.
pub struct WeatherPlugin;
impl Plugin for WeatherPlugin {
    fn name(&self) -> &'static str {
        "WeatherPlugin"
    }
    fn build(&self, app: &mut App) {
        app.insert_resource(RainSettings::default())
            .insert_resource(RainEntities::default())
            .insert_resource(RainDiagnostics::default())
            .add_system(Stage::Update, rain_system);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        next_unit, rain_system, RainDiagnostics, RainEntities, RainSettings, DROP_STREAK_LENGTH,
    };
    use glam::Vec3;
    use ribbon_app::{Time, World};
    use ribbon_render_contracts::{Camera3D, Transform3D};

    #[test]
    fn rain_settings_keep_the_existing_athar_defaults() {
        let settings = RainSettings::default();
        assert!(!settings.enabled);
        assert_eq!(settings.particle_count, 220);
        assert_eq!(settings.radius, 21.0);
        assert_eq!(settings.minimum_speed, 11.0);
        assert_eq!(settings.maximum_speed, 20.0);
        assert_eq!(settings.color, [0.47, 0.56, 0.67]);
        assert_eq!(DROP_STREAK_LENGTH, 0.42);
        assert_eq!(settings.active_particles(), 0);
    }

    #[test]
    fn rain_particle_creation_batch_is_measured_once_and_then_reuses_entities() {
        let mut world = World::new();
        world.insert_resource(RainSettings {
            enabled: true,
            particle_count: 3,
            ..RainSettings::default()
        });
        world.insert_resource(RainEntities::default());
        world.insert_resource(RainDiagnostics::default());
        world.insert_resource(Time::default());
        world.spawn((
            Camera3D::default(),
            Transform3D::from_translation(Vec3::new(1.0, 2.0, 3.0)),
        ));

        rain_system(&mut world);
        let first = *world.resource::<RainDiagnostics>();
        assert_eq!(first.particle_batches_created, 1);
        assert_eq!(first.last_batch_particle_count, 3);
        assert!(first.last_batch_creation_time_ns > 0);
        assert_eq!(world.query::<&super::RainParticle>().iter().count(), 3);

        rain_system(&mut world);
        let second = world.resource::<RainDiagnostics>();
        assert_eq!(second.particle_batches_created, 1);
        assert_eq!(second.last_batch_particle_count, 3);
    }

    #[test]
    fn random_unit_values_are_bounded_and_repeatable() {
        let mut a = 12;
        let mut b = 12;
        for _ in 0..1000 {
            let left = next_unit(&mut a);
            let right = next_unit(&mut b);
            assert_eq!(left, right);
            assert!((0.0..1.0).contains(&left));
        }
    }
}
