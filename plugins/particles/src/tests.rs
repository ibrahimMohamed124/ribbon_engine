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
fn curves_and_gradients_clamp_times_and_bound_their_stop_count() {
    let mut curve = Curve {
        points: vec![
            (-1.0, 0.0),
            (0.1, 0.1),
            (0.2, 0.2),
            (0.3, 0.3),
            (0.4, 0.4),
            (0.5, 0.5),
            (0.6, 0.6),
            (0.7, 0.7),
            (2.0, 2.0),
        ],
    };
    assert!(curve.validate().is_ok());
    assert_eq!(curve.points.len(), 8);
    assert_eq!(curve.points.first().map(|point| point.0), Some(0.0));
    assert_eq!(curve.points.last().map(|point| point.0), Some(0.7));

    let mut gradient = Gradient {
        stops: (0..10)
            .map(|index| (index as f32 / 4.0 - 0.5, [index as f32; 4]))
            .collect(),
    };
    assert!(gradient.validate().is_ok());
    assert_eq!(gradient.stops.len(), 8);
    assert!(gradient
        .stops
        .iter()
        .all(|(time, _)| (0.0..=1.0).contains(time)));
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

#[test]
fn local_space_world_acceleration_remains_world_aligned() {
    use glam::Quat;
    let mut app = App::new();
    app.insert_resource(Time {
        delta: Duration::from_millis(100),
        ..Time::default()
    });
    app.insert_resource(ParticleRenderData::default());
    app.insert_resource(ParticleBudget {
        max_total_particles: 100,
    });
    app.insert_resource(ParticleDiagnostics::default());
    let emitter = ParticleEmitter {
        enabled: true,
        space: SimulationSpace::Local,
        rate_per_second: 10.0,
        lifetime: (5.0, 5.0),
        start_speed: (0.0, 0.0),
        gravity: Vec3::new(0.0, -10.0, 0.0),
        gravity_scale: 1.0,
        ..ParticleEmitter::default()
    };
    app.world.spawn((
        emitter,
        Transform3D {
            rotation: Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            ..Transform3D::default()
        },
    ));
    particle_update_system(&mut app.world);
    let p = app.world.resource::<ParticleRenderData>().batches[0].instances[0].position;
    assert!(p[1] < -0.09, "world gravity should remain downward: {p:?}");
    assert!(
        p[0].abs() < 1.0e-4,
        "rotation must not turn gravity sideways: {p:?}"
    );
}

#[test]
fn steady_state_emission_rate_matches_rate_times_mean_lifetime() {
    let mut app = App::new();
    app.insert_resource(Time {
        delta: Duration::from_millis(50),
        ..Time::default()
    });
    app.insert_resource(ParticleRenderData::default());
    app.insert_resource(ParticleBudget {
        max_total_particles: 1000,
    });
    app.insert_resource(ParticleDiagnostics::default());
    app.world.spawn((
        ParticleEmitter {
            rate_per_second: 20.0,
            lifetime: (1.0, 1.0),
            start_speed: (0.0, 0.0),
            gravity_scale: 0.0,
            max_particles: 100,
            ..ParticleEmitter::default()
        },
        Transform3D::default(),
    ));
    for _ in 0..40 {
        particle_update_system(&mut app.world);
    }
    let alive = app.world.resource::<ParticleDiagnostics>().alive_particles as f32;
    let expected = 20.0 * 1.0;
    assert!(
        (alive - expected).abs() <= expected * 0.10,
        "steady state {alive}, expected {expected}"
    );
}

#[test]
fn burst_emission_count_is_exact_and_occurs_once() {
    let mut app = App::new();
    app.insert_resource(Time {
        delta: Duration::from_millis(50),
        ..Time::default()
    });
    app.insert_resource(ParticleRenderData::default());
    app.insert_resource(ParticleBudget {
        max_total_particles: 100,
    });
    app.insert_resource(ParticleDiagnostics::default());
    app.world.spawn((
        ParticleEmitter {
            rate_per_second: 0.0,
            bursts: vec![Burst {
                time: 0.05,
                count: 7,
            }],
            lifetime: (5.0, 5.0),
            ..ParticleEmitter::default()
        },
        Transform3D::default(),
    ));
    particle_update_system(&mut app.world);
    assert_eq!(
        app.world.resource::<ParticleDiagnostics>().alive_particles,
        7
    );
    for _ in 0..4 {
        particle_update_system(&mut app.world);
    }
    assert_eq!(
        app.world.resource::<ParticleDiagnostics>().alive_particles,
        7
    );
}

#[test]
fn per_emitter_particle_cap_is_hard_and_stable() {
    let mut app = App::new();
    app.insert_resource(Time {
        delta: Duration::from_millis(100),
        ..Time::default()
    });
    app.insert_resource(ParticleRenderData::default());
    app.insert_resource(ParticleBudget {
        max_total_particles: 1000,
    });
    app.insert_resource(ParticleDiagnostics::default());
    app.world.spawn((
        ParticleEmitter {
            rate_per_second: 100.0,
            lifetime: (10.0, 10.0),
            max_particles: 4,
            ..ParticleEmitter::default()
        },
        Transform3D::default(),
    ));
    for _ in 0..10 {
        particle_update_system(&mut app.world);
        assert!(app.world.resource::<ParticleDiagnostics>().alive_particles <= 4);
    }
    assert_eq!(
        app.world.resource::<ParticleDiagnostics>().alive_particles,
        4
    );
}

#[test]
fn world_ground_kill_bounce_and_stop_responses_are_applied() {
    fn run(response: GroundResponse) -> Option<[f32; 3]> {
        let mut app = App::new();
        app.insert_resource(Time {
            delta: Duration::from_millis(100),
            ..Time::default()
        });
        app.insert_resource(ParticleRenderData::default());
        app.insert_resource(ParticleBudget {
            max_total_particles: 10,
        });
        app.insert_resource(ParticleDiagnostics::default());
        app.world.spawn((
            ParticleEmitter {
                rate_per_second: 10.0,
                lifetime: (5.0, 5.0),
                start_speed: (3.0, 3.0),
                direction: -Vec3::Y,
                gravity_scale: 0.0,
                ground: Some(GroundPlane { y: 0.0, response }),
                ..ParticleEmitter::default()
            },
            Transform3D::default(),
        ));
        particle_update_system(&mut app.world);
        app.world
            .resource::<ParticleRenderData>()
            .batches
            .first()
            .and_then(|batch| batch.instances.first())
            .map(|instance| instance.velocity)
    }
    assert!(run(GroundResponse::Kill).is_none());
    let bounce = run(GroundResponse::Bounce { restitution: 0.5 }).expect("bounced particle");
    assert!(
        bounce[1] > 0.0,
        "bounce must reverse vertical velocity: {bounce:?}"
    );
    let stop = run(GroundResponse::Stop).expect("stopped particle");
    assert_eq!(stop, [0.0, 0.0, 0.0]);
}

#[test]
fn disabled_emitter_stops_spawning_but_drains_existing_particles() {
    let mut app = App::new();
    app.insert_resource(Time {
        delta: Duration::from_millis(50),
        ..Time::default()
    });
    app.insert_resource(ParticleRenderData::default());
    app.insert_resource(ParticleBudget {
        max_total_particles: 100,
    });
    app.insert_resource(ParticleDiagnostics::default());
    let entity = app.world.spawn((
        ParticleEmitter {
            rate_per_second: 0.0,
            bursts: vec![Burst {
                time: 0.0,
                count: 2,
            }],
            lifetime: (0.2, 0.2),
            ..ParticleEmitter::default()
        },
        Transform3D::default(),
    ));
    particle_update_system(&mut app.world);
    assert_eq!(
        app.world.resource::<ParticleDiagnostics>().alive_particles,
        2
    );
    app.world
        .get_mut::<ParticleEmitter>(entity)
        .expect("emitter")
        .enabled = false;
    for _ in 0..3 {
        particle_update_system(&mut app.world);
    }
    assert_eq!(
        app.world.resource::<ParticleDiagnostics>().alive_particles,
        0
    );
}

#[test]
fn world_space_particles_stay_put_while_local_particles_follow_the_emitter() {
    let mut app = App::new();
    app.insert_resource(Time {
        delta: Duration::from_millis(100),
        ..Time::default()
    });
    app.insert_resource(ParticleRenderData::default());
    app.insert_resource(ParticleBudget {
        max_total_particles: 100,
    });
    app.insert_resource(ParticleDiagnostics::default());
    let world = app.world.spawn((
        ParticleEmitter {
            space: SimulationSpace::World,
            rate_per_second: 10.0,
            lifetime: (5.0, 5.0),
            start_speed: (0.0, 0.0),
            gravity_scale: 0.0,
            ..ParticleEmitter::default()
        },
        Transform3D::default(),
    ));
    let local = app.world.spawn((
        ParticleEmitter {
            space: SimulationSpace::Local,
            rate_per_second: 10.0,
            lifetime: (5.0, 5.0),
            start_speed: (0.0, 0.0),
            gravity_scale: 0.0,
            ..ParticleEmitter::default()
        },
        Transform3D::default(),
    ));
    particle_update_system(&mut app.world);
    app.world
        .get_mut::<Transform3D>(world)
        .expect("world transform")
        .translation
        .x = 10.0;
    app.world
        .get_mut::<Transform3D>(local)
        .expect("local transform")
        .translation
        .x = 10.0;
    particle_update_system(&mut app.world);
    let positions = app
        .world
        .resource::<ParticleRenderData>()
        .batches
        .iter()
        .filter_map(|batch| batch.instances.first().map(|instance| instance.position[0]))
        .collect::<Vec<_>>();
    assert!(
        positions.iter().any(|x| *x < 1.0),
        "world-space particle followed emitter: {positions:?}"
    );
    assert!(
        positions.iter().any(|x| *x > 9.0),
        "local-space particle did not follow emitter: {positions:?}"
    );
}

#[test]
fn prewarm_starts_looping_emitters_with_existing_particles() {
    let mut app = App::new();
    app.insert_resource(Time {
        delta: Duration::from_millis(16),
        ..Time::default()
    });
    app.insert_resource(ParticleRenderData::default());
    app.insert_resource(ParticleBudget {
        max_total_particles: 1000,
    });
    app.insert_resource(ParticleDiagnostics::default());
    app.world.spawn((
        ParticleEmitter {
            prewarm: true,
            looping: true,
            rate_per_second: 30.0,
            lifetime: (0.5, 0.5),
            ..ParticleEmitter::default()
        },
        Transform3D::default(),
    ));
    particle_update_system(&mut app.world);
    assert!(app.world.resource::<ParticleDiagnostics>().alive_particles > 0);
}

#[test]
fn removing_emitter_component_releases_its_runtime_state() {
    let mut app = App::new();
    app.insert_resource(Time {
        delta: Duration::from_millis(100),
        ..Time::default()
    });
    app.insert_resource(ParticleRenderData::default());
    app.insert_resource(ParticleBudget {
        max_total_particles: 100,
    });
    app.insert_resource(ParticleDiagnostics::default());
    let entity = app
        .world
        .spawn((ParticleEmitter::default(), Transform3D::default()));
    particle_update_system(&mut app.world);
    assert!(app.world.get::<ParticleEmitterState>(entity).is_some());
    let _ = app.world.remove::<ParticleEmitter>(entity);
    particle_update_system(&mut app.world);
    assert!(app.world.get::<ParticleEmitterState>(entity).is_none());
}

#[test]
fn non_looping_emitter_uses_only_time_before_its_duration() {
    let mut app = App::new();
    app.insert_resource(Time {
        delta: Duration::from_millis(100),
        ..Time::default()
    });
    app.insert_resource(ParticleRenderData::default());
    app.insert_resource(ParticleBudget {
        max_total_particles: 100,
    });
    app.insert_resource(ParticleDiagnostics::default());
    app.world.spawn((
        ParticleEmitter {
            looping: false,
            duration: 0.15,
            rate_per_second: 100.0,
            lifetime: (5.0, 5.0),
            start_speed: (0.0, 0.0),
            ..ParticleEmitter::default()
        },
        Transform3D::default(),
    ));
    particle_update_system(&mut app.world);
    particle_update_system(&mut app.world);
    assert_eq!(
        app.world.resource::<ParticleDiagnostics>().alive_particles,
        15
    );
    particle_update_system(&mut app.world);
    assert_eq!(
        app.world.resource::<ParticleDiagnostics>().alive_particles,
        15
    );
}

#[test]
fn non_looping_emitter_does_not_fire_bursts_after_duration() {
    let mut app = App::new();
    app.insert_resource(Time {
        delta: Duration::from_millis(100),
        ..Time::default()
    });
    app.insert_resource(ParticleRenderData::default());
    app.insert_resource(ParticleBudget {
        max_total_particles: 100,
    });
    app.insert_resource(ParticleDiagnostics::default());
    app.world.spawn((
        ParticleEmitter {
            looping: false,
            duration: 0.15,
            rate_per_second: 0.0,
            bursts: vec![Burst {
                time: 0.2,
                count: 7,
            }],
            lifetime: (5.0, 5.0),
            ..ParticleEmitter::default()
        },
        Transform3D::default(),
    ));
    particle_update_system(&mut app.world);
    particle_update_system(&mut app.world);
    assert_eq!(
        app.world.resource::<ParticleDiagnostics>().alive_particles,
        0
    );
}

#[test]
fn looping_emitter_processes_each_cycle_when_a_frame_crosses_multiple_boundaries() {
    let mut app = App::new();
    app.insert_resource(Time {
        delta: Duration::from_millis(100),
        ..Time::default()
    });
    app.insert_resource(ParticleRenderData::default());
    app.insert_resource(ParticleBudget {
        max_total_particles: 100,
    });
    app.insert_resource(ParticleDiagnostics::default());
    app.world.spawn((
        ParticleEmitter {
            looping: true,
            duration: 0.05,
            rate_per_second: 0.0,
            bursts: vec![Burst {
                time: 0.0,
                count: 2,
            }],
            lifetime: (5.0, 5.0),
            ..ParticleEmitter::default()
        },
        Transform3D::default(),
    ));
    particle_update_system(&mut app.world);
    assert_eq!(
        app.world.resource::<ParticleDiagnostics>().alive_particles,
        4
    );
}

#[test]
fn non_finite_emitter_is_skipped_until_its_generation_changes() {
    let mut app = App::new();
    app.insert_resource(Time {
        delta: Duration::from_secs_f32(1.0 / 30.0),
        ..Time::default()
    });
    app.insert_resource(ParticleRenderData::default());
    app.insert_resource(ParticleBudget {
        max_total_particles: 100,
    });
    app.insert_resource(ParticleDiagnostics::default());
    let emitter = ParticleEmitter {
        rate_per_second: f32::NAN,
        ..ParticleEmitter::default()
    };
    let entity = app.world.spawn((emitter, Transform3D::default()));
    particle_update_system(&mut app.world);
    assert!(app.world.get::<ParticleEmitterState>(entity).is_some());
    assert_eq!(
        app.world.resource::<ParticleDiagnostics>().alive_particles,
        0
    );
    particle_update_system(&mut app.world);
    assert_eq!(
        app.world.resource::<ParticleDiagnostics>().alive_particles,
        0
    );
    {
        let mut emitter = app
            .world
            .get_mut::<ParticleEmitter>(entity)
            .expect("emitter still exists");
        emitter.rate_per_second = 30.0;
        emitter.seed = emitter.seed.wrapping_add(1);
    }
    particle_update_system(&mut app.world);
    assert_eq!(
        app.world
            .resource::<ParticleDiagnostics>()
            .spawned_this_frame,
        1
    );
}
