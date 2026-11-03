//! Documented particle emitter presets.
use crate::{EmitterShape, GroundPlane, GroundResponse, ParticleEmitter, SimulationSpace};
use glam::Vec3;
use ribbon_render_contracts::{ParticleBillboard, ParticleBlendMode, ParticleShape};
impl ParticleEmitter {
    pub fn rain() -> Self {
        Self {
            shape: EmitterShape::Box {
                half_extents: Vec3::new(21.0, 8.5, 21.0),
            },
            space: SimulationSpace::World,
            rate_per_second: 147.0,
            lifetime: (1.2, 1.8),
            direction: Vec3::NEG_Y,
            start_speed: (11.0, 20.0),
            direction_spread_deg: 0.0,
            start_size: (0.018, 0.018),
            aspect: 23.3,
            billboard: ParticleBillboard::VelocityStretched { stretch: 0.0 },
            shape_sprite: ParticleShape::Streak,
            blend: ParticleBlendMode::Alpha,
            start_color_a: [0.47, 0.56, 0.67, 0.8],
            start_color_b: [0.47, 0.56, 0.67, 0.8],
            gravity_scale: 0.0,
            wind: Vec3::ZERO,
            max_particles: 512,
            emissive: 0.0,
            ground: None,
            seed: 0x51A7_2026,
            ..Self::default()
        }
    }
    pub fn snow() -> Self {
        Self {
            shape: EmitterShape::Sphere {
                radius: 8.0,
                surface_only: false,
            },
            rate_per_second: 18.0,
            lifetime: (5.0, 9.0),
            start_speed: (0.2, 0.8),
            direction: Vec3::NEG_Y,
            direction_spread_deg: 20.0,
            start_size: (0.035, 0.06),
            aspect: 1.0,
            gravity_scale: 0.05,
            wind: Vec3::new(0.1, 0.0, 0.0),
            max_particles: 512,
            ..Self::default()
        }
    }
    pub fn fire() -> Self {
        Self {
            shape: EmitterShape::Cone {
                angle_deg: 18.0,
                radius: 0.15,
            },
            rate_per_second: 48.0,
            lifetime: (0.4, 0.9),
            start_speed: (0.5, 1.5),
            direction: Vec3::Y,
            start_size: (0.08, 0.2),
            start_color_a: [1.0, 0.22, 0.02, 0.95],
            start_color_b: [1.0, 0.8, 0.1, 0.8],
            color_over_life: crate::Gradient {
                stops: vec![(0.0, [1.0; 4]), (1.0, [0.3, 0.05, 0.0, 0.0])],
            },
            blend: ParticleBlendMode::Additive,
            emissive: 1.0,
            max_particles: 256,
            ..Self::default()
        }
    }
    pub fn smoke() -> Self {
        Self {
            shape: EmitterShape::Sphere {
                radius: 0.08,
                surface_only: false,
            },
            rate_per_second: 8.0,
            lifetime: (2.0, 4.0),
            start_speed: (0.2, 0.6),
            direction: Vec3::Y,
            start_size: (0.15, 0.35),
            aspect: 1.2,
            start_color_a: [0.25, 0.25, 0.25, 0.55],
            start_color_b: [0.5, 0.5, 0.5, 0.4],
            size_over_life: crate::Curve {
                points: vec![(0.0, 0.5), (1.0, 2.0)],
            },
            max_particles: 256,
            ..Self::default()
        }
    }
    pub fn sparks() -> Self {
        Self {
            shape: EmitterShape::Point,
            rate_per_second: 24.0,
            lifetime: (0.2, 0.65),
            start_speed: (2.0, 6.0),
            direction: Vec3::Y,
            direction_spread_deg: 65.0,
            start_size: (0.025, 0.04),
            aspect: 5.0,
            billboard: ParticleBillboard::VelocityStretched { stretch: 0.03 },
            start_color_a: [1.0, 0.45, 0.1, 1.0],
            start_color_b: [1.0, 0.9, 0.25, 1.0],
            blend: ParticleBlendMode::Additive,
            emissive: 0.6,
            max_particles: 192,
            gravity_scale: 1.0,
            ground: Some(GroundPlane {
                y: 0.0,
                response: GroundResponse::Bounce { restitution: 0.25 },
            }),
            ..Self::default()
        }
    }
    pub fn dust() -> Self {
        Self {
            shape: EmitterShape::Sphere {
                radius: 0.5,
                surface_only: false,
            },
            rate_per_second: 12.0,
            lifetime: (0.6, 1.4),
            start_speed: (0.1, 0.6),
            direction: Vec3::Y,
            direction_spread_deg: 50.0,
            start_size: (0.04, 0.1),
            start_color_a: [0.55, 0.43, 0.28, 0.35],
            start_color_b: [0.55, 0.43, 0.28, 0.2],
            max_particles: 128,
            ..Self::default()
        }
    }
}
