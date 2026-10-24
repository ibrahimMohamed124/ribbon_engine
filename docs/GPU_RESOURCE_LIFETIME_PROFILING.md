# GPU Resource Lifetime and Profiling

## Scope

This milestone adds explicit reuse/lifetime management for transient 2D render textures, per-render-graph-node CPU timings, frame-wall and surface-acquire/present summaries, tracked resource estimates, and optional asynchronous GPU timestamp samples. It preserves `HdrTarget` as a standalone public API for callers that manage their own resource lifetime. The built-in window and offscreen 3D paths use the pool.

## Feature integration

`ribbon_plugin_render3d` explicitly requests `ribbon_render` with `features = ["gpu"]` and `default-features = false`. This matters when validating the full workspace with `--no-default-features`: the render3d plugin uses GPU context and transient pool APIs and must not rely on `ribbon_render` default features being implicitly enabled. The `ribbon_render::profiling` module is backend-neutral and remains public regardless of the `gpu` feature.

## Transient texture pool

`ribbon_render::gpu::TransientTexturePool` keys reusable allocations by the exact descriptor (width, height, format and usage). The current allocator creates one 2D, single-layer, single-mip, single-sample texture per slot. A `TransientTextureHandle` includes a slot and generation. Only an active lease can return a view or descriptor; releasing twice, releasing a stale handle, or accessing a cached handle fails cleanly. `MeshId` and `MaterialId` also include generations, so releasing and reusing renderer slots does not make stale handles alias the replacement mesh/material. The native renderer drops cached model parts and releases their uploaded mesh/material slots when the last placement of that model disappears from the extracted scene; unused glTF file materials are not uploaded. Dynamic voxel mesh caches follow the same live-scene rule.

Release a target only after encoding/submitting commands that use it and after dropping bind groups/views owned by the caller where required. wgpu retains resources referenced by submitted work, and ordered operations on the same queue preserve use-before-reuse. Cache trimming evicts only unleased textures; it keeps slot indices stable and reuses empty slots with a new generation. The built-in renderers use a 64 MiB estimated cache budget on target release/resize. Active leases are never evicted to satisfy that budget.

The built-in window MeshPass and `OffscreenVoxelRenderer` pool the HDR scene target. Pool trimming budgets only unleased cached textures; active leases are excluded and never evicted. Unknown-size cached formats are evicted when trimming is requested. The offscreen target is also accounted separately as context-owned output. `GpuContext::readback()` retains one synchronous staging buffer between calls, reports its capacity as `context_readback_bytes_estimate`, and drops/reallocates it after a failed map or a target resize. Tone mapper resources are released before the HDR source is returned to the pool. The tone mapper's current estimate accounts for its quarter-resolution RGBA16Float bloom texture; other driver/pipeline allocations are not fully measured. The window surface/swap-chain images are also excluded because their backing allocation is owned by the graphics backend.

## Profiling surface

For native-window applications, `GpuWindowPlugin` installs an `ribbon_render::profiling::RenderDiagnostics` world resource. It provides:

- full CPU duration for the frame path through surface acquisition, graph execution and presentation; running average and peak, in nanoseconds
- total render-graph CPU duration and a name/time pair for each graph node
- asynchronous GPU timing for the shadow-pass interval, main mesh pass, Bloom pass, and tone-mapping pass, when timestamp queries are supported
- timestamp support/sample/skip counters and the latest profiler sequence
- tracked live mesh/material counts, resource byte estimates, transient pool active/cached texture counts and bytes, allocation/reuse counters, and high-water estimates

`OffscreenVoxelRenderer::cpu_timing()` provides frame count, last/average/peak total CPU duration for its render call. This duration includes synchronous output readback and is not directly comparable to the native-window CPU frame value. `gpu_timing_snapshot()` returns the latest completed asynchronous GPU sample, and `resource_stats()` returns tracked estimate categories. Because the public offscreen API exposes a latest-value snapshot rather than a stream of individual completions, the offscreen renderer clears its private completed-sample queues after polling; this prevents the bounded queues from counting unconsumed records as dropped samples without changing the latest snapshot or waiting for the GPU. The offscreen resource snapshot also reports the most recently supplied object count, currently uploaded chunk-mesh entries, cumulative successful chunk-mesh uploads/releases, and instance-buffer capacity/grow/shrink counters; model-cache fields remain zero for this renderer because it does not own the window render pass's glTF model cache.

GPU render-pass timestamp query setup is enabled only when the adapter advertises `wgpu::Features::TIMESTAMP_QUERY`, which is the feature used by `RenderPassTimestampWrites`. The renderer and tone mapper each use a four-slot ring that resolves query data to GPU buffers, copies it to mappable readback buffers and maps asynchronously. The frame loop polls completed results without waiting for unfinished GPU work; if a ring is busy, the sample is skipped and counted.

The renderer query pairs measure the directional-shadow interval (extended through the optional spot-shadow pass when present) and the main mesh pass. Its combined shadow-plus-mesh value spans from directional-shadow start through mesh end, including the gap between those stages. The tone mapper query pairs independently measure Bloom and tone mapping. These query writes do not synchronously wait for results. The latest completed durations carry their own profiler sequence numbers because results can arrive several frames after submission. The CSV stores the latest completed value at each frame snapshot and deduplicates repeated sequence numbers for percentiles; if multiple results complete between snapshots, only the newest is retained for the report, so the distinct GPU sample count can be lower than the cumulative completed-query counter. Check both counts when interpreting GPU distributions.

## Profile captures and reports

`RenderProfilePlugin::new(path)` is opt-in and requires `GpuWindowPlugin` and `Render3dPlugin` to be installed first. It captures a bounded history after `Stage::Render`, ignores the configured warmup (120 successful frames by default), and writes CSV at `Stage::Shutdown`. Native-window examples `graphics_showcase`, `model_viewer`, `horror_slice`, and `voxel_mesh_showcase` accept `--profile-csv PATH` and `--profile-warmup N`. The file is written only after the app shuts down; use `--frames N` for repeatable finite captures.

The CSV contains normalized `frame` records and per-node `pass` records. Run `python tools/render_profile_report.py <capture.csv> --output <report.md>` to create mean/p50/p95/p99/peak timing tables, resource counters and pass rankings. Add `--compare <baseline.csv>` to show current-vs-baseline P95 deltas. GPU stage distributions are deduplicated by their profiler sequence, separately for mesh/shadow and post-processing query rings. The tracked post-process estimate includes the Bloom texture and timestamp readback buffers; it still excludes query-set storage and pipeline/bind-group allocations. See [`PERFORMANCE_BASELINES.md`](PERFORMANCE_BASELINES.md) for exact commands and same-hardware capture guidance. No performance improvement is claimed until before/after captures exist.

## Interpreting estimates

All durations are nanoseconds. CPU graph pass values measure CPU-side invocation/encoding and do not represent GPU execution. GPU timestamp values measure GPU time between timestamp markers and can be absent or delayed. Resource bytes are estimates for known allocation sizes, not actual VRAM reported by the driver. They can exclude alignment, allocator/driver overhead, texture tiling/compression, bind groups, render pipelines, the window surface/swap-chain images, and unknown texture formats. Use the allocation/reuse counters and current/peak estimates to discover likely trends, not as exact GPU budget enforcement.

## Validation and acceptance

On the Windows target on 2026-10-24, `cargo fmt --all --check`, `cargo check --release -p graphics_showcase`, and
`cargo build --release -p graphics_showcase` succeeded. Thirty static placement captures and thirty active-asset
captures ran successfully on NVIDIA GeForce RTX 2050 / driver 581.86 / Vulkan. Their 541-frame CSVs and generated
reports were checked against the requested configuration; the raw-report analyses are in
[`PERFORMANCE_BASELINES.md`](PERFORMANCE_BASELINES.md). The GPU timing readback is asynchronous and nonblocking,
but the CSV retains only the latest completed result at each frame snapshot, so a query can complete between
snapshots without becoming a distinct reported sample. Surface timeout/lost/outdated counts and dropped frames are
not captured. No image-level baseline-versus-current comparison was performed. No complete workspace test or
Clippy matrix was run as part of this profiling work, and the scalability data does not establish a frame-time
speedup from an optimization.
