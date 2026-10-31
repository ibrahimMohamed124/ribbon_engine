# Ribbon World Streaming — Continuation Delivery Report

Date: 2026-10-31

## Executive summary

This continuation builds on the existing `ribbon_plugin_streaming` implementation rather than adding a parallel loader, renderer, scheduler, or physics world. It closes several integration gaps around serialized scene descriptions, prefab-owned resources/entities, authored world partitions, standalone renderer-owned GPU warmup, physics prefab serialization, and streaming diagnostics.

This is an engineering continuation, not a claim that every item in the original open-world production prompt has reached final acceptance. In particular, Rust compilation, unit/integration tests, Clippy, Windows runtime, and real GPU validation were not executable in the environment used to prepare this overlay because `cargo`, `rustc`, `rustfmt`, and PowerShell were unavailable. Run the repository validation script on the Windows development checkout before merging.

## 1. Existing architecture retained

The implementation continues to use the existing ownership boundaries:

- `ribbon_assets` owns CPU file IO, worker decoding, typed asset state, priorities, generations, strong `AssetRef` leases, and decoded-cache eviction.
- `ribbon_scene` owns renderer-independent `Prefab` / `SceneDoc` data, hierarchy validation, component codecs, and transform/hierarchy behavior.
- `ribbon_plugin_render3d` owns all `wgpu` resource creation, upload scheduling, cache retention, and GPU destruction on its render-owning thread.
- `ribbon_plugin_physics` owns Rapier bodies/colliders and reconciles ECS physics components on the existing schedule.
- `ribbon_plugin_streaming` coordinates readiness, region state, observer policy, asset leases, region-owned entities, and diagnostics.
- ATHAR remains a consumer of the generic streaming coordinator; no ATHAR-specific names or asset paths were introduced into the generic runtime.

The dependency direction is kept explicit: the scene crate has no dependency on the asset server, renderer, or physics plugin; the streaming plugin depends on those integrations and coordinates them.

## 2. SceneDoc / Prefab streaming integration

Added `plugins/streaming/src/scene_doc.rs` and coordinator integration:

- `.scenedoc` documents are decoded as `SceneDoc` assets through the normal bounded `AssetServer` worker queue.
- SceneDoc asset-size estimation includes prefab entity/component/value/string storage and metadata rather than only `size_of::<SceneDoc>()`.
- `PrefabCodecRegistry` can expose typed external dependencies through `register_with_dependencies` and `SceneDoc::asset_dependencies` without making `ribbon_scene` depend on `ribbon_assets` or plugin crates.
- Render3D's `ModelInstance` codec declares its required `ModelAsset` dependency; model assets are CPU-loaded and GPU-warmed before the SceneDoc prefab is instantiated.
- `StreamingAssetAdapters` provides type-erased but strongly leased CPU dependency requests for game/plugin asset types. Required dependencies without a registered adapter fail the region with a clear error; optional dependencies do not block readiness.
- Typed dependencies retain a lease while the region is resident, account their declared CPU estimate, report stale generations, and reacquire the current generation after hot reload. When a SceneDoc generation changes, obsolete model and typed dependency leases are pruned from the old dependency graph before readiness and warmup accounting continues.
- The coordinator instantiates prefab-owned entities only after required assets are ready, records their ownership, and despawns them during unload.
- SceneDoc and model generation handling reacquires a current generation rather than waiting forever on an obsolete handle. Replacing a scene document also releases the superseded lease when it becomes unshared.
- Physics readiness includes `Collider` components created by the prefab, not only colliders authored directly on `RegionDefinition`.

The SceneDoc loader remains renderer-agnostic. It does not create GPU resources or perform synchronous asset IO.

## 3. Physics prefab support

Added `plugins/physics/src/prefab.rs` and registered it from `PhysicsPlugin`:

- `Collider`, `CollisionLayers`, and `RigidBody` now have reflection and prefab encode/decode support.
- Serialized values contain portable collider/body configuration only; Rapier runtime handles are never serialized.
- Collider dimensions, friction, restitution, density, velocities, and damping are validated before a decoded component is accepted.
- Invalid prefab component data is rejected through the existing prefab rollback path.
- Render3D and Physics compose into the same `PrefabCodecRegistry`; either plugin registration order preserves the other plugin's codecs instead of replacing the shared registry.

This covers the current collider/body component model. It does not claim serialized support for joints, navmeshes, or other future physics resource types.

## 4. Authored world partition format

Added `plugins/streaming/src/partition_asset.rs` and `WorldPartitionLoader`:

- `.worldpart` RON documents load through the AssetServer worker path.
- Version 1 describes stable partition/region IDs, AABB bounds, optional SceneDoc paths, model placements/transforms, dependencies, neighbors, and per-region observer/activation policy.
- Loading validates IDs, duplicate regions, finite ordered bounds, distance hysteresis, non-empty paths, finite transforms, non-degenerate quaternions, and positive scale.
- `StreamingCoordinator::register_partition` validates unknown/self/duplicate dependency references and cycles before mutating the coordinator, then registers regions in dependency order.
- Neighbor references are validated against the current partition or already registered regions.
- The `.worldpart` size estimator includes dynamic strings, vectors, model placements, and dependency/neighbor paths.
- Added `examples/streaming/starter.worldpart` and two minimal `.scenedoc` files, with usage instructions in `docs/architecture/WORLD_PARTITION_FORMAT.md`.

This is an authored AABB-region format. It is not a procedural world builder, octree, or editor authoring workflow.

## 5. Generic GPU warmup

Added `plugins/render3d/src/gpu_streaming.rs` and renderer integration:

- Stable `GpuResourceKey { namespace, id, generation }` identities prevent new content from being confused with older resource generations.
- Explicit leases deduplicate shared requests and pin queued/preparing/ready resources until the final consumer releases them.
- The renderer-owned queue supports standalone indexed triangle meshes, standalone RGBA8 textures, standalone samplers, and PBR material payloads with embedded RGBA8 texture maps.
- Standalone texture and sampler requests use generation-checked renderer handles, explicit leases, asynchronous wgpu error-scope validation, and render-thread release. `MeshRenderer` exposes generation-checked view/sampler resolution, and samplers are described with renderer-independent address/filter/LOD settings.
- Payloads are validated before queueing; priority can be upgraded; queue ordering is deterministic with wait aging.
- All GPU creation, error-scope validation, and release occur on the renderer-owning thread.
- Asynchronous wgpu validation errors produce `Failed` status and release failed allocations instead of reporting fake success.
- Generic GPU work shares the renderer's bounded per-frame upload operation and estimated-byte budget with model warmup; visible model work retains precedence.
- Generic preparation continues when the window is minimized or no camera is available, avoiding a readiness deadlock where the scene being prepared owns the first camera.
- `GpuMeshInstance3D` can be created from ready mesh/material leases. Extraction retains lease clones so the resource remains pinned while a frame uses it.
- The existing renderer statistics and profile CSV now expose queued/preparing/ready/failed counts, upload/failure/release totals, per-frame upload counts, and estimated upload bytes.

The current public generic payload API covers standalone mesh, texture, and sampler resources plus PBR material resources with embedded texture maps. It does not expose shader/pipeline objects, animation GPU resources, or arbitrary backend payloads. Standalone texture and sampler handles currently provide renderer-owned residency and readiness but are not yet wired into the native `GpuMeshInstance3D` material binding path; custom renderer integrations must consume them through their own pipeline integration.

## 6. Profiling, memory, and editor visibility

- `RenderResourceStats` and CSV output have nine additional generic GPU warmup/upload fields. Existing CSV column ordering is retained; the extension is appended.
- CSV tests now assert the new header/value indices and the complete current 118-column row shape.
- The editor Streaming panel now displays region and request states, CPU/GPU residency estimates and budgets, memory-pressure information, asset worker source/decode durations, generic GPU warmup counters, standalone texture/sampler residency, and upload-budget information.
- SceneDoc and WorldPartition assets now provide CPU decoded-size estimates to the existing AssetServer accounting.
- GPU residency remains an estimate based on renderer-tracked allocations, not driver-reported VRAM or exact physical allocation. CPU totals are likewise decoded/cache estimates, not total process RSS.
- The existing coordinator memory policy protects active and leased content; its eviction granularity remains eligible regions plus the AssetServer's unpinned cache policy, rather than arbitrary per-resource eviction inside a region.

## 7. Tests added or extended

The overlay adds/extends tests for:

- SceneDoc RON loading, invalid UTF-8/RON, and dynamic CPU-size estimation.
- Typed streaming adapter loading/readiness/lease release and missing-adapter reporting.
- Authored partition decode, invalid bounds/version/transform, dependency ordering, and atomic cycle rejection.
- SceneDoc region load, prefab entity ownership through activation/unload, and prefab-owned collider readiness/cleanup.
- Generic GPU resource payload validation, standalone texture/sampler validation, duplicate request sharing/priority upgrade, and lease-safe collection.
- CSV diagnostics assertions for standalone texture/sampler residency and byte estimates.
- Physics Collider/CollisionLayers/RigidBody prefab round-trip, validation failure, and prefab rollback.
- CSV generic-GPU metric header/value alignment.

Tests are present in source but were not run here because the Rust toolchain is unavailable. Unit tests for the generic GPU scheduler are CPU-side scheduling tests; they are not a substitute for a real adapter/device upload test.

## 8. Files changed in this continuation

Modified from the uploaded `Ribbon_Engine(3).zip` (26 files):

- `Cargo.lock`
- `README_APPLY.md`
- `WORLD_STREAMING_DELIVERY_REPORT.md`
- `WORLD_STREAMING_OVERLAY_README.md`
- `crates/render/src/profiling/csv.rs`
- `crates/render/src/profiling/types.rs`
- `crates/scene/src/prefab.rs`
- `plugins/editor_tools/src/lib.rs`
- `plugins/physics/Cargo.toml`
- `plugins/physics/src/lib.rs`
- `plugins/physics/src/plugin.rs`
- `plugins/render3d/src/components.rs`
- `plugins/render3d/src/extract.rs`
- `plugins/render3d/src/lib.rs`
- `plugins/render3d/src/plugin.rs`
- `plugins/render3d/src/prefab.rs`
- `plugins/render3d/src/preload.rs`
- `plugins/render3d/src/renderer.rs`
- `plugins/render3d/src/renderer/assets.rs`
- `plugins/render3d/src/renderer/setup.rs`
- `plugins/streaming/Cargo.toml`
- `plugins/streaming/src/coordinator.rs`
- `plugins/streaming/src/lib.rs`
- `plugins/streaming/src/scene_doc.rs`
- `tools/allowed_deps.toml`
- `tools/validate_streaming.ps1`

Added (8 files):

- `docs/architecture/WORLD_PARTITION_FORMAT.md`
- `examples/streaming/scenes/north_path.scenedoc`
- `examples/streaming/scenes/starting_courtyard.scenedoc`
- `examples/streaming/starter.worldpart`
- `plugins/physics/src/prefab.rs`
- `plugins/render3d/src/gpu_streaming.rs`
- `plugins/streaming/src/asset_adapters.rs`
- `plugins/streaming/src/partition_asset.rs`

## 9. Validation status

### Static checks performed in the preparation environment

- Parsed all 40 workspace/package Cargo manifests and dependency-policy TOML with Python's TOML parser.
- Verified all 39 workspace member paths, direct dependency allowlist changes, and lockfile package/dependency entries.
- Verified no original source files were removed from the uploaded repository tree.
- Verified `.worldpart` and `.scenedoc` example paths exist.
- Parsed the profile CSV source header and verified the 118-column schema, including the standalone texture/sampler metric indices (115–117).
- Compared the working tree byte-for-byte against the uploaded archive: 26 modified files, 8 added files, and no deleted original files. The final overlay archive is checked separately before delivery.

### Not run here

- `cargo fmt --all -- --check`
- `cargo check --workspace --all-targets`
- Any `cargo test` / `cargo test --workspace --no-fail-fast`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `py -3 tools/check_deps.py` (the script invokes `cargo metadata`; `cargo` is not installed in this environment)
- Windows application launch, gameplay transition tests, and manual editor inspection
- Real GPU adapter upload/error/device-loss validation and runtime frame-time measurements

Run the following from the Windows repository root:

```powershell
Set-ExecutionPolicy -Scope Process Bypass
.\tools\validate_streaming.ps1
```

The script includes formatting, workspace and focused package checks/tests, ATHAR, Clippy, and dependency-policy validation. Preserve the full console output and fix any failure before treating the overlay as merged.

## 10. Remaining limitations / acceptance gaps

1. No local Rust compilation/test/clippy result is available yet. Source-level additions must pass the repository validation script before merge.
2. Generic GPU warmup now exposes independent mesh, texture, and sampler resources plus PBR material payloads. Shader/pipeline and animation GPU resources are not independently managed by the current renderer. Standalone texture/sampler leases provide renderer-owned lifetime and readiness; native PBR `GpuMeshInstance3D` currently binds textures through its material path.
3. Custom typed SceneDoc adapters guarantee CPU readiness and a caller-provided size estimate only; plugins must explicitly submit non-model GPU resources through the renderer API. Those arbitrary custom GPU dependencies are not auto-inferred from a typed CPU adapter.
4. SceneDoc hot reload is generation-aware during preparation and reacquires the current document/dependency handles. It does not yet atomically diff and replace already-active prefab entity graphs in place; active document reload policy should be expanded before depending on live-editing semantics.
5. Superseded SceneDoc dependency graphs are reconciled during preparation: obsolete model and typed CPU leases are released after the replacement document's dependency set is discovered. Active prefab graphs are still not atomically diffed/replaced in-place; reload during the active state remains outside the current contract.
6. Authored `.worldpart` is available, but there is no dedicated editor authoring UI or procedural partition generator.
7. Physics prefab codecs cover the current `Collider`, `CollisionLayers`, and `RigidBody` components only. Joints, navigation data, and arbitrary physics-owned assets need explicit codecs/readiness gates if added.
8. Memory values are estimates. There is no exact process RSS or device VRAM accounting, nor arbitrary per-resource-class eviction across all renderer resource families.
9. Existing region-level eviction and AssetServer cache eviction remain coarse-grained; a complete lease-aware per-resource CPU/GPU residency manager is still a future extension.
10. GPU timestamps remain asynchronous according to the existing wgpu query path. The new generic upload byte count is an estimate for scheduling, not measured GPU execution duration.
11. No manual Windows visual verification, ATHAR room-to-village runtime timing, stress test, or long-duration multi-observer test was performed during this continuation.

The overlay should therefore be described as an expanded streaming foundation with substantially stronger SceneDoc, prefab, authored partition, and generic mesh/material warmup integration—not as proof that every original production acceptance criterion has been met.
