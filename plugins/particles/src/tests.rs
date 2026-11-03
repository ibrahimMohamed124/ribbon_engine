use super::*;
use glam::Vec3;
use ribbon_app::{App, Time};
use ribbon_scene::Transform3D;
use std::time::Duration;

#[test]
fn same_seed_produces_same_render_data() {
    fn run() -> Vec<[f32; 3]> {
        let mut a = App::new();
        a.insert_resource(Time {
            delta: Duration::from_millis(100),
            ..Time::default()
        });
        a.insert_resource(ParticleRenderData::default());
        a.insert_resource(ParticleBudget {
            max_total_particles: 1000,
        });
        a.insert_resource(ParticleDiagnostics::default());
        let e = ParticleEmitter {
            seed: 23,
            rate_per_second: 10.0,
            lifetime: (4.0, 4.0),
            ..ParticleEmitter::default()
        };
        a.world
            .spawn((e, Transform3D::from_translation(Vec3::ZERO)));
        for _ in 0..4 {
            particle_update_system(&mut a.world)
        }
        a.world.resource::<ParticleRenderData>().batches[0]
            .instances
            .iter()
            .map(|i| i.position)
            .collect()
    }
    assert_eq!(run(), run());
}
#[test]
fn curves_and_gradients_interpolate_and_validate_swaps_ranges() {
    let mut c = Curve {
        points: vec![(1.0, 1.0), (0.0, 0.0)],
    };
    c.validate().unwrap();
    assert_eq!(c.eval(0.5), 0.5);
    let g = Gradient {
        stops: vec![(0.0, [0.0; 4]), (1.0, [1.0; 4])],
    };
    assert_eq!(g.eval(0.5), [0.5; 4]);
    let mut e = ParticleEmitter {
        lifetime: (3.0, 1.0),
        ..ParticleEmitter::default()
    };
    assert!(e.validate().is_ok());
    assert_eq!(e.lifetime, (1.0, 3.0));
}
#[test]
fn rain_preset_validates() {
    let mut rain = ParticleEmitter::rain();
    assert!(rain.validate().is_ok());
    assert_eq!(rain.max_particles, 512);
}
#[test]
fn budget_limits_spawn_and_counts_drops() {
    let mut app = App::new();
    app.insert_resource(Time {
        delta: Duration::from_secs(1),
        ..Time::default()
    });
    app.insert_resource(ParticleRenderData::default());
    app.insert_resource(ParticleBudget {
        max_total_particles: 2,
    });
    app.insert_resource(ParticleDiagnostics::default());
    let e = ParticleEmitter {
        rate_per_second: 200.0,
        lifetime: (4.0, 4.0),
        ..ParticleEmitter::default()
    };
    app.world.spawn((e, Transform3D::default()));
    particle_update_system(&mut app.world);
    assert_eq!(
        app.world.resource::<ParticleDiagnostics>().alive_particles,
        2
    );
    assert!(
        app.world
            .resource::<ParticleDiagnostics>()
            .dropped_by_budget
            > 0
    );
}
