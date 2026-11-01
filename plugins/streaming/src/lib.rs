//! Large-world streaming coordination for Ribbon.
//!
//! `AssetServer` remains the CPU asset owner, `ModelWarmup`/render3d remain the GPU model owner,
//! and `PhysicsPlugin` remains the collider owner. This plugin coordinates their readiness and
//! region lifetime without introducing a second loader, renderer, or physics world.

mod asset_adapters;
mod coordinator;
mod partition;
mod partition_asset;
mod scene_doc;

pub use asset_adapters::{StreamingAssetAdapters, StreamingDependencyState};
pub use coordinator::{
    RegionActivationBreakdown, RegionCollider, RegionDefinition, RegionId, RegionLifecycleTiming,
    RegionModel, RegionPointLight, RegionPolicy, RegionSnapshot, RegionState, SceneDefinition,
    SceneId, SceneSnapshot, SceneState, SceneTransitionParticipant, StreamDiagnostics, StreamEvent,
    StreamObserver, StreamingConfig, StreamingCoordinator, StreamingMemoryBudget, StreamingPlugin,
    StreamingSnapshot,
};
pub use partition::{RegionBounds, WorldPartitionIndex};
pub use partition_asset::{
    WorldModelPlacementAsset, WorldPartitionAsset, WorldPartitionLoader, WorldRegionAsset,
    WorldRegionPolicyAsset,
};

pub use scene_doc::SceneDocLoader;
