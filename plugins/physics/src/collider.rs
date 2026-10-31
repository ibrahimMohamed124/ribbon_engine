use glam::Vec3;
use rapier3d::prelude::{ColliderBuilder, Group, InteractionGroups};

/// Collision primitive supported by the initial 3D physics integration.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ColliderShape {
    /// Sphere centered on the entity origin.
    Ball { radius: f32 },
    /// Axis-aligned box in body-local coordinates. Values are half-extents, not full size.
    Cuboid { half_extents: Vec3 },
    /// Capsule along local Y. `half_height` is the half-length of the cylindrical segment,
    /// excluding the hemispherical caps.
    CapsuleY { half_height: f32, radius: f32 },
    /// Cylinder along local Y. `half_height` is half of its total height.
    CylinderY { half_height: f32, radius: f32 },
}

/// Collider material, shape, and trigger configuration for a Ribbon entity.
///
/// The first implementation supports one collider per entity. Use multiple entities with
/// independent transforms if a compound shape is needed. Collider dimensions are in meters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Collider {
    pub shape: ColliderShape,
    pub friction: f32,
    pub restitution: f32,
    /// Rapier density in kilograms per cubic meter. Ignored by fixed bodies.
    pub density: f32,
    /// Sensor colliders report overlaps but do not generate contact response.
    pub sensor: bool,
}

impl Default for Collider {
    fn default() -> Self {
        Self::ball(0.5)
    }
}

impl Collider {
    pub const fn ball(radius: f32) -> Self {
        Self {
            shape: ColliderShape::Ball { radius },
            friction: 0.7,
            restitution: 0.0,
            density: 1.0,
            sensor: false,
        }
    }

    pub const fn cuboid(half_extents: Vec3) -> Self {
        Self {
            shape: ColliderShape::Cuboid { half_extents },
            friction: 0.7,
            restitution: 0.0,
            density: 1.0,
            sensor: false,
        }
    }

    pub const fn capsule_y(half_height: f32, radius: f32) -> Self {
        Self {
            shape: ColliderShape::CapsuleY {
                half_height,
                radius,
            },
            friction: 0.7,
            restitution: 0.0,
            density: 1.0,
            sensor: false,
        }
    }

    pub const fn cylinder_y(half_height: f32, radius: f32) -> Self {
        Self {
            shape: ColliderShape::CylinderY {
                half_height,
                radius,
            },
            friction: 0.7,
            restitution: 0.0,
            density: 1.0,
            sensor: false,
        }
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        let positive = |value: f32| value.is_finite() && value > 0.0;
        let shape_valid = match self.shape {
            ColliderShape::Ball { radius } => positive(radius),
            ColliderShape::Cuboid { half_extents } => {
                half_extents.is_finite()
                    && half_extents.x > 0.0
                    && half_extents.y > 0.0
                    && half_extents.z > 0.0
            }
            ColliderShape::CapsuleY {
                half_height,
                radius,
            } => half_height.is_finite() && half_height >= 0.0 && positive(radius),
            ColliderShape::CylinderY {
                half_height,
                radius,
            } => positive(half_height) && positive(radius),
        };
        if !shape_valid {
            return Err("invalid collider dimensions; capsule half-height may be zero");
        }
        if !self.friction.is_finite() || self.friction < 0.0 {
            return Err("collider friction must be finite and non-negative");
        }
        if !self.restitution.is_finite() || !(0.0..=1.0).contains(&self.restitution) {
            return Err("collider restitution must be between 0 and 1");
        }
        if !self.density.is_finite() || self.density <= 0.0 {
            return Err("collider density must be finite and positive");
        }
        Ok(())
    }

    pub fn validate_for_scale(self, scale: Vec3) -> Result<(), &'static str> {
        self.validate()?;
        validate_scale(scale)?;
        let positive = |value: f32| value.is_finite() && value > 0.0;
        match self.shape {
            ColliderShape::Ball { radius } => {
                if !is_uniform_scale(scale) {
                    return Err("ball colliders require uniform world scale");
                }
                if !positive(radius * scale.x) {
                    return Err("scaled ball radius must be finite and positive");
                }
            }
            ColliderShape::Cuboid { half_extents } => {
                let extents = half_extents * scale;
                if !extents.is_finite() || extents.min_element() <= 0.0 {
                    return Err("scaled cuboid half-extents must be finite and positive");
                }
            }
            ColliderShape::CapsuleY {
                half_height,
                radius,
            } => {
                if (scale.x - scale.z).abs() > 1.0e-4 {
                    return Err("vertical capsule colliders require equal X and Z scale");
                }
                if !positive(half_height * scale.y) && half_height != 0.0 {
                    return Err("scaled capsule half-height must be finite and non-negative");
                }
                if !positive(radius * scale.x) {
                    return Err("scaled capsule radius must be finite and positive");
                }
            }
            ColliderShape::CylinderY {
                half_height,
                radius,
            } => {
                if (scale.x - scale.z).abs() > 1.0e-4 {
                    return Err("vertical cylinder colliders require equal X and Z scale");
                }
                if !positive(half_height * scale.y) || !positive(radius * scale.x) {
                    return Err("scaled cylinder dimensions must be finite and positive");
                }
            }
        }
        Ok(())
    }

    pub(crate) fn to_builder(
        self,
        scale: Vec3,
        layers: CollisionLayers,
    ) -> Result<ColliderBuilder, &'static str> {
        self.validate_for_scale(scale)?;
        let shape = match self.shape {
            ColliderShape::Ball { radius } => ColliderBuilder::ball(radius * scale.x),
            ColliderShape::Cuboid { half_extents } => {
                let extents = half_extents * scale;
                ColliderBuilder::cuboid(extents.x, extents.y, extents.z)
            }
            ColliderShape::CapsuleY {
                half_height,
                radius,
            } => ColliderBuilder::capsule_y(half_height * scale.y, radius * scale.x),
            ColliderShape::CylinderY {
                half_height,
                radius,
            } => ColliderBuilder::cylinder(half_height * scale.y, radius * scale.x),
        };
        Ok(shape
            .friction(self.friction)
            .restitution(self.restitution)
            .density(self.density)
            .sensor(self.sensor)
            .collision_groups(layers.rapier_groups())
            .active_events(rapier3d::prelude::ActiveEvents::COLLISION_EVENTS))
    }
}

/// Ribbon-level collision membership and filter bits.
///
/// A pair interacts when each collider's memberships intersect the other collider's filter.
/// All bits are enabled by default, so existing callers need not configure layers to collide.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CollisionLayers {
    pub memberships: u32,
    pub filters: u32,
}

impl Default for CollisionLayers {
    fn default() -> Self {
        Self {
            memberships: u32::MAX,
            filters: u32::MAX,
        }
    }
}

impl CollisionLayers {
    pub const fn new(memberships: u32, filters: u32) -> Self {
        Self {
            memberships,
            filters,
        }
    }

    pub(crate) fn rapier_groups(self) -> InteractionGroups {
        InteractionGroups::new(
            Group::from_bits_truncate(self.memberships),
            Group::from_bits_truncate(self.filters),
            rapier3d::prelude::InteractionTestMode::And,
        )
    }
}

pub(crate) fn validate_scale(scale: Vec3) -> Result<(), &'static str> {
    if !scale.is_finite() || scale.min_element() <= 0.0 {
        return Err("physics transforms require finite, strictly positive scale");
    }
    Ok(())
}

fn is_uniform_scale(scale: Vec3) -> bool {
    (scale.x - scale.y).abs() <= 1.0e-4 && (scale.x - scale.z).abs() <= 1.0e-4
}
