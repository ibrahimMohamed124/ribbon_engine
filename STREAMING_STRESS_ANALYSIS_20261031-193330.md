# Ribbon Engine — Streaming Stress Analysis

Run ID: `20261031-193330`  
Hardware: NVIDIA GeForce RTX 2050, Vulkan, 1280×720, Mailbox, timestamp queries enabled  
Configuration: 48 route regions, 2 cycles, 64 generated GLB assets, 512px textures, 2 uploads / 2 MiB per frame, 25 ms requested process-memory sampling interval.

## Executive verdict

- **Functional stress result: PASS.** Build, Headless, corpus generation, and real-GPU phases exited with code 0.
- **The Mipmap preparation change delivered a major measured improvement.** The renderer's `model GPU resource creation/submission` phase fell from p50 16.591 ms / p95 17.272 ms / max 18.727 ms in the preceding run to p50 0.560 ms / p95 0.637 ms / max 0.692 ms in this run (about 96.6% lower at p50 and 96.3% lower at p95).
- **Frame-time performance improved substantially.** Profiled render-frame CPU p95 fell from 17.092 ms to 7.472 ms (56.3% lower); render-graph CPU p95 fell from 16.628 ms to 0.872 ms (94.8% lower).
- **Generic GPU-resource release is now demonstrated.** 9/9 generic resources released, with 0 standalone textures, 0 samplers, and 0 standalone-texture estimated bytes at the end. Model GPU uploads/releases balanced at 191/191 and the model GPU cache ended at 0 entries.
- **There is still a latency outlier to investigate.** GPU activation latency p99 is 1,078.966 ms even though p95 is 62.679 ms. The report does not identify the particular region/phase responsible.
- **CPU decoded assets remain in the AssetServer cache at the end.** 85.52 MiB / 64 assets are unpinned, with no budget pressure (configured budget 512 MiB). This is consistent with cache retention, not proof of a lease leak; the harness does not prove CPU cache eviction in the GPU phase.

## Phase outcomes

| Phase | Result | Duration | Peak working set | Peak private bytes |
|---|---:|---:|---:|---:|
| Release build | PASS | 113.23 s | 10.36 MiB | 2.90 MiB |
| Headless | PASS | 0.18 s | 30.52 MiB | 24.46 MiB |
| Corpus generation | PASS | 4.58 s | 8.76 MiB | 1.86 MiB |
| Real GPU | PASS | 18.55 s | 425.47 MiB | 533.26 MiB |

The build memory row measures the process monitored by the runner (Cargo), not the aggregate memory of Cargo plus all `rustc` child processes. The Headless phase has only 4 process-memory samples spanning about 91 ms, so its reported peak is not a robust full-phase peak. The GPU phase has 555 samples spanning about 18.36 s.

## Headless findings

| Metric | Result |
|---|---:|
| Base regions / cycles | 48 / 2 |
| Successful activations | 95/95 |
| Final unloaded regions | 54 |
| Peak active regions | 4/4 |
| Four-region cluster held | 8/8 frames |
| Region transitions | 907 |
| Invalid state transitions | 0 |
| Active worker cancellation observed | 1 |
| Active SceneDoc read cancelled by region unload | true |
| CPU decoded-residency peak | 16.25 MiB |
| p50 / p95 / p99 App update | 0.008 / 0.048 / 0.052 ms |
| p50 / p95 / p99 activation latency | 0.282 / 0.303 / 0.369 ms |
| Four-region cluster activation latency | 0.855 ms |

The earlier `96/95` activation counter is corrected to `95/95`. The four-region concurrency scenario is now exercised in Headless. The final-unloaded count exceeds the 48 route regions because the harness also creates auxiliary stress regions for its additional scenarios; this is a test accounting detail, not a failed route activation.

## GPU and upload findings

| Metric | Result |
|---|---:|
| Rendered frames | 2,473 |
| Profiled render-frame samples | 2,458 (frames 16–2,473) |
| Model GPU uploads / releases | 191 / 191 |
| Generic GPU uploads / releases | 9 / 9 |
| Generic resources ready / failed | 9 / 0 |
| Standalone textures / samplers after release | 0 / 0 |
| Model GPU cache entries at end | 0 |
| Upload operations in one frame, maximum | 2 |
| Upload bytes in one frame, maximum | 1,399,268 bytes |
| Configured byte budget | 2,097,152 bytes |
| Budget violations | 0 |
| Peak active regions / concurrent cluster | 4/4 |
| Concurrent cluster held | 42 frames (minimum requested: 24) |
| Final unloaded regions / failed regions | 52 / 0 |
| GPU timestamp samples completed / skipped | 1,870 / 0 |
| Surface / render-graph errors | 0 / 0 |

There were 200 upload operations total (191 model + 9 generic), across 199 frames with uploads. The maximum frame used 2 operations and 1,399,268 bytes, about 66.7% of the 2 MiB budget. The cumulative sum of the per-frame upload-byte counter is about 265.55 MiB over the entire run; this is total traffic across frames, not simultaneous residency.

The first 3 model-upload frames (9–11) occur before the detailed render profiler starts at frame 16. Therefore the detailed `model GPU resource creation/submission` phase contains 188 samples while the cumulative model-upload counter reports 191 operations. The 3-operation difference is explained by the unprofiled startup frames.

## Before/after performance comparison

The previous run (`20261031-180003`) used the same main 48-region / 2-cycle / 64-asset / 512px / 2-op / 2-MiB setup. This run adds the explicit 4-region concurrency check, generic resource release verification, and tighter memory sampling. The comparison is therefore useful but not a perfectly identical workload.

| Metric | Previous run | Current run | Change |
|---|---:|---:|---:|
| Model GPU creation p50 | 16.591 ms | 0.560 ms | 96.6% lower |
| Model GPU creation p95 | 17.272 ms | 0.637 ms | 96.3% lower |
| Model GPU creation max | 18.727 ms | 0.692 ms | 96.3% lower |
| Profiled frame CPU p95 | 17.092 ms | 7.472 ms | 56.3% lower |
| Render graph CPU p95 | 16.628 ms | 0.872 ms | 94.8% lower |
| Surface acquire p95 | 6.689 ms | 6.711 ms | Essentially unchanged |
| GPU generic releases | 0 | 9/9 | Verified |
| Live generic textures / samplers after release | 8 / 1 | 0 / 0 | Drained |
| Headless activations | 96/95 | 95/95 | Counter corrected |
| Peak active-region concurrency | 1/4 | 4/4 | Scenario now exercised |

## Residual performance observations

1. **One startup outlier:** `gpu-frames.csv` frame 1 has App update wall time 983.464 ms, while render-frame CPU is 25.187 ms and streaming coordinator CPU is 0.212 ms. The detailed renderer profiler starts at frame 16, so this cold-start frame is not fully attributed. It should be reported separately from steady-state frame percentiles; it must not be silently discarded from the max statistic.
2. **Steady-state frame time is now healthy for a 60-FPS target:** detailed frame CPU p95 / p99 is 7.472 / 8.132 ms, below 16.67 ms. The maximum profiled frame is 10.219 ms (frames 16–2,473 only).
3. **Surface acquisition dominates the profiled CPU frame:** surface-acquire p95 is 6.711 ms, compared with render-graph CPU p95 0.872 ms and present p95 0.046 ms. This likely includes frame pacing / surface synchronization (Mailbox with maximum frame latency 1); it is not evidence that the render graph itself is slow.
4. **GPU mesh pass is small:** GPU mesh-pass timestamp p95 is 0.048 ms. Timestamp results are asynchronous and not perfectly frame-aligned, as documented by the harness.
5. **Region activation tail remains large:** p50 / p95 / p99 is 61.819 / 62.679 / 1,078.966 ms for 95 successful samples. p95 suggests the common case is around 62 ms; p99 is dominated by an extreme tail (with only 95 samples it is near the upper end of the sample set). Add per-region latency records and phase breakdown before assigning a cause.
6. **Asset worker timings:** 49 non-zero worker-poll samples were recorded. Source-read aggregate p50 / p95 / max is approximately 7.712 / 15.758 / 29.001 ms; decode aggregate is 2.513 / 5.419 / 11.105 ms. These are per-poll aggregates, not per-file latency distributions.

## Memory and residency

| Metric | Result |
|---|---:|
| Peak process working set | 425.47 MiB |
| Peak process private bytes | 533.26 MiB |
| Final sampled working set / private bytes | 287.15 / 369.98 MiB |
| Peak AssetServer decoded CPU residency | 85.52 MiB |
| Final AssetServer decoded CPU residency | 85.52 MiB |
| Final CPU pinned bytes / pinned count | 0 / 0 |
| Final CPU unpinned bytes / loaded count | 85.52 MiB / 64 |
| CPU cache budget | 512 MiB |
| CPU budget evictions in GPU run | 0 |
| Peak renderer GPU allocation estimate | 39.84 MiB |
| Final renderer GPU allocation estimate | 22.07 MiB |
| Peak / final standalone texture estimate | 10.67 MiB / 0 |

All GPU streamed models and generic standalone resources are released. The remaining 85.52 MiB CPU residency is unpinned decoded-asset cache content under a 512 MiB cache budget, so it is retained by policy. It is not a live region lease. If a test is meant to prove CPU cache eviction after streaming, add a separate GPU-phase memory-pressure step with a deliberately low CPU cache budget and assert that unpinned assets are evicted while pinned assets remain valid.

The renderer estimate decreases from 39.84 MiB peak to 22.07 MiB at the end. The remaining estimate includes renderer/window persistent resources; `live_meshes=1` and `live_materials=1` remain, while model GPU cache entries are 0 and model upload/release counts balance. Confirm whether those single live mesh/material entries are intended built-ins before treating them as retained stream assets. These are estimates, not driver-reported physical VRAM.

Process-memory sampling was requested at 25 ms but actual GPU sample spacing has a median of about 31.3 ms and p95 about 46.8 ms. The Headless phase has too few samples for a strong OS memory peak. For short phases, add an explicit minimum sampling window or report sample count and coverage (as done here).

## Recommended next steps

1. Keep the worker-side Mipmap preparation change: the measured Model GPU creation phase is now sub-millisecond at p95 and is no longer the bottleneck.
2. Add per-region activation latency CSV rows with region ID and timestamps for request, CPU-ready, GPU-ready, physics-ready, Ready, and Active. This is necessary to root-cause the 1.079 s p99 outlier.
3. Add an explicit CPU-cache pressure/eviction phase to the GPU test after all region leases are released. Verify unpinned asset eviction and pinned-asset protection separately.
4. Extend startup instrumentation to capture frames 1–15 and report cold-start separately from steady state.
5. Treat surface-acquire time as frame pacing/synchronization until a controlled FIFO/Mailbox comparison proves otherwise; do not optimize render-graph code based on this run because graph p95 is already under 1 ms.
6. Run a longer soak (e.g. 10–20 cycles) and compare memory baselines after each full cleanup, not just one traversal. This is the next meaningful check for cumulative leaks.

## Scope and limitations

This report analyzes the uploaded run artifacts. It does not rerun the harness, compile the source, query actual VRAM through the NVIDIA driver, or establish long-term soak stability. Functional PASS applies to the exact configured stress scenario and adapter shown above.
