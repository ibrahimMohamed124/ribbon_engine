//! Renderer-neutral particle batch contracts.

/// Blend behavior for particle instances.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParticleBlendMode {
    Alpha,
    Additive,
}
/// Procedural shape used by the particle shader.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParticleShape {
    SoftDisc,
    Streak,
    Square,
}
/// Camera-facing or velocity-aligned quad behavior.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ParticleBillboard {
    Camera,
    VelocityStretched { stretch: f32 },
}
/// One plain-data particle record in world space.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ParticleInstance {
    pub position: [f32; 3],
    pub rotation: f32,
    pub velocity: [f32; 3],
    pub emissive: f32,
    pub size: [f32; 2],
    pub color: [f32; 4],
}
/// A renderer-compatible batch of particles.
#[derive(Clone, Debug, PartialEq)]
pub struct ParticleBatch {
    pub blend: ParticleBlendMode,
    pub shape: ParticleShape,
    pub billboard: ParticleBillboard,
    pub sort_back_to_front: bool,
    pub instances: Vec<ParticleInstance>,
}
/// Frame-local renderer-neutral particle data.
#[derive(Clone, Debug, Default)]
pub struct ParticleRenderData {
    pub batches: Vec<ParticleBatch>,
}

#[cfg(test)]
mod tests {
    use super::ParticleInstance;
    #[test]
    fn instance_is_plain_data_layout() {
        assert_eq!(std::mem::size_of::<ParticleInstance>(), 56);
        assert_eq!(std::mem::align_of::<ParticleInstance>(), 4);
    }
}
