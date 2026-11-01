//! Renderer-independent physics integration backed by Rapier 3D.
//!
//! Install `ScenePlugin` first, then `PhysicsPlugin`. Add `Transform3D` and `Collider` to a
//! physics entity; add `RigidBody` when it should be dynamic or kinematic. A collider without a
//! `RigidBody` is fixed. The existing AABB movement helpers remain available for compatibility.

pub mod body;
pub mod collider;
pub mod config;
pub mod controller;
pub mod events;
mod physics;
pub mod plugin;
mod prefab;
pub mod queries;
mod runtime;

pub use body::{RigidBody, RigidBodyKind};
pub use collider::{Collider, ColliderShape, CollisionLayers};
pub use config::PhysicsConfig;
pub use controller::{CharacterController, CharacterMovementIntent, CharacterMovementResult};
pub use events::{CollisionStarted, CollisionStopped, SensorEntered, SensorExited};
pub use physics::{slide_capsule_xz, Aabb3};
pub use plugin::PhysicsPlugin;
pub use queries::{raycast, RaycastHit, RaycastQuery};
pub use runtime::{PhysicsDebugCollider, PhysicsDebugData, PhysicsDiagnostic, PhysicsDiagnostics};

#[cfg(test)]
mod tests;
