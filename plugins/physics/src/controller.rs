use glam::Vec3;

/// Settings for a Rapier kinematic character controller.
///
/// Attach this to an entity that also has `RigidBody::kinematic_position_based()` and a
/// vertical capsule collider. Submit per-fixed-step intent through `CharacterMovementIntent`;
/// the physics plugin consumes that intent before the Rapier simulation step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CharacterController {
    /// Small separation distance used to avoid numerical contact jitter, in meters.
    pub offset: f32,
    /// Slide along walls and sloped surfaces instead of stopping at first contact.
    pub slide: bool,
    /// Maximum slope angle the character can climb, in radians.
    pub max_slope_climb_angle: f32,
    /// Minimum slope angle at which the character slides down automatically, in radians.
    pub min_slope_slide_angle: f32,
    /// Optional snap-to-ground distance, in meters.
    pub snap_to_ground: Option<f32>,
}

impl Default for CharacterController {
    fn default() -> Self {
        Self {
            offset: 0.01,
            slide: true,
            max_slope_climb_angle: 45.0_f32.to_radians(),
            min_slope_slide_angle: 30.0_f32.to_radians(),
            snap_to_ground: Some(0.2),
        }
    }
}

impl CharacterController {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !self.offset.is_finite() || self.offset <= 0.0 {
            return Err("character-controller offset must be finite and positive");
        }
        if !self.max_slope_climb_angle.is_finite()
            || !(0.0..=std::f32::consts::FRAC_PI_2).contains(&self.max_slope_climb_angle)
        {
            return Err("character-controller max slope angle must be between 0 and pi/2");
        }
        if !self.min_slope_slide_angle.is_finite()
            || !(0.0..=std::f32::consts::FRAC_PI_2).contains(&self.min_slope_slide_angle)
        {
            return Err("character-controller min slope angle must be between 0 and pi/2");
        }
        if self
            .snap_to_ground
            .is_some_and(|distance| !distance.is_finite() || distance <= 0.0)
        {
            return Err("character-controller snap distance must be finite and positive");
        }
        Ok(())
    }
}

/// Desired displacement consumed once by the next physics fixed step.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CharacterMovementIntent {
    pub desired_displacement: Vec3,
}

/// Result of the most recently processed character movement intent.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CharacterMovementResult {
    pub requested_displacement: Vec3,
    pub applied_displacement: Vec3,
    pub grounded: bool,
    pub obstructed: bool,
    pub contact_count: usize,
}

impl CharacterMovementResult {
    pub(crate) fn new(
        requested_displacement: Vec3,
        applied_displacement: Vec3,
        grounded: bool,
        contact_count: usize,
    ) -> Self {
        Self {
            requested_displacement,
            applied_displacement,
            grounded,
            obstructed: (requested_displacement - applied_displacement).length_squared() > 1.0e-6,
            contact_count,
        }
    }
}
