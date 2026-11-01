use glam::Vec3;
use ribbon_ecs::{Entity, World};

use crate::collider::CollisionLayers;
use crate::runtime::PhysicsContext;

/// Input for a closest-hit raycast. Direction is normalized by the query function.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RaycastQuery {
    pub origin: Vec3,
    pub direction: Vec3,
    pub max_distance: f32,
    pub exclude_entity: Option<Entity>,
    pub layers: CollisionLayers,
    pub include_sensors: bool,
}

impl RaycastQuery {
    pub fn new(origin: Vec3, direction: Vec3, max_distance: f32) -> Self {
        Self {
            origin,
            direction,
            max_distance,
            exclude_entity: None,
            layers: CollisionLayers::default(),
            include_sensors: true,
        }
    }
}

/// Ribbon-facing result of a successful raycast.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RaycastHit {
    pub entity: Entity,
    pub position: Vec3,
    pub normal: Vec3,
    /// Distance along the normalized ray in world units (meters by convention).
    pub distance: f32,
}

/// Cast a ray against the current physics scene and return the closest permitted hit.
///
/// Returns `None` for an invalid ray, missing physics plugin, or no intersection.
pub fn raycast(world: &World, query: RaycastQuery) -> Option<RaycastHit> {
    if !query.origin.is_finite()
        || !query.direction.is_finite()
        || !query.max_distance.is_finite()
        || query.max_distance <= 0.0
        || query.direction.length_squared() <= 1.0e-12
    {
        return None;
    }
    world
        .try_resource::<PhysicsContext>()
        .and_then(|context| context.raycast(query))
}
