use glam::Vec3;

/// World-level settings for the Rapier-backed simulation.
///
/// One Ribbon world unit is one meter. The application owns the fixed-step accumulator;
/// `PhysicsConfig` intentionally does not contain a second timestep or accumulator.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PhysicsConfig {
    /// Acceleration applied to dynamic bodies, in world units per second squared.
    pub gravity: Vec3,
    /// CCD enabled on bodies unless an individual [`crate::RigidBody`] overrides it.
    pub default_ccd_enabled: bool,
    /// Constraint-solver iterations per step. Accepted range is 1 through 64.
    pub solver_iterations: usize,
}

impl Default for PhysicsConfig {
    fn default() -> Self {
        Self {
            gravity: Vec3::new(0.0, -9.81, 0.0),
            default_ccd_enabled: false,
            solver_iterations: 4,
        }
    }
}

impl PhysicsConfig {
    /// Validate configuration values before they reach Rapier.
    pub fn validate(&self) -> Result<(), &'static str> {
        if !self.gravity.is_finite() {
            return Err("physics gravity must contain only finite values");
        }
        if !(1..=64).contains(&self.solver_iterations) {
            return Err("physics solver iterations must be between 1 and 64");
        }
        Ok(())
    }
}
