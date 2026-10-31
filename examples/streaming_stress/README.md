# Ribbon Engine Streaming Stress Harness

A reusable engine example for exercising Ribbon's actual AssetServer, StreamingCoordinator,
Rapier integration, renderer-owned GPU warmup and upload scheduler. It is a diagnostic harness,
not a game, and does not require creating game-specific gameplay code.

## Run the full test

From the repository root in PowerShell:

```powershell
.\examples\streaming_stress\run_streaming_stress.ps1
```

The script:

1. Builds `ribbon_streaming_stress` in release mode.
2. Runs a headless phase against the real AssetServer, streaming coordinator, ECS and Rapier.
3. Generates a deterministic GLB corpus under `target/streaming-stress-corpus`.
4. Opens a native GPU render window and runs the real GLB/model warmup and standalone texture/sampler upload stress phase.
5. Samples the directly tracked child process working set and private bytes from PowerShell every 25 ms by default (`-MemorySampleIntervalMs` can change the interval).
6. Writes per-frame CSV, summaries and process-memory CSV files under a timestamped folder in `target/streaming-stress-runs`.

The GPU window is automated: the observer and camera move along a generated forward/backward route.
Keep the window open until the harness closes it. Do not minimize it during the run because some
window/render loops throttle redraw while minimized.

## Headless-only run

For machines or CI workers without a graphics adapter:

```powershell
.\examples\streaming_stress\run_streaming_stress.ps1 -HeadlessOnly
```

This still tests region churn, CPU asset loading, active worker cancellation, shared asset leases,
logical-scene shared region ownership, region-triggered cancellation of an in-flight SceneDoc read, CPU residency pressure, and update/activation timings.
It intentionally reports GPU metrics as unavailable; it never fabricates a successful GPU result. The phase also activates four overlapping authored regions together, holds them active, and then verifies their unload.

## Adjust stress intensity

```powershell
.\examples\streaming_stress\run_streaming_stress.ps1 `
  -Regions 96 `
  -Cycles 3 `
  -Assets 128 `
  -AssetBytesKiB 512 `
  -TextureSize 512 `
  -FramesPerRegion 30 `
  -TailFrames 180 `
  -UploadsPerFrame 2 `
  -UploadBudgetMiB 2
```

`-TailFrames` must be at least 48 so the test has time to reconcile physics cleanup and region unloading after the route. `-SkipBuild` reuses the existing release executable. `-GpuTimeoutMinutes` bounds the GPU phase and
terminates it if the window stops making progress. The PowerShell runner starts Cargo, Python, and
the harness as tracked child processes, captures stdout/stderr separately, and records working-set
and private-byte samples for each process.

## What is exercised

| Requested scenario | Harness path |
|---|---|
| Repeated load/activate/deactivate/unload of many regions | Deterministic observer traverses generated collider regions forward and backward; final out-of-range phase verifies unload. |
| CPU/GPU memory while traversing | Per-update AssetServer residency, pinned/unpinned bytes, renderer GPU allocation estimates, plus OS process working set/private bytes. |
| GPU upload budgets under load | Many distinct GLB model uploads plus standalone texture and sampler warmups, measured against the configured count and byte budget each rendered frame. |
| Cancellation while work is active | A dedicated AssetSource first holds a real AssetServer worker in cancellable I/O and validates direct cancellation; a second case starts a required `.scenedoc` load through a real region, moves the observer beyond unload distance while the worker is blocked, and verifies region teardown releases the lease and cancels that active read. |
| Shared resources across regions/consumers | Every GPU route/overlap region references a common model plus unique models; CPU phase verifies duplicate `AssetRef` requests, lease retention and logical region references from two scenes. Generic GPU duplicate keys are checked for one queued upload. |
| Active-region concurrency | Both phases explicitly activate four overlapping regions at the same time, hold the four-region state for a configured interval, record activation latency/peak concurrency, and fail if the cap is exceeded. |
| Frame time and streaming latency | CSV captures App update wall time, renderer CPU frame time, StreamingCoordinator CPU time, region activation latency percentiles, per-poll AssetServer source-read/decode timing and asynchronous GPU mesh-pass timestamps when supported. |

## Output layout

A timestamped run creates:

```text
target/streaming-stress-runs/<timestamp>/
  run-summary.md
  build/                         # Release build logs and process-memory.csv (unless -SkipBuild)
  corpus-generator/              # GLB generation logs and process-memory.csv (GPU run only)
  headless/
    headless-summary.md
    headless-frames.csv
    process-memory.csv
    headless.stdout.log
    headless.stderr.log
  gpu/
    gpu-summary.md
    gpu-frames.csv
    render-profile.csv
    process-memory.csv
    gpu.stdout.log
    gpu.stderr.log
```

## Measurement semantics and limits

- AssetServer CPU figures are decoded-asset residency estimates and lease/pin accounting; they are not the process's complete heap usage. The PowerShell sampler records process working set and private bytes as a separate OS-level view.
- GPU allocation bytes are the renderer's tracked estimates, not exact physical VRAM reported by the graphics driver. The harness does not issue GPU readbacks to infer memory size.
- `last_gpu_mesh_pass_ns` is an asynchronous completed timestamp sample. It may refer to an earlier submitted frame; the CSV preserves that distinction rather than pretending every row has a same-frame GPU duration.
- GLB image mip chains are prepared by the AssetServer decode worker. The render thread uploads the prepared levels and does not run CPU mip filtering for these model images. Standalone texture callers that do not provide prepared levels retain the compatibility mip-generation path. Decoded CPU residency includes the retained prepared mip chains (roughly one-third of each used texture per transfer-function role); this is accounted by `ModelAsset::estimated_size_bytes`.
- Generic standalone texture/sampler leases are dropped after their warmup requests reach terminal states. The harness continues rendering until the renderer's orphan collection records releases and live standalone texture/sampler counts return to zero; it does not infer release merely from dropping a Rust lease.
- The PowerShell `process-memory.csv` sampler observes the direct child process only, not all descendant processes or total system/GPU memory. A 25 ms default interval is intended to collect multiple samples during the short headless run.
- GPU timing is only available when the actual adapter/backend exposes timestamp queries.
- The upload budget permits one oversized first upload in a frame to avoid starvation. The acceptance check allows that documented exception, but verifies subsequent work does not exceed the configured budget.
- The GPU phase uses real renderer work. A headless run never marks GPU warmup or GPU memory as successful.
- These tests are stress/regression signals, not a substitute for profiling a shipping game's actual content on its target devices.

## Direct executable usage

The script is the recommended entry point. After generating the GLB corpus, the phases can also be
run directly:

```powershell
cargo run --release -p ribbon_streaming_stress -- --mode headless --regions 48 --cycles 2
cargo run --release -p ribbon_streaming_stress -- --mode gpu --regions 48 --cycles 2 --asset-root target/streaming-stress-corpus --require-gpu
```
