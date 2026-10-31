# Render profile capture and performance baselines

## Purpose

Use this workflow to collect comparable performance evidence before changing the renderer. The profile
capture plugin is opt-in; when it is absent, the engine does not allocate frame-history samples or do
profile-file I/O. With the plugin installed, samples are bounded, warmup frames are excluded, and CSV
is written once during `Stage::Shutdown`.

The capture records one `frame` row per retained successful window presentation and zero or more `pass`
rows containing render-graph node CPU durations. Columns include whole-frame wall duration, surface-acquire
and surface-present wall durations, graph CPU time, per-node CPU time, latest completed GPU timestamp
results and their submission sequence, resource estimates, transient texture allocation/reuse counters,
and cumulative GPU-profiler sample counts. Material diagnostics also report unique live GPU textures, unique texture mip-chain bytes, material uniform bytes, and cumulative content-cache hit/miss counters; those breakdown fields are included in the aggregate material-byte estimate, not extra categories in the total.

## Capture options

The four representative native-surface examples accept:

- `--profile-csv PATH`: enable profile capture and write the CSV at shutdown.
- `--profile-warmup N`: exclude the first N successfully rendered frames; default is 120.
- `--frames N`: already supported by each example and should be used to end a capture consistently.

The recorder retains up to 3,600 post-warmup frames by default. If the run lasts longer, it keeps the
most recent samples in a bounded rolling window. It creates the output directory as needed. A profile
capture that ends before warmup writes a header but has no frame records; the report tool reports this
as an input error so it is not mistaken for an empty/fast run.

## Suggested capture scenarios

Run release builds on the same machine and adapter for every before/after pair. Use a stable resolution,
frame count, warmup, scene content and GPU power state where practical.

### Mixed mesh, material, shadow and post-processing workload

```powershell
cargo run --release -p graphics_showcase -- --frames 1800 --profile-warmup 120 --profile-csv target/profiles/graphics-showcase.csv
python tools/perf/render_profile_report.py target/profiles/graphics-showcase.csv --output target/profiles/graphics-showcase.md
```

This scene contains moving built-in meshes, glTF models, a directional light/shadow pass, HDR tone mapping,
and frustum culling. It is the primary broad rendering baseline.

### Single glTF model workload

```powershell
cargo run --release -p model_viewer -- --frames 1800 --profile-warmup 120 --profile-csv target/profiles/model-viewer.csv
python tools/perf/render_profile_report.py target/profiles/model-viewer.csv --output target/profiles/model-viewer.md
```

Use the bundled checker cube unless the exact same external model is supplied to both baseline and current
runs. This scenario is useful for the steady-state mesh/material pipeline after initial asynchronous loading.

### Dynamic voxel chunk workload

```powershell
cargo run --release -p voxel_mesh_showcase -- --frames 1800 --profile-warmup 240 --profile-csv target/profiles/voxel-mesh-showcase.csv
python tools/perf/render_profile_report.py target/profiles/voxel-mesh-showcase.csv --output target/profiles/voxel-mesh-showcase.md
```

This example removes and restores a boundary voxel and exercises dirty-chunk remeshing, mesh replacement,
and transient render targets. A 240-frame warmup deliberately excludes the initial scene setup and the
built-in edits at frames 120 and 180; use a separate run with a shorter warmup if those bursts themselves
are the subject of the measurement.

### Native first-person scene

```powershell
cargo run --release -p athar -- --frames 1800 --profile-warmup 120 --profile-csv target/profiles/athar.csv
python tools/perf/render_profile_report.py target/profiles/athar.csv --output target/profiles/athar.md
```

Without scripted input, this captures the loaded room in its default state. Treat movement and mouse-look
runs as a separate scenario and describe the interaction sequence in the report metadata.

### Large-scene repeated-model stress and asset lifetime

The stress switches are opt-in to `graphics_showcase` so the normal showcase and its baseline remain unchanged. Start
with a grid of 32 by 32 placements; increase the side only after a lower-size capture completes successfully:

```powershell
cargo run --release -p graphics_showcase -- --stress-grid 32 --frames 1800 --profile-warmup 60 --profile-csv target/profiles/large-scene.csv
python tools/perf/render_profile_report.py target/profiles/large-scene.csv --output target/profiles/large-scene.md
```

Use an interval of 300 frames to exercise CPU asset unload/reload, model/voxel GPU-cache cleanup, texture-cache
release, and instance-buffer shrink behavior:

```powershell
cargo run --release -p graphics_showcase -- --stress-grid 24 --stress-cycle-frames 300 --frames 1800 --profile-warmup 60 --profile-csv target/profiles/asset-lifecycle.csv
python tools/perf/render_profile_report.py target/profiles/asset-lifecycle.csv --output target/profiles/asset-lifecycle.md
```

For these reports, use both `Latest` and `Peak in retained capture`: when the final frame lands in an inactive
cycle, latest placements/cache counts can be zero even though the capture successfully stressed a large grid.
The GPU model upload/release counters count distinct model cache entries, not individual primitive buffers; voxel
upload/release counters count distinct immutable chunk-mesh cache entries. Instance-buffer grow/shrink counters
show whether the high-water allocation was later released after the 120-frame hysteresis window. Compare expected
placements with the bounded GPU-cache entry count and check that releases rise during inactive periods and uploads
rise on activation. Keep cycle captures separate from steady-state captures because asynchronous reload and upload
frames deliberately create bursts that are not representative of steady rendering.

## Surface acquisition and present-mode A/B experiment

The native `ribbon_render::GpuContext` surface now requests `mailbox` by default, with automatic fallback to FIFO
if Mailbox is unsupported. This is a provisional, target-machine-informed default, not a claim that Mailbox is best
on all GPUs or displays. `RIBBON_GPU_FRAME_LATENCY` defaults to `1`; `2` and `3` remain explicit options. The
separate `studs_cpu_renderer` GPU voxel surface was not part of these captures and retains its existing fixed latency.

- `RIBBON_GPU_PRESENT_MODE`: `mailbox` (default), `fifo`, `auto-vsync`, `auto-no-vsync`, `fifo-relaxed`, or `immediate`.
- `RIBBON_GPU_FRAME_LATENCY`: `1` (default), `2`, or `3`.

Unsupported explicit modes fall back to FIFO; `auto-vsync` resolves to `fifo-relaxed` when supported, and
`auto-no-vsync` resolves to `immediate`, then `mailbox`, then `fifo`, using the modes reported by the adapter.
`immediate` may tear. Each GPU context logs a `[GPU ADAPTER]` record (name, vendor/device IDs, device type, driver,
driver info, backend, surface compatibility and timestamp-query support). Each native surface also logs requested and
resolved present mode, available modes and selected frame-latency hint.

Run a repeated comparison that changes only FIFO vs Mailbox, alternates order to reduce order/thermal bias, and
writes the console logs alongside CSV/report files. Those logs preserve the exact adapter and surface setup used by
`wgpu` for each capture:

```powershell
.\tools\perf\run_present_mode_comparison.ps1 `
    -Runs 3 `
    -Frames 1050 `
    -Warmup 60 `
    -GridSide 32 `
    -MultiAssetCount 32 `
    -CachePrefetchCount 32 `
    -AssetMemoryBudgetMiB 1 `
    -CycleFrames 300 `
    -StressTextureSize 128 `
    -FrameLatency 1
```

The tool writes `present-mode-comparison-summary.md`, a manifest, per-capture reports and `.log` files under a new
`target/profiles/present-mode-comparison/<timestamp>/` directory. It checks the *resolved* mode and latency from
CSV rows; if requested Mailbox fell back to FIFO, the result is invalid as a direct mode comparison. The output
folder is collision-safe so a quick rerun will not silently overwrite previous captures.

### Target-machine result (2026-10-24)

One Windows Release run series compared three captures per mode using a 32x32 scene, 32 distinct active GLB assets,
32 unleased prefetch assets, a 1 MiB unpinned decoded CPU-cache budget, 300-frame lifecycle cycling, 60 warmup
frames and 991 retained samples per capture. The CSV confirmed FIFO/1 for all FIFO captures and Mailbox/1 for all
Mailbox captures. Across the three runs, the median run-level values were:

| Metric | FIFO | Mailbox | Mailbox vs FIFO |
|---|---:|---:|---:|
| Frame wall mean | 6.481 ms | 6.734 ms | 3.91% slower |
| Frame wall P95 | 13.982 ms | 7.092 ms | 49.28% lower |
| Frame wall P99 | 14.616 ms | 7.653 ms | 47.64% lower |
| Surface acquisition P95 | 13.314 ms | 6.885 ms | 48.29% lower |
| Render graph CPU P95 | 1.773 ms | 0.757 ms | 57.30% lower |
| Surface presentation P95 | 0.114 ms | 0.062 ms | 45.69% lower |
| GPU shadow + mesh P95 | 0.254 ms | 0.179 ms | 29.32% lower |

Frame-wall P95 was tightly clustered across repeats (FIFO range 13.828–14.139 ms; Mailbox range 7.080–7.180 ms). Mean
frame-wall time did not improve. These observations support better tail latency/pacing on the measured setup; they do
not prove that acquisition alone caused the full graph/GPU delta. CPU graph duration measures elapsed time around CPU
invocation/encoding and can reflect driver/backend backpressure within calls; it is not a CPU-utilization metric or a
per-pass GPU timing. Keep mode changes reversible and remeasure after driver, display or rendering changes.

The tested machine reported NVIDIA GeForce RTX 2050 (driver `32.0.15.8186`) and Intel UHD Graphics (driver
`32.0.101.7026`) in the Windows device list. The adapter actually selected by `wgpu` was not captured in those old
logs, so do not infer from that list which GPU ran the benchmark. New runs capture the selected adapter directly.

The historical FIFO/2 vs FIFO/1 and no-vsync results recorded in `VALIDATION_LOG_FOLLOWUP.md` remain separate
experiments, not part of the direct FIFO-vs-Mailbox baseline above. No-vsync is diagnostic only because `immediate` may tear and may use more power.

## Repeat the FIFO latency experiment

To compare FIFO queue latency separately from present mode, use the existing helper. It writes reports for each run
and a summary; do not mix this result with a FIFO-vs-Mailbox A/B:

```powershell
powershell -ExecutionPolicy Bypass -File tools/perf/run_surface_ab.ps1 -Runs 3
# Optional no-vsync diagnostic group (may tear):
powershell -ExecutionPolicy Bypass -File tools/perf/run_surface_ab.ps1 -Runs 3 -IncludeNoVSync
```

## Compare a current capture to a baseline

Save baseline files under distinct names before changing code. Generate a report with the optional comparison:

```powershell
python tools/perf/render_profile_report.py target/profiles/graphics-showcase-current.csv --compare target/profiles/graphics-showcase-baseline.csv --output target/profiles/graphics-showcase-comparison.md
```

The report displays mean, p50, p95, p99 and peak values for whole-frame wall duration, surface acquisition,
render-graph CPU, surface presentation, and available GPU timings. Older CSVs without phase columns remain
readable; their report omits phase rows. Pass-level CPU summaries are ordered by p95. Comparisons show the current-vs-baseline p95 delta;
negative duration deltas indicate lower latency, not proof of causation. For GPU timings, repeated polls of
the same asynchronous submission are deduplicated by `gpu_sample_sequence`.

Record the following alongside every baseline (in the Markdown report or a short adjacent note): commit or
source archive identifier, Windows version, GPU adapter and driver, display resolution, release/debug
profile, example command, warmup frames, captured frame range, and any changes to power settings or scene
content. The tool intentionally does not guess hardware details that the renderer does not expose.

## First target-machine capture interpretation (2026-10-22)

The initially supplied release captures reported whole-frame P95 values around 19.8–21.2 ms, while
render-graph CPU P95 was around 0.9–1.0 ms. The asynchronous shadow-plus-mesh GPU P95 was around
0.16–0.17 ms for `graphics_showcase` and 0.43–0.46 ms for `voxel_mesh_showcase`. These numbers do not
support choosing draw-call batching or shader changes as the first optimization. They show a large gap
between whole-frame wall duration and the measured render-graph/GPU work, but the original CSV schema did
not split out surface acquisition from presentation. The phase breakdown added after these captures is
intended to determine whether that gap is dominated by surface acquisition, presentation, or other render
setup before changing renderer behavior. New reports also show a per-frame remainder after subtracting those
phases and graph time; this is a diagnostic estimate of setup/other work plus timing overhead, not a named
CPU function or direct CPU-utilization measurement. Hardware/driver/resolution metadata was not supplied with these
captures, so treat these values as an initial observation, not a portable engine performance claim.

## Interpretation and limits

- Whole-frame wall duration includes surface acquisition, graph execution and presentation. The CSV separately
  records time spent in `Surface::get_current_texture` and `SurfaceTexture::present`, helping identify waits
  outside the render graph. These phase times are wall-clock elapsed durations and do not equal CPU utilization.
- Render graph pass values measure elapsed CPU-side invocation/encoding per graph node. Calls into the graphics
  backend may experience backpressure; such time can appear inside a pass or graph duration. These values are not
  CPU-utilization measurements and do not provide detailed GPU timing for every internal sub-pass.
- GPU timestamps are asynchronous. They currently cover the shadow-through-main-mesh interval and the main
  mesh pass, not the complete frame. The interval includes an optional spot shadow pass. Unsupported adapters
  may produce no GPU samples; delayed results can be assigned to earlier submission sequence numbers.
- Resource byte values are engine-side estimates. They are not driver-reported VRAM and may exclude allocator,
  tiling/compression, swap-chain, pipeline, bind-group and other backend overhead.
- The first measured frame still includes variance from asset arrival, shader/pipeline state and OS scheduling;
  compare distributions across multiple runs rather than trusting a single capture.
- Debug and release timings must not be compared as equivalent. Do not add a performance claim until the
  same workload has been captured before and after on target hardware.

## Static scene and asset scalability profile (2026-10-24)

This is a scaling investigation, not another before/after rendering optimization. It was run on the
current single-submit post-processing implementation at source revision
`b4626931a7253edf2d7882b5a5ecb2a785d7ef6a` (dirty working tree, due to the profiling instrumentation
being measured). All runs used the Release `graphics_showcase` executable, NVIDIA GeForce RTX 2050,
driver 581.86, Vulkan, 1280x800, desired frame latency 1, 600 frames with 60 warmup frames and 541
contiguous retained successful-frame rows. FIFO and Mailbox both resolved as requested. Every capture
recorded the executable hash, invocation, revision/dirty state, adapter, driver, resolution, mode and
workload in its adjacent log and capture manifest.

The static placement sweep used 64, 576 and 1,600 placements of one shared model/material, one sun,
fixed transforms and a paused camera. This isolates placement scaling more cleanly than cycling assets.
The second sweep held 576 placements fixed and changed the number of distinct active GLB model/material/
texture assets from 1 to 8 to 32. Those assets were generated at 128x128 texture size; model count,
material count, texture bytes and renderer draw groups therefore vary together. Both sweeps contain five
runs per size and presentation mode. Every run has 541 frame rows; capture reports were regenerated and
their mean, P50, P95 and P99 rows checked against the raw CSVs. Percentiles use linear interpolation at
`(n - 1) * p`.

### Placement count: median run P95 [minimum, maximum] (ms)

The brackets are variability across the five run-level P95 values; the full report includes each
workload/mode's mean, P50, P95 and P99 and asynchronous GPU sample counts.

| Mode | Placements | Frame wall | Surface acquisition | Render graph CPU | GPU mesh pass |
|---|---:|---:|---:|---:|---:|
| FIFO | 64 | 14.038 [13.921, 14.109] | 13.000 [12.842, 13.184] | 1.201 [1.144, 1.467] | 0.073 [0.069, 0.078] |
| FIFO | 576 | 14.120 [13.890, 14.162] | 12.839 [12.648, 12.982] | 1.659 [1.520, 1.815] | 0.099 [0.092, 0.102] |
| FIFO | 1,600 | 14.081 [13.816, 14.146] | 12.447 [12.342, 12.612] | 2.319 [1.766, 2.521] | 0.152 [0.148, 0.155] |
| Mailbox | 64 | 1.138 [1.060, 2.952] | 0.845 [0.698, 2.192] | 0.371 [0.352, 1.080] | 0.023 [0.022, 0.023] |
| Mailbox | 576 | 1.339 [0.966, 2.163] | 0.927 [0.568, 1.714] | 0.560 [0.439, 0.785] | 0.022 [0.020, 0.022] |
| Mailbox | 1,600 | 1.024 [0.818, 1.376] | 0.270 [0.241, 0.914] | 0.682 [0.540, 0.862] | 0.033 [0.030, 0.033] |

The GPU shadow-plus-mesh interval includes the time between its shadow and mesh timestamps; Bloom and
tone-mapping GPU timings are reported separately in the full capture report. In the static workload,
render-graph CPU P95 rises with placement count in both modes, while the GPU mesh P95 remains below
0.16 ms. FIFO frame P95 stays near 14 ms and surface acquisition alone is about 12.4–13.0 ms. That is
surface pacing/compositor wall time, not evidence that mesh rendering consumes that time. Mailbox frame
P95 and acquisition vary substantially across runs and are not monotonic in placement count.

The tracked allocation estimate for the one-model placement sweep rises from 25,298,384 B at 64
placements to 25,615,824 B at 1,600 placements; renderer instance-buffer capacity grows from 64 to
1,024. The material texture estimate remains 352 B and cumulative model uploads stay at one, as
expected for a single reused model/material.

### Unique asset count at 576 placements: median run P95 [minimum, maximum]

| Mode | Active assets | Frame wall (ms) | Surface acquisition (ms) | Render graph CPU (ms) | GPU mesh (ms) | Estimated total GPU allocations (MiB) | Estimated material textures (MiB) | Estimated decoded CPU assets (MiB) |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| FIFO | 1 | 14.146 [14.129, 14.613] | 12.795 [12.692, 12.836] | 2.000 [1.645, 2.507] | 0.065 [0.062, 0.068] | 24.35 | 0.08 | 0.06 |
| FIFO | 8 | 14.167 [13.960, 14.385] | 12.745 [12.508, 12.912] | 2.089 [1.511, 2.531] | 0.101 [0.098, 0.104] | 24.94 | 0.67 | 0.52 |
| FIFO | 32 | 14.242 [13.855, 14.577] | 12.716 [12.407, 12.836] | 2.208 [1.165, 2.526] | 0.108 [0.104, 0.111] | 26.97 | 2.67 | 2.07 |
| Mailbox | 1 | 1.363 [1.245, 2.164] | 0.834 [0.485, 1.788] | 0.746 [0.449, 0.832] | 0.019 [0.018, 0.019] | 24.35 | 0.08 | 0.06 |
| Mailbox | 8 | 1.383 [1.211, 2.122] | 0.796 [0.660, 1.743] | 0.683 [0.471, 1.042] | 0.026 [0.025, 0.026] | 24.94 | 0.67 | 0.52 |
| Mailbox | 32 | 1.526 [1.358, 2.180] | 0.717 [0.149, 1.050] | 0.955 [0.599, 1.871] | 0.027 [0.025, 0.027] | 26.97 | 2.67 | 2.07 |

Increasing active assets also grows draw groups and uploads from 1 to 32, and the estimated material
texture and decoded-asset footprints scale accordingly. The Mailbox graph P95 increase at 32 assets has
a wide run range, so this data points to CPU-side multi-group draw preparation/encoding as the next
place to profile, but does not yet isolate a specific function or establish an optimization win.

### What the captures establish and what they do not

- FIFO's high P95 frame duration is dominated by surface-acquisition pacing in this setup. Mailbox has
  lower frame-wall times, but its tails and acquisition vary between runs. Neither observation proves
  that the post-processing submission merge changed acquisition behavior.
- Increasing placements increases render-graph CPU time and GPU mesh time, but measured GPU mesh P95
  remains small relative to FIFO pacing and sub-millisecond Mailbox frame-wall P95/medians.
- Increasing unique assets increases draw groups, uploads and estimated resource use. The experiment
  changes geometry/material/texture count together and cannot attribute cost to texture memory alone.
- The async timestamp readbacks are nonblocking and retain the latest completed value per frame
  snapshot. Deduplication means GPU stage samples are lower than 541 frame rows (roughly 320–533 per
  stage/run here); the exact per-run counts are in `run-level-results.csv` and
  `asset-run-level-results.csv`. Zero samples were reported skipped by the timestamp rings, but a
  result can complete between snapshots and not appear as a distinct CSV sample.
- Frame rows represent successful presentations only. `SurfaceError::Timeout` is ignored and Lost /
  Outdated surfaces are reconfigured without a CSV error counter. Thus these captures cannot quantify
  dropped, timed-out or failed acquisition frames. The reported GPU allocation and CPU asset byte
  counts are estimates, not driver VRAM or process RSS.
- The renderer exposes an offscreen framebuffer readback API, but there is no existing repeatable
  baseline-vs-optimized pixel-capture runner for the native HDR/Bloom path. No pixel equivalence claim
  was tested here. Empty/no-camera and post-process image comparisons remain a follow-up correctness
  check rather than an inference from timing captures.
- The first moving-scene attempt is preserved at
  `profiles/render-scalability/20261024-131416/` and excluded: the previous static flag froze camera
  motion but not model/light animation. The final runs use the corrected flag that pauses those updates.

No rendering optimization is made from these measurements. The strongest next step is to split CPU
instance collection/culling and draw encoding/submission timing for the large multi-placement/multi-asset
cases. Keep presentation wait as its own investigation, and do not target GPU shaders unless a workload
shows GPU stage time becoming material.

Exact commands used from the repository root (capture scripts each build no source and log the
executable SHA-256, revision, command line, actual adapter/surface configuration and exit code):

```powershell
cargo fmt --all --check
cargo check --release -p graphics_showcase
cargo build --release -p graphics_showcase
python tools/assets/generate_asset_stress_corpus.py --count 64 --texture-size 128 --output-root target/asset-stress-corpus
& .\tools\perf\run_render_scalability_profile.ps1
python tools\perf\analyze_render_scalability.py profiles\render-scalability\20261024-133006\captures profiles\render-scalability\20261024-133006
& .\tools\perf\run_render_asset_scalability.ps1
python tools\perf\analyze_render_asset_scalability.py target\profiles\render-scalability-assets\20261024-175552\captures target\profiles\render-scalability-assets\20261024-175552
```

The raw static placement captures, logs and per-capture reports are retained locally under
`profiles/render-scalability/20261024-133006/`; the raw asset/texture set is under
`target/profiles/render-scalability-assets/20261024-175552/`. These generated profile directories are
ignored by Git so they do not pollute the source tree. The manifests, aggregate reports and run-level
CSVs are also checked in under `docs/performance/performance-results/render-scalability-20261024/` for review on
the branch; the raw CSV/log captures remain on the measurement machine.

## Validation boundary

`cargo fmt --all --check`, `cargo check --release -p graphics_showcase`, and
`cargo build --release -p graphics_showcase` succeeded on the target Windows machine. The capture validators
accepted all 30 static placement runs and all 30 unique-asset runs, each with 541 contiguous frame rows and
matching report statistics. No workspace-wide test or Clippy matrix was run. This is a scalability profile of
one implementation, not a baseline/current speedup claim; see the capture and measurement limits above before
selecting any optimization.

## CPU phase profiling and cache-resolution A/B (2026-10-24)

An opt-in CPU phase profiler and frame-associated asynchronous GPU timing records were added to support the
large-scene CPU investigation. CPU timings separate scene conversion, asset cache reconciliation, model
resolution/draw assembly, visibility/sorting, instance and uniform preparation, shadow/mesh/Bloom/tone-map
encoding, queue submissions, GPU query polling, and profile-recorder bookkeeping. These are CPU wall durations
for the named call scopes; they are not CPU utilization. GPU timestamp records are emitted separately, retain
their submission and observation frame IDs, and are not artificially repeated across frame rows.

The controlled A/B result supports one targeted CPU change: deduplicate model-handle lookups during extraction
for repeated placements of the same `(asset id, generation)`. For 576 and 1,600 placements, asset-cache
reconciliation median run-level P95 fell 68–73% in both FIFO and Mailbox, and the five-run ranges do not
overlap. This is a phase-level result. Overall frame and surface-acquisition timings do not show a consistent
end-to-end improvement; Mailbox wall timings are noisy and cannot be causally attributed to this CPU change.
No rendering output equivalence test was performed. Full setup, commands, per-phase/per-mode results,
run-to-run ranges, A/B executable hashes, and limitations are documented in
[`render-cpu-phase-ab-20261024.md`](performance-results/render-cpu-phase-ab-20261024.md). Its companion
[`render-cpu-phase-ab-runs-20261024.csv`](performance-results/render-cpu-phase-ab-runs-20261024.csv) records
per-workload run-level mean/P50/P95/P99 summaries and percent changes.

The preserved raw A/B captures and logs are in ignored local output directories:
`target/profiles/render-cpu-ab/baseline/20261024-183953/captures/` and
`target/profiles/render-cpu-ab/optimized/20261024-184601/captures/`. Each variant has 30 captures with 541
contiguous retained frame rows each; captured GPU timing records number 1,076–1,080 per run. All recorded
GPU dropped/unavailable and surface/render-graph error counters were zero. Pending GPU queries can remain at
capture boundaries because query results are polled asynchronously.
