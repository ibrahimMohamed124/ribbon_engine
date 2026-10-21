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

**Status:** first worker-backed loading and lease/eviction API implemented; target-side compilation, lifecycle verification, and memory/performance measurement remain mandatory.

**Prior limit:** loads were synchronous; `Handle<T>` was only a copyable non-owning ID and cached assets could not be evicted safely.

**Implemented design:** `AssetId` identifies the path entry, weak `Handle<T>` carries a generation, and strong `AssetRef<T>` leases pin a generation against eviction. Regular source IO and CPU decode can run on worker threads; `AssetPlugin` commits completed results and emits lifecycle events on the server-owning thread. The task queue and pending decoded-completion queue are bounded, with explicit `QueueFull` feedback and retry support. The legacy synchronous APIs remain available. Successful hot reload keeps the handle generation, advances a content revision, and emits `Modified`; failed hot reload retains the last-good value and emits `ReloadFailed`. Model viewer and Horror Slice now request glTF models asynchronously and retain strong leases. GPU upload remains inside the rendering plugin's render pass.

**Boundaries:** auto-eviction is opt-in because legacy consumers use weak handles; callers must pin assets with `AssetRef<T>` when they need residency. Hot-reload timestamp detection still polls synchronously, but source reads/decode run on workers for AssetPlugin/request users; direct synchronous-only users can preserve immediate reload behavior. There is no request priority/cancellation, progress reporting, byte budget, or LRU policy. Queue capacities bound counts, not decoded bytes. See [`ASSET_LIFETIME_STREAMING.md`](ASSET_LIFETIME_STREAMING.md).

**Verification:** no Rust formatter/compiler/linter, runtime, benchmark, or test command was run for this delivery, and no new tests were added. Existing tests were left unchanged. Run the target-side command matrix before treating this milestone as accepted.

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
