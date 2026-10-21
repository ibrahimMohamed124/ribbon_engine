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
- The stage scheduler is deterministic and serial. This is a safe baseline, not a parallel scheduler.
- `plugin_voxel::ChunkedVoxelWorld` tracks edits/dirty chunks and `plugin_render3d` releases removed dynamic meshes.
- CPU rendering remains a deterministic reference path; the render graph validates named resource dependencies.

## Priority 1 — transform invalidation

**Current limit:** `propagate_transforms` and `propagate_transforms_3d` rebuild the transform maps and
resolve the hierarchy in `PostUpdate`. Public mutable fields mean the engine cannot reliably know which
component changed without an explicit change-detection contract.

**Design:** add component change ticks or controlled transform mutation APIs, invalidate descendants when a
local transform or parent link changes, recompute only dirty branches, and write `GlobalTransform` only when
its resolved value differs. Preserve deterministic parent-before-child order, cycle safeguards, 2D/voxel vs
quaternion-based 3D conventions, and the existing golden/render baselines. Benchmark sparse and all-dirty scenes.

## Priority 2 — asset lifetime and streaming

**Current limit:** loads are synchronous; `Handle<T>` is a copyable non-owning ID and the server owns cached
values without a strong-reference unload/eviction policy.

**Design:** introduce an explicit `AssetId` + strong `AssetRef<T>`/weak `Handle<T>` ownership distinction,
reference or lease counting, generation-safe eviction, load-state events, and worker-thread IO/decode. GPU upload
should remain on the renderer/device-owning thread. Hot-reload events must keep stable IDs only on successful
replacement; failed reloads must retain or explicitly invalidate the previous value by documented policy.

## Priority 3 — access-declared scheduling

**Current limit:** systems get exclusive `&mut World`; arbitrary systems cannot safely execute in parallel.

**Design:** define read/write component/resource access metadata, reject invalid duplicate access declarations, build
a conflict graph per stage, keep ordered barriers and explicit before/after constraints, and provide a deterministic
serial fallback. Parallelism should only schedule conflict-free systems and should be checked against serial output.

## Priority 4 — render resource lifetime and profiling

**Current limit:** `RenderGraph` validates named-resource dependencies/order, but does not allocate transient GPU
resources, alias compatible allocations, expose GPU timestamp profiling, or schedule asynchronous copies.

**Design:** add typed resource descriptors and lifetimes, a transient resource pool, explicit surface/present ownership,
GPU timestamp queries with graceful fallback, and per-pass CPU/GPU timings. Track peak allocated bytes and frame
latency before applying aliasing or async scheduling.

## Priority 5 — rendering API ownership

`engine_render`, `plugin_rendering`, and `plugin_render3d` intentionally remain separate: shared frame/draw contracts;
voxel scene/material data plus CPU/GPU voxel backends and ECS bridge; native-surface mesh rendering. Split or rename
`plugin_rendering` only when material/scene ownership and backend interfaces can be separated without breaking the
CPU golden baselines or forcing `engine_platform` to depend on a renderer implementation.

## Acceptance for future changes

Each milestone should include a short ADR, before/after benchmark for the targeted path, public API/dependency review,
and CI coverage for default and `--no-default-features` configurations. Preserve STUDS golden values unless a separately
approved, explicitly reviewed behavior change requires a new baseline. Never report an optimization as successful based
only on compiling or on a single interactive screenshot.
