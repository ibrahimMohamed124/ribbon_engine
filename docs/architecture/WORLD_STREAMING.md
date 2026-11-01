# World streaming, partitioning, and residency

`ribbon_plugin_streaming` is the orchestration layer for large-world content. It deliberately sits
under `plugins/`, because it coordinates peer plugins (`ribbon_plugin_render3d` and
`ribbon_plugin_physics`). Engine crates do not depend on plugins. Resource ownership remains with
the subsystem that creates the resource:

- `ribbon_assets::AssetServer` owns CPU assets, source IO/decode workers, `AssetRef<T>` leases,
  typed load errors, generations, and unpinned CPU-cache eviction.
- `ribbon_plugin_render3d::ModelWarmup` owns warmup references to GPU model cache entries. The
  renderer processes those requests on the renderer-owning thread and applies its existing
  two-model-per-frame upload cap.
- `ribbon_plugin_physics` owns Rapier bodies/colliders and exposes its reconciled collider snapshot.
- `ribbon_scene` owns local/global transforms and their propagation; the streaming plugin only
  spawns the components consumed by that pipeline.
- `ribbon_plugin_streaming::StreamingCoordinator` owns region metadata, state transitions, strong
  CPU leases, warmup reference counts, and the ECS entities it created for each region.

## Installing the plugin

Install `ScenePlugin`, `AssetPlugin`, `PhysicsPlugin`, `GpuWindowPlugin`, and `Render3dPlugin` first.
Register the appropriate asset loaders and sources with `AssetServer` before requesting a region.
Then install `StreamingPlugin`. A game owns its region descriptions and chooses policy distances and
per-frame work limits.

```rust,no_run
use glam::Vec3;
use ribbon_plugin_streaming::{
    RegionBounds, RegionDefinition, RegionId, RegionPolicy, StreamObserver,
    StreamingConfig, StreamingCoordinator, StreamingPlugin,
};

# fn register(app: &mut ribbon_app::App) -> Result<(), Box<dyn std::error::Error>> {
app.add_plugin(StreamingPlugin::new(StreamingConfig {
    max_new_region_requests_per_update: 3,
    max_asset_requests_per_update: 48,
    max_region_activations_per_update: 1,
    max_entity_spawns_per_update: 128,
    max_active_regions: 32,
    ..StreamingConfig::default()
}))?;
let id = RegionId::new("world.north_forest");
let definition = RegionDefinition::new(
    id.clone(),
    "North forest",
    RegionBounds::new(Vec3::new(-128.0, -16.0, -128.0), Vec3::new(128.0, 64.0, 128.0))
        .map_err(|message| std::io::Error::new(std::io::ErrorKind::InvalidInput, message))?,
).with_policy(RegionPolicy {
    preload_distance: 500.0,
    activation_distance: 160.0,
    unload_distance: 700.0,
    ..RegionPolicy::default()
});
let coordinator = app.world.resource_mut::<StreamingCoordinator>();
coordinator.register_region(definition)?;
coordinator.add_observer(StreamObserver::new(1, Vec3::ZERO))?;
# Ok(()) }
```

Add `RegionModel`, `RegionCollider`, and `RegionPointLight` values to the definition. Model paths are
loaded as `ribbon_plugin_gltf::ModelAsset`; the matching `GltfLoader` must be registered in the
AssetServer. Region dependencies must refer to regions already registered. This registration order
makes dependency cycles impossible in the current API.

## Lifecycle and safety contract

A region moves through `Unloaded`, `Requested`, `LoadingAssets`, `WarmingGpu`, `Ready`,
`PreparingPhysics`, `Activating`, and `Active`. When no longer in activation range, it deactivates
into `Dormant`; after every observer leaves the unload range, it enters `Unloading` and returns to
`Unloaded`. Errors are retained in `Failed(String)` and can be retried with `retry_region` after
Rapier has reconciled removed colliders.

`Ready` means all required typed model assets have CPU data and their renderer warmup state is
`Ready`. `Active` additionally means all authored region colliders have been created and appear in
the physics plugin's reconciled collider snapshot, and all model/light ECS placements have been
spawned. A preview region may spawn render-only entities after `Ready` while the old scene remains
active; this is the doorway-preview pattern used by ATHAR. Preview does not make the region
playable: collision registration remains a separate activation step.

`deactivate` semantics are deliberately different from unload. A dormant region retains asset leases
and GPU warmup residency for rapid re-entry. Unload releases these only after its region-owned
collider entities have disappeared from the physics snapshot. Shared CPU assets are independently
pinned by each region's `AssetRef`; shared warmup entries use an internal user count, so one region
cannot release another region's residency.

The coordinator uses explicit request/activation methods as well as observers. `request_region`
retains a region for preparation; `activate_region` additionally forces activation; release the
manual request with `release_region_request` after an explicit handoff if distance-based retention
should take over. Observers use Ribbon's world-space, right-handed, Y-up convention. Preload and
activation use an OR rule across observers; unload occurs only after every observer is outside the
unload distance. Velocity only biases priority and never affects correctness.

## Budgets and diagnostics

`StreamingConfig` independently bounds new region admissions, typed model-path submissions to
AssetServer, region activations, and spawned region entities per `Update`. `max_active_regions`
counts active/preparing regions and regions whose colliders are still being removed by the physics
owner. A zero budget pauses that class of new work without blocking already-running work. Asset
submissions are ordered by current observer-derived priority; a full AssetServer queue is treated as
backpressure and retried on a later `Update`, not as a permanent region failure.

The AssetServer has a bounded priority queue (256 queued jobs) and bounded completion channel (64
decoded results). Worker jobs are prioritized, FIFO at equal effective priority, and age upward to
prevent starvation. Raising a request's priority updates a job only while it remains queued; running
IO/decode is not preempted. Diagnostics report queued jobs, queue capacity, active workers, and
region paths not yet submitted.

The renderer-owned `ModelWarmup` queue follows the same priority intent: higher-priority region warmups are serviced first, equal priorities preserve FIFO order, and a shared `(asset ID, generation)` request can be promoted by another region without scheduling a duplicate upload. Visible render placements remain ahead of background warmups, and both consume the existing two-model-per-frame upload allowance.

Memory policy has three independent limits. Each Update, the coordinator sets AssetServer's effective
unpinned-cache target to the minimum of `cpu_cache_bytes` and the remaining `cpu_resident_bytes`
allowance after pinned bytes are subtracted. This lets the unleased cache shrink as the total working
set approaches its target; strong `AssetRef` leases are never invalidated. If total decoded CPU
residency is still above `cpu_resident_bytes`, the coordinator may unload one eligible dormant region
per Update, but only when the region owns at least one unique lease (a shared asset with multiple
region leases is not treated as independently reclaimable). `gpu_estimate_bytes` compares against
renderer-tracked resource estimates and may release dormant region warmup residency. In-range,
neighbor-prefetched, requested, active, physics-teardown, and shared-only assets are protected.
Releasing a region drops its lease; AssetServer performs actual unpinned cache eviction under the
effective cache budget. If the remaining live working set is above a target, diagnostics report the
overage rather than invalidating content ownership. All byte counts are estimates, not OS RSS or
driver-reported VRAM.

Inspect `StreamingCoordinator::snapshot()` or `diagnostics()` for region bounds/state, observers,
queue depth/capacity, active workers, still-pending request submissions, effective unpinned CPU-cache
budget and overage, total CPU-residency target and overage, GPU estimates, recent transitions,
invalid transition rejections, and failures. The existing `RenderProfilePlugin` CSV gains
streaming region counts, asset queue/worker counters, request backlog, coordinator CPU time, memory
eviction/pressure counters, and invalid state-transition counts. CPU and GPU byte fields are estimates. GPU values
are copied from existing renderer diagnostics and are not driver-reported VRAM usage.

## Scheduling and ownership details

- Region asset requests are deduplicated by exact model path within a region and by the typed
  path cache in AssetServer across regions. `max_asset_requests_per_update` limits submissions per
  frame, including requests that resolve to an already-loading shared asset.
- Asset leases are retained only while their region is requested, prepared, active, or dormant.
  If a path's generation changes during preparation (for example, hot reload or another consumer's
  retry), the coordinator reacquires a strong lease for the current path generation rather than
  polling an obsolete handle indefinitely. Releasing a region does not preempt an already-running
  shared decode job; a late result has no strong lease and is eligible for AssetServer budget eviction.
- Memory-pressure eviction selects the lowest-priority, least-recently-used eligible dormant
  region. CPU pressure requires at least one asset lease unique to that region; GPU pressure requires
  at least one warmup reference owned only by that region. Regions still within any observer's preload
  range or required as a neighbor prefetch are retained. Active, warming, preparing,
  dependency-pinned, and physics teardown regions are not invalidated to force a budget.
- A region cannot release its asset leases or free its activation slot while the physics snapshot is
  unavailable and its despawned collider entities have not been confirmed removed.

## Current limits

- Region descriptions are currently authored through Rust API calls. No serialized partition-file
  format, editor authoring UI, or automatic discovery of scene-file assets is included.
- The spatial index is a 3D hash of authored AABBs. It is a broad phase; exact point-to-AABB distance
  decides preload/activation/unload. It is not a procedural grid generator or octree.
- The first renderer warmup adapter supports glTF `ModelAsset` through `ModelWarmup`; arbitrary
  texture/mesh/animation asset warmup types need future renderer-owned adapters.
- Region-to-region dependencies are supported. A general typed asset dependency graph and cycle
  diagnostics for arbitrary loader dependencies are not yet provided by AssetServer.
- Physics readiness uses `PhysicsDebugData`'s reconciled collider list. It does not yet prepare
  streamed navmeshes, joints, or other physics structures.
- Renderer upload pacing is currently a model-count cap of two per frame. A strict upload-byte or
  measured upload-time budget is not available in the existing renderer API.
- CPU and GPU residency values use loader/renderer estimates. Driver allocator overhead, transient
  GPU allocations outside tracked resources, and exact VRAM residency are not measured. CPU pressure
  can release whole eligible dormant regions, but it does not split individual region leases or
  perform per-asset LRU within a region.
- Dropping the last region lease releases ownership safely but does not interrupt a shared or
  already-running source/decode job. A late completion is unpinned and may be evicted by the CPU
  budget. Worker shutdown closes the bounded queue and allows an active job to finish.
- A renderer-independent debug snapshot and CSV counters are provided; a dedicated editor viewport
  overlay is not yet implemented.

The ATHAR region is an integration consumer, not part of the coordinator's implementation. It
registers 128 render placements, 111 colliders and eight point lights, stages visuals while the room
remains active, then transfers the player only after the physics-ready `Active` state.
