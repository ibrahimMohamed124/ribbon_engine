use ribbon_ecs::Entity;

/// Two non-sensor colliders began touching/overlapping, or a sensor overlap began.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CollisionStarted {
    pub entity_a: Entity,
    pub entity_b: Entity,
}

/// Two colliders stopped touching/overlapping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CollisionStopped {
    pub entity_a: Entity,
    pub entity_b: Entity,
}

/// A sensor began overlapping another collider. This is also reported as `CollisionStarted`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SensorEntered {
    pub sensor: Entity,
    pub other: Entity,
}

/// A sensor stopped overlapping another collider. This is also reported as `CollisionStopped`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SensorExited {
    pub sensor: Entity,
    pub other: Entity,
}
