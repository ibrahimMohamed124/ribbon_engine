//! Large-world streaming coordination for Ribbon.
//!
//! `AssetServer` remains the CPU asset owner, `ModelWarmup`/render3d remain the GPU model owner,
//! and `PhysicsPlugin` remains the collider owner. This plugin coordinates their readiness and
//! region lifetime without introducing a second loader, renderer, or physics world.

mod coordinator;
mod partition;

pub use coordinator::{
    RegionCollider, RegionDefinition, RegionId, RegionModel, RegionPointLight, RegionPolicy,
    RegionSnapshot, RegionState, SceneDefinition, SceneId, SceneSnapshot, SceneState,
    StreamDiagnostics, StreamEvent, StreamObserver, StreamingConfig, StreamingCoordinator,
    StreamingMemoryBudget, StreamingPlugin, StreamingSnapshot,
};
pub use partition::{RegionBounds, WorldPartitionIndex};
