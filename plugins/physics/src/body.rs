use glam::Vec3;

/// Simulation mode of a Ribbon rigid body.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RigidBodyKind {
    /// Immovable body, commonly used for floors and walls.
    Fixed,
    /// Body whose position and rotation are integrated by the physics solver.
    #[default]
    Dynamic,
    /// Body moved by setting a target pose. Rapier derives its velocity from the target.
    KinematicPositionBased,
    /// Body moved by its configured velocities and not by forces.
    KinematicVelocityBased,
}

/// Desired rigid-body configuration for an entity with a [`crate::Collider`].
///
/// The component describes creation/configuration state, not live solver state. Modify or
/// replace it to reconfigure a body. A missing `RigidBody` next to a `Collider` means a fixed
/// collider. Velocities are expressed in meters and radians per second.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RigidBody {
    pub kind: RigidBodyKind,
    pub linear_velocity: Vec3,
    pub angular_velocity: Vec3,
    pub linear_damping: f32,
    pub angular_damping: f32,
    /// Enable/disable CCD for this body. `None` uses the world default.
    pub ccd_enabled: Option<bool>,
}

impl Default for RigidBody {
    fn default() -> Self {
        Self::dynamic()
    }
}

impl RigidBody {
    pub const fn fixed() -> Self {
        Self {
            kind: RigidBodyKind::Fixed,
            linear_velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            linear_damping: 0.0,
            angular_damping: 0.0,
            ccd_enabled: None,
        }
    }

    pub const fn dynamic() -> Self {
        Self {
            kind: RigidBodyKind::Dynamic,
            linear_velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            linear_damping: 0.0,
            angular_damping: 0.0,
            ccd_enabled: None,
        }
    }

    pub const fn kinematic_position_based() -> Self {
        Self {
            kind: RigidBodyKind::KinematicPositionBased,
            ..Self::fixed()
        }
    }

    pub const fn kinematic_velocity_based() -> Self {
        Self {
            kind: RigidBodyKind::KinematicVelocityBased,
            ..Self::fixed()
        }
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if !self.linear_velocity.is_finite() || !self.angular_velocity.is_finite() {
            return Err("rigid-body velocities must contain only finite values");
        }
        if !self.linear_damping.is_finite() || self.linear_damping < 0.0 {
            return Err("rigid-body linear damping must be finite and non-negative");
        }
        if !self.angular_damping.is_finite() || self.angular_damping < 0.0 {
            return Err("rigid-body angular damping must be finite and non-negative");
        }
        Ok(())
    }
}
