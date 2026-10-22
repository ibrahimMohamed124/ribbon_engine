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
python tools/render_profile_report.py target/profiles/graphics-showcase.csv --output target/profiles/graphics-showcase.md
```

This scene contains moving built-in meshes, glTF models, a directional light/shadow pass, HDR tone mapping,
and frustum culling. It is the primary broad rendering baseline.

### Single glTF model workload

```powershell
cargo run --release -p model_viewer -- --frames 1800 --profile-warmup 120 --profile-csv target/profiles/model-viewer.csv
python tools/render_profile_report.py target/profiles/model-viewer.csv --output target/profiles/model-viewer.md
```

Use the bundled checker cube unless the exact same external model is supplied to both baseline and current
runs. This scenario is useful for the steady-state mesh/material pipeline after initial asynchronous loading.

### Dynamic voxel chunk workload

```powershell
cargo run --release -p voxel_mesh_showcase -- --frames 1800 --profile-warmup 240 --profile-csv target/profiles/voxel-mesh-showcase.csv
python tools/render_profile_report.py target/profiles/voxel-mesh-showcase.csv --output target/profiles/voxel-mesh-showcase.md
```

This example removes and restores a boundary voxel and exercises dirty-chunk remeshing, mesh replacement,
and transient render targets. A 240-frame warmup deliberately excludes the initial scene setup and the
built-in edits at frames 120 and 180; use a separate run with a shorter warmup if those bursts themselves
are the subject of the measurement.

### Native first-person scene

```powershell
cargo run --release -p horror_slice -- --frames 1800 --profile-warmup 120 --profile-csv target/profiles/horror-slice.csv
python tools/render_profile_report.py target/profiles/horror-slice.csv --output target/profiles/horror-slice.md
```

Without scripted input, this captures the loaded room in its default state. Treat movement and mouse-look
runs as a separate scenario and describe the interaction sequence in the report metadata.

## Surface acquisition A/B experiment

The first Windows release capture showed surface-acquire P95 close to whole-frame P95. The primary `engine_render::GpuContext` surface keeps `fifo` as its default present mode. After the first controlled
Windows captures, its default desired frame latency is `1`: compared with FIFO/2, FIFO/1 reduced frame-wall P95 from 21.359 ms to
14.069 ms and surface-acquire P95 from 20.272 ms to 13.269 ms in one 1,681-frame capture per setting. The mean
frame time remained about 6.7 ms, while the median rose from 1.194 ms to 6.131 ms; this indicates less bursty
frame pacing and a shorter tail, not higher average throughput. Repeat captures across sessions and adapters
before treating this as universal. The frame-latency setting remains overridable for controlled experiments. This override is read by `engine_render::GpuContext`; the separate `plugin_rendering` GPU voxel surface remains at its existing fixed latency of `2` because it was not part of this capture:

- `RIBBON_GPU_PRESENT_MODE`: `fifo` (default), `auto-vsync`, `auto-no-vsync`, `fifo-relaxed`, `immediate`, or `mailbox`.
- `RIBBON_GPU_FRAME_LATENCY`: `1` (default), `2`, or `3`.

Unsupported explicit modes fall back to `fifo`; `auto-vsync` resolves to `fifo-relaxed` when supported, and
`auto-no-vsync` resolves to `immediate`, then `mailbox`, then `fifo`, using the modes reported by the adapter.
`immediate` may tear. The application logs both the requested setting and the resolved mode. CSV and Markdown
reports retain the resolved mode and requested frame-latency hint so comparisons cannot silently mix settings.

Run these three captures separately, keeping scene, window size, release profile, warmup and frame count the
same. Change only one variable between runs:

```powershell
$env:RIBBON_GPU_PRESENT_MODE = "fifo"
$env:RIBBON_GPU_FRAME_LATENCY = "2"
cargo run --release -p graphics_showcase -- --frames 1800 --profile-warmup 120 --profile-csv target/profiles/graphics-fifo-latency2.csv

$env:RIBBON_GPU_PRESENT_MODE = "fifo"
$env:RIBBON_GPU_FRAME_LATENCY = "1"
cargo run --release -p graphics_showcase -- --frames 1800 --profile-warmup 120 --profile-csv target/profiles/graphics-fifo-latency1.csv

$env:RIBBON_GPU_PRESENT_MODE = "auto-no-vsync"
$env:RIBBON_GPU_FRAME_LATENCY = "2"
cargo run --release -p graphics_showcase -- --frames 1800 --profile-warmup 120 --profile-csv target/profiles/graphics-no-vsync-latency2.csv

python tools/render_profile_report.py target/profiles/graphics-fifo-latency1.csv --compare target/profiles/graphics-fifo-latency2.csv --output target/profiles/graphics-latency1-comparison.md
python tools/render_profile_report.py target/profiles/graphics-no-vsync-latency2.csv --compare target/profiles/graphics-fifo-latency2.csv --output target/profiles/graphics-no-vsync-comparison.md
Remove-Item Env:RIBBON_GPU_PRESENT_MODE
Remove-Item Env:RIBBON_GPU_FRAME_LATENCY
```

The no-vsync run is diagnostic, not the production default. In the submitted capture it resolved to `immediate` and lowered P95 substantially, but it can tear and may increase power use. If `auto-no-vsync` resolves
to `fifo` on the target adapter, it does not isolate a no-vsync path; the resolved mode shown in the report must
be inspected. A lower surface-acquire wait must also be weighed against tearing, latency, frame pacing and power
use. The FIFO/1 default is a measured provisional choice based on the observed tail-latency reduction; use the repeat-capture tool described below to check reproducibility. Keep FIFO/2 available when its more eager queueing is preferred.

## Repeat the surface-latency experiment

Run at least three captures for each FIFO latency setting using the helper below. It writes each run to a
new timestamped directory, produces a report per CSV, and summarizes median/min/max run-level P95 values.
An optional switch adds the no-vsync mode as a separate diagnostic group. Do not interpret no-vsync as a fair
VSync performance comparison, and inspect the resolved present mode recorded in each capture.

```powershell
powershell -ExecutionPolicy Bypass -File tools/run_surface_ab.ps1 -Runs 3
# Optional diagnostic group (may tear):
powershell -ExecutionPolicy Bypass -File tools/run_surface_ab.ps1 -Runs 3 -IncludeNoVSync
```

## Compare a current capture to a baseline

Save baseline files under distinct names before changing code. Generate a report with the optional comparison:

```powershell
python tools/render_profile_report.py target/profiles/graphics-showcase-current.csv --compare target/profiles/graphics-showcase-baseline.csv --output target/profiles/graphics-showcase-comparison.md
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
- Render graph pass values measure CPU-side invocation/encoding per graph node. They do not provide detailed
  timings for all internal GPU sub-passes.
- GPU timestamps are asynchronous. They currently cover the shadow-through-main-mesh interval and the main
  mesh pass, not the complete frame. The interval includes an optional spot shadow pass. Unsupported adapters
  may produce no GPU samples; delayed results can be assigned to earlier submission sequence numbers.
- Resource byte values are engine-side estimates. They are not driver-reported VRAM and may exclude allocator,
  tiling/compression, swap-chain, pipeline, bind-group and other backend overhead.
- The first measured frame still includes variance from asset arrival, shader/pipeline state and OS scheduling;
  compare distributions across multiple runs rather than trusting a single capture.
- Debug and release timings must not be compared as equivalent. Do not add a performance claim until the
  same workload has been captured before and after on target hardware.

## Validation boundary

No new tests were added for this milestone and no Cargo build/test command was run during its implementation.
Run the workspace format, Clippy, default test and `--no-default-features` test commands on the target
Windows checkout before accepting this capture workflow. Then collect a baseline and a current capture before
choosing the first rendering optimization.
