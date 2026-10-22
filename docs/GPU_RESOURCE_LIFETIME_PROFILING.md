# GPU Resource Lifetime and Profiling

## Scope

This milestone adds explicit reuse/lifetime management for transient 2D render textures, per-render-graph-node CPU timings, frame-wall and surface-acquire/present summaries, tracked resource estimates, and optional asynchronous GPU timestamp samples. It preserves `HdrTarget` as a standalone public API for callers that manage their own resource lifetime. The built-in window and offscreen 3D paths use the pool.

## Feature integration

`plugin_render3d` explicitly requests `engine_render` with `features = ["gpu"]` and `default-features = false`. This matters when validating the full workspace with `--no-default-features`: the render3d plugin uses GPU context and transient pool APIs and must not rely on `engine_render` default features being implicitly enabled. The `engine_render::profiling` module is backend-neutral and remains public regardless of the `gpu` feature.

## Transient texture pool

`engine_render::gpu::TransientTexturePool` keys reusable allocations by the exact descriptor (width, height, format and usage). The current allocator creates one 2D, single-layer, single-mip, single-sample texture per slot. A `TransientTextureHandle` includes a slot and generation. Only an active lease can return a view or descriptor; releasing twice, releasing a stale handle, or accessing a cached handle fails cleanly. `MeshId` and `MaterialId` also include generations, so releasing and reusing renderer slots does not make stale handles alias the replacement mesh/material. The native renderer drops cached model parts and releases their uploaded mesh/material slots when the last placement of that model disappears from the extracted scene; unused glTF file materials are not uploaded. Dynamic voxel mesh caches follow the same live-scene rule.

Release a target only after encoding/submitting commands that use it and after dropping bind groups/views owned by the caller where required. wgpu retains resources referenced by submitted work, and ordered operations on the same queue preserve use-before-reuse. Cache trimming evicts only unleased textures; it keeps slot indices stable and reuses empty slots with a new generation. The built-in renderers use a 64 MiB estimated cache budget on target release/resize. Active leases are never evicted to satisfy that budget.

The built-in window MeshPass and `OffscreenVoxelRenderer` pool the HDR scene target. Pool trimming budgets only unleased cached textures; active leases are excluded and never evicted. Unknown-size cached formats are evicted when trimming is requested. The offscreen target is also accounted separately as context-owned output. `GpuContext::readback()` retains one synchronous staging buffer between calls, reports its capacity as `context_readback_bytes_estimate`, and drops/reallocates it after a failed map or a target resize. Tone mapper resources are released before the HDR source is returned to the pool. The tone mapper's current estimate accounts for its quarter-resolution RGBA16Float bloom texture; other driver/pipeline allocations are not fully measured. The window surface/swap-chain images are also excluded because their backing allocation is owned by the graphics backend.

## Profiling surface

For native-window applications, `GpuWindowPlugin` installs an `engine_render::profiling::RenderDiagnostics` world resource. It provides:

- full CPU duration for the frame path through surface acquisition, graph execution and presentation; running average and peak, in nanoseconds
- total render-graph CPU duration and a name/time pair for each graph node
- last completed shadow-through-mesh GPU duration and main mesh-pass GPU duration, when timestamp queries are supported
- timestamp support/sample/skip counters and the latest profiler sequence
- tracked live mesh/material counts, resource byte estimates, transient pool active/cached texture counts and bytes, allocation/reuse counters, and high-water estimates

`OffscreenVoxelRenderer::cpu_timing()` provides frame count, last/average/peak total CPU duration for its render call. This duration includes synchronous output readback and is not directly comparable to the native-window CPU frame value. `gpu_timing_snapshot()` returns the latest completed asynchronous GPU sample, and `resource_stats()` returns tracked estimate categories.

GPU render-pass timestamp query setup is enabled only when the adapter advertises `wgpu::Features::TIMESTAMP_QUERY`, which is the feature used by `RenderPassTimestampWrites`. A four-slot ring resolves query data to GPU buffers, copies it to mappable readback buffers and maps asynchronously. The render path polls for completed results without blocking to wait on a sample; if the ring is busy the current sample is skipped and counted. The combined GPU interval starts at the directional shadow pass and ends at the main mesh pass; an optional spot shadow pass lies inside that interval. The main mesh pass is timed separately. These readings can arrive several frames after submission.

## Profile captures and reports

`RenderProfilePlugin::new(path)` is opt-in and requires `GpuWindowPlugin` and `Render3dPlugin` to be installed first. It captures a bounded history after `Stage::Render`, ignores the configured warmup (120 successful frames by default), and writes CSV at `Stage::Shutdown`. Native-window examples `graphics_showcase`, `model_viewer`, `horror_slice`, and `voxel_mesh_showcase` accept `--profile-csv PATH` and `--profile-warmup N`. The file is written only after the app shuts down; use `--frames N` for repeatable finite captures.

The CSV contains normalized `frame` records and per-node `pass` records. Run `python tools/render_profile_report.py <capture.csv> --output <report.md>` to create mean/p50/p95/p99/peak timing tables, resource counters and pass rankings. Add `--compare <baseline.csv>` to show current-vs-baseline P95 deltas. GPU samples are deduplicated by profiler submission sequence so a delayed result is not counted repeatedly. See [`PERFORMANCE_BASELINES.md`](PERFORMANCE_BASELINES.md) for exact commands and same-hardware capture guidance. No performance improvement is claimed until before/after captures exist.

## Interpreting estimates

All durations are nanoseconds. CPU graph pass values measure CPU-side invocation/encoding and do not represent GPU execution. GPU timestamp values measure GPU time between timestamp markers and can be absent or delayed. Resource bytes are estimates for known allocation sizes, not actual VRAM reported by the driver. They can exclude alignment, allocator/driver overhead, texture tiling/compression, bind groups, render pipelines, the window surface/swap-chain images, and unknown texture formats. Use the allocation/reuse counters and current/peak estimates to discover likely trends, not as exact GPU budget enforcement.

## Validation and acceptance

No new tests were added for this delivery, and the project or Cargo commands were not run in the implementation environment. Run these commands in a clean checkout on the target platform:

```powershell
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace -j 1
cargo test --workspace --no-default-features -j 1
python tools/check_deps.py
```

Then record baseline and new measurements for both a static scene and a dynamic scene with frequent mesh changes. Inspect CPU frame/graph timings, asynchronous GPU samples where supported, transient texture allocations/reuses, active/cached bytes and peaks. Do not claim a speedup until these values have been collected on target hardware. Existing CPU golden outputs should remain unchanged.
