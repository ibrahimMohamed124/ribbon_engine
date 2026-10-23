# Engine scalability roadmap

This roadmap separates current architectural guarantees from future capacity work. The engine should
scale by adding explicit ownership and invalidation contracts, not by adding dependencies or parallelism
that can bypass those contracts. None of the milestones below is claimed as implemented solely because
it is documented.

## Current guarantees

- Engine crates and plugins are separate from games/examples; `tools/allowed_deps.toml` and
  `tools/check_deps.py` enforce direct dependency policy across normal, build, development, and optional edges.
- The deprecated `engine/`/`voxelworks-*` compatibility tree has been removed. New examples use the ECS
  app and current plugin APIs.
- `engine_schedule` preserves a deterministic serial fallback and now builds access-conflict batches for opted-in parallel systems. Legacy `&mut World` systems remain serial barriers.
- `plugin_voxel::ChunkedVoxelWorld` tracks edits/dirty chunks and `plugin_render3d` releases removed dynamic meshes using generation-checked mesh/material handles.
- CPU rendering remains a deterministic reference path; the render graph validates named resource dependencies.

## Priority 1 — transform invalidation

**Status:** first incremental-resolution milestone implemented; profiling and lower-cost mutation tracking remain.

**Prior limit:** both propagation systems resolved every transform-bearing entity at `PostUpdate` and wrote
all outputs each frame. Since local transform fields are public and are commonly modified through `query_mut`,
there was no reliable way to depend on an explicit change tick without breaking existing caller patterns.

**Implemented design:** `engine_scene` now snapshots local transforms and parent links, detects added/removed/changed
values, and invalidates descendants across the union of the previous and current hierarchy edges. The old graph
is included so reparenting or removing a transform does not leave descendants with stale globals. Dirty branches
are resolved parent-first with deterministic entity ordering; clean ancestors reuse cached global transforms.
Global components are only written when the resolved value differs, and the derived global is removed when its
local transform component disappears. The 2D/voxel angle convention and quaternion-based 3D composition remain
separate, and the existing cycle fallback behavior is retained.

**Known bound:** because `Transform`, `Transform3D`, and `Parent` remain directly mutable, each pass still needs
an O(N) snapshot scan to detect changes safely. The current improvement skips hierarchy composition and component
writes for clean branches; it does not claim sublinear total frame cost or benchmark-proven gains. The next step
within this milestone is target-side sparse/all-dirty profiling. Only after these measurements should the project
consider ECS change ticks or controlled transform mutation APIs that can avoid full scans without breaking current
callers. See [`TRANSFORM_INVALIDATION.md`](TRANSFORM_INVALIDATION.md) for the implementation contract and verification status.

## Priority 2 — asset lifetime and streaming

**Status:** worker-backed loading, strong leases, generation-safe eviction, loader-provided CPU size estimates, unpinned-cache budgeting, and cache-retention priorities are implemented; target-side compilation, lifecycle verification, and multi-asset memory/performance measurement remain mandatory.

**Prior limit:** loads were synchronous; `Handle<T>` was only a copyable non-owning ID and cached assets could not be evicted safely.

**Implemented design:** `AssetId` identifies the path entry, weak `Handle<T>` carries a generation, and strong `AssetRef<T>` leases pin a generation against eviction. Regular source IO and CPU decode can run on worker threads; `AssetPlugin` commits completed results and emits lifecycle events on the server-owning thread. The task queue and pending decoded-completion queue are bounded, with explicit `QueueFull` feedback and retry support. The legacy synchronous APIs remain available. Successful hot reload keeps the handle generation, advances a content revision, and emits `Modified`; failed hot reload retains the last-good value and emits `ReloadFailed`. Model viewer and Horror Slice now request glTF models asynchronously and retain strong leases. GPU upload remains inside the rendering plugin's render pass.

**Boundaries:** auto-eviction remains opt-in because legacy consumers use weak handles; callers must pin assets with `AssetRef<T>` when they need residency. CPU decoded-byte accounting is a loader estimate and the configured budget applies to unpinned resident cache values only. Cache-retention priority affects eviction order (priority first, FIFO load age second), not worker scheduling. There is still no request cancellation, progress reporting, access-based LRU, dependency graph, or strict byte limit for queued/in-flight decode results. Hot-reload timestamp detection still polls synchronously, but source reads/decode run on workers for AssetPlugin/request users. See [`ASSET_LIFETIME_STREAMING.md`](ASSET_LIFETIME_STREAMING.md) and [`CPU_ASSET_CACHE_BUDGET.md`](CPU_ASSET_CACHE_BUDGET.md).

**Verification:** no Rust formatter/compiler/linter, runtime, benchmark, or test command was run for this delivery, and no new tests were added. Existing tests were left unchanged. Run the target-side command matrix before treating this milestone as accepted.

## Priority 3 — access-declared scheduling

**Status:** first access-declared parallel execution layer implemented; target-side compilation, race/behavior review, serial-vs-parallel comparison, and profiling remain mandatory.

**Previous limit:** systems accepted unrestricted exclusive `&mut World`, so the scheduler could not safely run arbitrary systems concurrently.

**Implemented design:** legacy systems created with `SystemConfig::new` are explicitly exclusive and remain serial barriers. `SystemConfig::parallel` accepts a `Fn(&World) + Send + Sync` callback plus declared component/resource reads and writes. Compatible systems form deterministic conflict-free stage batches; a write conflicts with reads and writes of the same typed component/resource, while read/read may overlap. User-defined `before`/`after` constraints are kept, and conflict edges are oriented by the stable declared topological order. Missing/duplicate access declarations, duplicate labels/names, unknown labels, and dependency cycles are rejected. `Schedule::execution_plan` exposes the built batches; parallel execution can be disabled or capped with a serial fallback. `SyncResource<T>` provides lock-backed resource mutation through shared world access.

**Important bounds:** access declarations are a contract, not compile-time filtering. Parallel callbacks must declare every touched component/resource accurately; hecs dynamic borrow checking protects component borrows, and resources intended for mutation must use interior mutability (prefer `SyncResource<T>`). Structural changes stay in exclusive systems/command phases. Conditional systems are serial barriers because their accesses cannot be inferred from the condition closure. The current implementation creates a bounded scoped worker set per parallel batch rather than maintaining a persistent pool; thread startup overhead must be measured before wider adoption.

**Verification still required:** no Rust compiler, linter, runtime or benchmark was run for this delivery, and no new tests were added. On the target checkout, run the standard workspace matrix and compare serial fallback (`set_parallel_enabled(false)` or `set_max_parallelism(1)`) against parallel output. Keep STUDS golden assertions unchanged. See [`ACCESS_DECLARED_PARALLEL_SCHEDULER.md`](ACCESS_DECLARED_PARALLEL_SCHEDULER.md).

## Priority 4 — render resource lifetime and profiling

**Status:** target pooling and CPU/GPU diagnostics are implemented; opt-in bounded CSV capture and Markdown performance reports are now implemented. Target-side validation and actual before/after workload measurements remain required. Resource estimates are approximations, not driver-reported VRAM usage.

**Implemented design:** `engine_render::gpu::TransientTexturePool` owns reusable single-sample 2D textures keyed by exact width/height/format/usage descriptors. Generation-checked leases prevent stale handles from exposing or releasing a texture after it has been returned to the cache and leased again. Cached resources can be trimmed to an estimated-byte budget without evicting active leases. The native window renderer and offscreen voxel renderer pool their HDR scene targets; post-process bind groups are dropped before a source target is released. The native render graph still owns pass order, and `GpuWindow` remains the owner of acquiring/presenting the window surface image.

`engine_render::profiling::RenderDiagnostics` records full-frame wall duration (including acquisition and presentation), separate surface-acquire/present wall durations, running average/peak, render-graph node CPU durations, graph CPU duration, current tracked resource categories, transient pool reuse/allocation counters, and high-water estimates. `plugin_render3d` adds an asynchronous four-slot GPU timestamp readback ring with graceful fallback when the adapter does not expose `TIMESTAMP_QUERY`; it reports the shadow-through-mesh GPU interval and the main mesh pass interval separately. Readback is polled without waiting for unfinished GPU work. `OffscreenVoxelRenderer` exposes CPU frame timing (including its synchronous framebuffer readback), GPU timing snapshots, and memory estimates directly. The native model cache releases uploaded model meshes/materials and prunes revision history after their final extracted-scene placement disappears.

**Accounting contract:** estimates include mesh vertex/index buffers, material textures/uniforms, known uniform/instance buffers, depth/shadow targets, the context RGBA8 target and reusable readback staging buffer, transient HDR leases/cache, and the tone mapper's quarter-resolution bloom texture. Driver allocator alignment, tiling/compression, heap overhead, pipeline/bind-group overhead, window surface/swap-chain images, and unknown pixel formats may be excluded. The transient pool currently supports 2D, single-layer, single-mip, single-sample textures; it does not alias resources within a single frame and is not a general render-graph allocator. Its 64 MiB cache budget applies only to unleased cached textures; active render targets are protected and excluded from that budget, so total pool bytes can exceed it. Cached texture formats without a byte estimator are evicted when trimming is requested. Timestamp queries are optional, asynchronous and backend-dependent, so UI/telemetry must tolerate absent or delayed samples.

**Baseline tooling:** `RenderProfilePlugin` captures a bounded post-warmup window-frame history to CSV on shutdown; `tools/render_profile_report.py` summarizes mean/p50/p95/p99/peak frame-wall, surface-acquire, render-graph CPU and surface-present timings, deduplicated asynchronous GPU samples, resource estimates, and highest-P95 graph nodes. It can compare current P95 values with a previously saved capture. The repeatable procedure and four representative scenarios are documented in [`PERFORMANCE_BASELINES.md`](PERFORMANCE_BASELINES.md).

**Not claimed yet:** no actual VRAM reading, allocation attribution from the driver, async framebuffer readback, general transient aliasing, or complete per-GPU-pass measurements are provided. No performance speedup is claimed until identical baseline/current scenarios have been measured on target hardware. Run the workspace validation matrix and capture at least the mixed mesh and dynamic voxel scenarios before selecting the first data-driven optimization. See [`GPU_RESOURCE_LIFETIME_PROFILING.md`](GPU_RESOURCE_LIFETIME_PROFILING.md) and [`PERFORMANCE_BASELINES.md`](PERFORMANCE_BASELINES.md).

## Priority 4a — texture and material memory audit

**Status:** asset-reference correction, renderer texture sharing, and allocation-accounting breakdown implemented in source; target-side build/runtime/profile confirmation remains.

The Horror Slice capture reported 224 MiB of estimated material data/textures. Static inspection found that `tools/horror_room_assets.py` computed material texture indices from `len(images)` before that list was populated, so maps all referenced texture zero. The generator and checked-in GLB references now route diffuse, ARM, normal, and emissive inputs to their intended images. `plugin_render3d::MeshRenderer` shares GPU texture allocations across materials and GLBs when decoded pixel content, dimensions, and sRGB/linear format match. Materials hold strong references; a weak cache does not retain unused GPU textures. Resource diagnostics now count unique live material textures once and expose texture bytes, uniform bytes, unique texture count, and cache hit/miss counters.

The nine unique 1K maps plus one 128x128 emissive placeholder imply a rough post-fix texture estimate around 48.1 MiB plus tiny fallbacks and material uniforms; this is a prediction, not an observed result. Correcting the GLB map indices changes the Horror Slice look to use the intended maps, so a visual check is part of acceptance. See [`TEXTURE_MATERIAL_MEMORY_AUDIT.md`](TEXTURE_MATERIAL_MEMORY_AUDIT.md).

## Priority 4b — asset lifetime and large-scene stress

**Status:** opt-in repeated-model stress grid, CPU asset eviction/reload cycles, GPU cache counters, and delayed instance-buffer shrinking are implemented in `graphics_showcase`; target-side validation remains required.

`graphics_showcase --stress-grid SIDE` spawns a bounded `SIDE * SIDE` grid of placements that all reference one `ModelAsset` generation. This separates ECS placement/draw workload from the count of distinct GPU model resources. The grid is opt-in, capped at 128 per side, and the ordinary showcase does not register its stress-cycle system or enable asset auto-eviction.

`--stress-cycle-frames N` periodically removes the grid, drops its strong `AssetRef<ModelAsset>`, evicts unpinned CPU assets, requests the next generation asynchronously, and restores the placements after loading completes. `plugin_render3d` prunes model/voxel GPU cache entries after the extracted scene no longer references them; the weak content-addressed texture cache does not own textures strongly. Profile CSV/reporting records latest and peak placement/cache/instance-capacity values plus cumulative model and voxel upload/release events. `tools/run_large_scene_stress.ps1` captures the recommended 16/32/48/64 grid sweep and an asset-lifecycle cycle into a timestamped folder with a manifest. Instance buffers grow immediately and shrink only after at least 120 frames of demand at one-quarter capacity or less, when capacity is above 256.

This is a repeatable stress harness, not a full streaming-world implementation. It exercises many instances of one model and repeated eviction/reload of that asset; it does not prove scalability for hundreds of distinct asset files, enforce a decoded-byte CPU budget, schedule prioritized/cancelable requests, or stream regions around a moving camera. Run the default workspace validation matrix and collect the steady-grid and asset-cycle captures described in [`ASSET_LIFETIME_LARGE_SCENE_STRESS.md`](ASSET_LIFETIME_LARGE_SCENE_STRESS.md) before declaring large-world readiness.

## Priority 5 — data-driven rendering optimization (measurement-gated)

**Status:** planned after repeatable captures have been collected; do not implement candidate optimizations on a guess alone.

Capture `graphics_showcase` and `voxel_mesh_showcase` on the target Windows GPU, then select one dominant cost based on
P95 frame/graph CPU time, deduplicated GPU timestamp samples, per-pass CPU timing, mesh/material counts, resource estimates,
and transient allocation/reuse counts. Candidate work may include draw batching/instancing, visibility filtering, shadow
quality/pass optimization, post-processing resolution, or removing repeated CPU extraction work. Select only the candidate
supported by the measurements, change one major variable at a time, and compare matching release-build captures. Preserve
the current CPU/golden reference results and report no speedup unless the before/after capture shows it.

## Priority 6 — rendering API ownership

`engine_render`, `plugin_rendering`, and `plugin_render3d` intentionally remain separate: shared frame/draw contracts;
voxel scene/material data plus CPU/GPU voxel backends and ECS bridge; native-surface mesh rendering. Split or rename
`plugin_rendering` only when material/scene ownership and backend interfaces can be separated without breaking the
CPU golden baselines or forcing `engine_platform` to depend on a renderer implementation.

## Acceptance for future changes

Each milestone should include a short ADR, before/after benchmark for the targeted path, public API/dependency review,
and CI coverage for default and `--no-default-features` configurations. Preserve STUDS golden values unless a separately
approved, explicitly reviewed behavior change requires a new baseline. Never report an optimization as successful based
only on compiling or on a single interactive screenshot.
