//! Particle emitter component definitions and validation.
use crate::{Curve, Gradient};
use glam::Vec3;
use ribbon_render_contracts::{ParticleBillboard, ParticleBlendMode, ParticleShape};
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EmitterShape {
    Point,
    Sphere { radius: f32, surface_only: bool },
    Box { half_extents: Vec3 },
    Cone { angle_deg: f32, radius: f32 },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SimulationSpace {
    World,
    Local,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GroundResponse {
    Kill,
    Bounce { restitution: f32 },
    Stop,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroundPlane {
    pub y: f32,
    pub response: GroundResponse,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Burst {
    pub time: f32,
    pub count: u32,
}
#[derive(Clone, Debug, PartialEq)]
pub enum ParticleError {
    InvalidRange,
    TooManyParticles,
    NonFinite,
    TooManyStops,
}
impl std::fmt::Display for ParticleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for ParticleError {}
pub struct ParticleEmitter {
    pub enabled: bool,
    pub shape: EmitterShape,
    pub space: SimulationSpace,
    pub rate_per_second: f32,
    pub bursts: Vec<Burst>,
    pub looping: bool,
    pub duration: f32,
    pub prewarm: bool,
    pub max_particles: u32,
    pub lifetime: (f32, f32),
    pub start_speed: (f32, f32),
    pub direction: Vec3,
    pub direction_spread_deg: f32,
    pub start_size: (f32, f32),
    pub aspect: f32,
    pub start_rotation: (f32, f32),
    pub angular_velocity: (f32, f32),
    pub start_color_a: [f32; 4],
    pub start_color_b: [f32; 4],
    pub color_over_life: Gradient,
    pub size_over_life: Curve,
    pub gravity: Vec3,
    pub gravity_scale: f32,
    pub drag: f32,
    pub wind: Vec3,
    pub ground: Option<GroundPlane>,
    pub blend: ParticleBlendMode,
    pub shape_sprite: ParticleShape,
    pub billboard: ParticleBillboard,
    pub emissive: f32,
    pub sort_back_to_front: bool,
    pub seed: u64,
}
impl Default for ParticleEmitter {
    fn default() -> Self {
        Self {
            enabled: true,
            shape: EmitterShape::Point,
            space: SimulationSpace::World,
            rate_per_second: 20.0,
            bursts: vec![],
            looping: true,
            duration: 0.0,
            prewarm: false,
            max_particles: 1000,
            lifetime: (1.0, 2.0),
            start_speed: (1.0, 2.0),
            direction: Vec3::Y,
            direction_spread_deg: 0.0,
            start_size: (0.1, 0.2),
            aspect: 1.0,
            start_rotation: (0.0, 0.0),
            angular_velocity: (0.0, 0.0),
            start_color_a: [1.0; 4],
            start_color_b: [1.0; 4],
            color_over_life: Gradient::constant([1.0; 4]),
            size_over_life: Curve::constant(1.0),
            gravity: Vec3::new(0.0, -9.81, 0.0),
            gravity_scale: 0.0,
            drag: 0.0,
            wind: Vec3::ZERO,
            ground: None,
            blend: ParticleBlendMode::Alpha,
            shape_sprite: ParticleShape::SoftDisc,
            billboard: ParticleBillboard::Camera,
            emissive: 0.0,
            sort_back_to_front: false,
            seed: 1,
        }
    }
}
impl ParticleEmitter {
    /// Restart simulation state on the next update while retaining deterministic seeding.
    pub fn restart(&mut self) {
        self.seed = self.seed.wrapping_add(1);
    }
    pub fn validate(&mut self) -> Result<(), ParticleError> {
        let mut bad = false;
        for (a, b) in [
            &mut self.lifetime,
            &mut self.start_speed,
            &mut self.start_size,
            &mut self.start_rotation,
            &mut self.angular_velocity,
        ] {
            if !a.is_finite() || !b.is_finite() {
                bad = true
            } else if *a > *b {
                std::mem::swap(a, b)
            }
        }
        if !self.direction.is_finite() || !self.gravity.is_finite() || !self.wind.is_finite() {
            bad = true
        }
        if self
            .start_color_a
            .iter()
            .chain(self.start_color_b.iter())
            .any(|v| !v.is_finite())
        {
            bad = true;
        }
        match self.shape {
            EmitterShape::Sphere { radius, .. } | EmitterShape::Cone { radius, .. }
                if !radius.is_finite() =>
            {
                bad = true
            }
            EmitterShape::Cone { angle_deg, .. } if !angle_deg.is_finite() => bad = true,
            EmitterShape::Box { half_extents } if !half_extents.is_finite() => bad = true,
            _ => {}
        }
        if self.bursts.iter().any(|b| !b.time.is_finite()) {
            bad = true;
        }
        if self.ground.is_some_and(|g| !g.y.is_finite()) {
            bad = true;
        }
        for v in [
            self.rate_per_second,
            self.duration,
            self.direction_spread_deg,
            self.aspect,
            self.gravity_scale,
            self.drag,
            self.emissive,
        ] {
            if !v.is_finite() {
                bad = true
            }
        }
        if bad {
            return Err(ParticleError::NonFinite);
        }
        self.max_particles = self.max_particles.clamp(1, 100_000);
        self.bursts
            .iter_mut()
            .for_each(|b| b.time = b.time.max(0.0));
        self.direction = if self.direction.length_squared() > 1e-8 {
            self.direction.normalize()
        } else {
            Vec3::Y
        };
        self.rate_per_second = self.rate_per_second.max(0.0);
        self.lifetime.0 = self.lifetime.0.max(0.001);
        self.start_size.0 = self.start_size.0.max(0.0);
        self.aspect = self.aspect.max(0.001);
        self.drag = self.drag.max(0.0);
        self.emissive = self.emissive.max(0.0);
        self.bursts.truncate(64);
        self.bursts.sort_by(|a, b| a.time.total_cmp(&b.time));
        self.color_over_life.validate()?;
        self.size_over_life.validate()?;
        Ok(())
    }
}
