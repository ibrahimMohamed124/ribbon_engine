//! Deterministic CPU particle simulation with renderer-neutral extraction.

pub mod curves;
pub mod diagnostics;
pub mod emitter;
pub mod plugin;
pub mod presets;
pub mod rng;
pub mod sim;
#[cfg(test)]
mod tests;

pub use curves::{Curve, Gradient};
pub use diagnostics::{ParticleBudget, ParticleDiagnostics};
pub use emitter::{
    Burst, EmitterShape, GroundPlane, GroundResponse, ParticleEmitter, ParticleError,
    SimulationSpace,
};
pub use plugin::ParticlePlugin;
pub use ribbon_render_contracts::{
    ParticleBatch, ParticleBillboard, ParticleBlendMode, ParticleInstance, ParticleRenderData,
    ParticleShape,
};
pub use sim::{particle_update_system, ParticleEmitterState};
