# FIFO vs Mailbox: Controlled Surface-Pacing Comparison

## Goal

The recent multi-asset lifecycle profile showed that the renderer's large model-upload spike was reduced by limiting first-use GPU model uploads to two distinct model cache entries per frame. In the latest focused summary, frame-wall peak was 15.991 ms while surface-acquisition P95 was 13.240 ms and GPU shadow-plus-mesh P95 was 0.254 ms. That suggests surface acquisition is worth investigating separately from CPU asset upload pacing, but it does not prove that FIFO is worse than Mailbox.

This workflow compares `fifo` and `mailbox` on the same multi-asset lifecycle workload, with the same requested frame-latency hint and three repeated captures per mode. The chosen mode must be read from the profile CSV, because unsupported requested modes fall back to FIFO. The target-side repeated comparison is recorded below and now informs the native `ribbon_render::GpuContext` default: prefer Mailbox when supported; fall back to FIFO when it is unavailable. FIFO remains an explicit environment override.

## Run the comparison

From the repository root in Windows PowerShell:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\run_present_mode_comparison.ps1 `
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

The script generates the deterministic GLB corpus once, then alternates capture order between runs to reduce systematic bias from warm-up, clocks or background activity. Each mode uses the same active assets, prefetch assets, grid, CPU-cache budget, lifecycle cycle, total frame count, warmup count and requested maximum frame latency.

Each run writes a CSV plus its individual Markdown profile report under:

```text
target/profiles/present-mode-comparison/<timestamp>/
```

The focused comparison result is `present-mode-comparison-summary.md`. The run manifest is `present-mode-comparison-manifest.md`.

## What the comparison reports

- Per-capture resolved present mode and frame-latency hint.
- Median/minimum/maximum run-level P95 and P99 frame-wall timings.
- Surface-acquisition, render-graph CPU, presentation and asynchronously sampled GPU timings.
- Share of captured frame samples above the 16.667 ms reference budget.
- A direct Mailbox-versus-FIFO delta table, based on each run's metric rather than pooling samples from distinct processes.
- A clear invalid-comparison warning if requested Mailbox resolves to FIFO or if the two groups use different latency hints.

The comparison uses three runs as a starting point, not a universal benchmark guarantee. Inspect repeatability ranges as well as median deltas. If the Mailbox request resolves to FIFO, do not interpret the two groups as distinct presentation modes. The current Windows target run resolved correctly to FIFO and Mailbox in all six captures, so it provides a valid machine-specific comparison.

## Controls for a meaningful comparison

Keep the window/display resolution, GPU adapter/driver, power mode, asset corpus, `--release` build, frame count and warmup stable. Avoid moving/resizing the window or running unrelated GPU-heavy workloads between captures. Do not change frame latency along with present mode; that would confound the comparison.

The supplied target-side run measured the following run-level medians (three captures per mode, 991 retained frame samples per capture; frames were not pooled):

| Metric | FIFO median | Mailbox median | Mailbox delta |
|---|---:|---:|---:|
| Frame-wall mean | 6.583 ms | 6.552 ms | -0.47% |
| Frame-wall P95 | 14.666 ms | 8.866 ms | -39.55% |
| Frame-wall P99 | 16.489 ms | 13.059 ms | -20.80% |
| Frame-wall peak | 33.010 ms | 17.774 ms | -46.15% |
| Surface-acquisition P95 | 13.796 ms | 8.340 ms | -39.54% |
| Render-graph CPU P95 | 1.667 ms | 1.042 ms | -37.45% |
| Samples above 16.667 ms | 0.91% | 0.20% | -77.78% |

The resolved modes were `fifo` and `mailbox` as requested, and all captures recorded desired maximum frame latency `1`; retained sample counts matched. Mean frame time was nearly unchanged while tail latency and acquisition waits improved. Mailbox's run-level peak range was still 17.436-18.002 ms, slightly above the 16.667 ms 60-FPS reference, so the result does not guarantee every frame stays within budget. Based on these repeat captures, the native `ribbon_render::GpuContext` now prefers Mailbox when supported and falls back to FIFO when it is unavailable. `RIBBON_GPU_PRESENT_MODE=fifo` keeps FIFO available for compatibility or further A/B captures.

These results are specific to the supplied machine, adapter, driver, display, build and workload. Before shipping on other targets, check visual frame pacing, perceived input latency and power behavior; do not infer a universal improvement from one system's capture set.

## Local summary helper

To create a smaller report for any one CSV without uploading the CSV file, use:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\summarize_render_profile.ps1 `
    -CsvPath "D:\path\to\capture.csv"
```

This writes a `-summary.md` next to the capture CSV. The summary helper checks sampled cumulative upload-counter deltas, but it does not infer uploads that happened before the first captured row.

## Validation boundary

The initial comparison workflow and report logic were reviewed statically. The six FIFO/Mailbox captures above were run by the user on the target Windows system and supplied as a report. The authoring environment did not run a Rust/Cargo command, the engine runtime, PowerShell scripts or the test suite for this default adjustment; rebuild and repeat the visual/runtime checks on each target adapter before release.

## Existing FIFO/latency-1 reference capture

Before the repeated A/B set, the user supplied one FIFO/latency-1 multi-asset lifecycle capture with the following summary. Treat it as a reference only, not as a statistically matched baseline for the Mailbox results:

| Metric | FIFO/1 reference |
|---|---:|
| Frame-wall mean | 6.265 ms |
| Frame-wall P95 / P99 / peak | 14.065 / 14.651 / 15.991 ms |
| Surface-acquisition P95 | 13.240 ms |
| Render-graph CPU P95 | 2.211 ms |
| GPU shadow + mesh P95 | 0.254 ms |
| Tracked GPU/renderer resource peak estimate | 26.97 MiB |
| Unpinned CPU-cache estimate / budget | 1,018,800 B / 1,048,576 B |

The repeated FIFO captures generated by the comparison script should be used as the true comparison baseline; environmental differences or run-to-run variation can make them differ from this earlier single capture.

## Verified run and benchmark-runner fix (2026-10-24)

The runner was exercised on Windows PowerShell 7.4.21 with the workload above (3 runs per mode, 1,050 frames, 60 warmup frames, 32 × 32 grid, 32 multi-asset placements, 32 prefetches, 1 MiB CPU-cache budget, 300-frame cycle, 128-pixel textures and latency 1). The six captures are in `target/profiles/present-mode-comparison/20261024-120033`; the full logs, CSV files and reports are retained there. They record an NVIDIA GeForce RTX 2050, NVIDIA driver 581.86, Vulkan and a 1280 × 800 surface. Each capture retained the expected 991 post-warmup frame rows and resolved to its requested mode.

| Metric | FIFO median [min, max] | Mailbox median [min, max] | Mailbox delta |
|---|---:|---:|---:|
| Frame-wall mean | 6.563 [6.478, 6.628] ms | 0.583 [0.580, 0.606] ms | -91.11% |
| Frame-wall P50 | 5.815 [5.419, 6.049] ms | 0.439 [0.433, 0.448] ms | -92.46% |
| Frame-wall P95 | 13.983 [13.876, 14.144] ms | 1.661 [1.514, 1.845] ms | -88.12% |
| Frame-wall P99 | 14.619 [14.482, 14.712] ms | 2.927 [2.707, 2.958] ms | -79.98% |
| Surface-acquisition P50 | 4.762 [4.167, 5.404] ms | 0.179 [0.157, 0.182] ms | -96.23% |
| Surface-acquisition P95 | 13.116 [13.060, 13.212] ms | 0.889 [0.805, 0.988] ms | -93.22% |
| Surface-acquisition P99 | 13.556 [13.304, 13.601] ms | 1.979 [1.926, 2.159] ms | -85.40% |
| Render-graph CPU P50 | 0.670 [0.554, 0.871] ms | 0.338 [0.335, 0.341] ms | -49.52% |
| Render-graph CPU P95 | 2.080 [1.692, 2.209] ms | 0.576 [0.571, 0.606] ms | -72.30% |
| Render-graph CPU P99 | 2.861 [2.773, 3.924] ms | 2.811 [2.617, 2.824] ms | -1.74% |
| GPU shadow + mesh P95 (async timestamps) | 0.312 [0.311, 0.317] ms | 0.074 [0.072, 0.075] ms | -76.36% |

The acquisition P95 is close to a 60 Hz frame interval under FIFO and far below it under Mailbox; presentation P95 itself is 0.150 ms vs 0.058 ms. This supports surface pacing/back-pressure as the largest measured FIFO tail component on this machine. It does not prove the driver or compositor is independently at fault. GPU timestamp values come from asynchronous timestamp queries; graph CPU is elapsed CPU-side node time that may include backend/queue calls. The very low Mailbox mean reflects faster CPU-side frame production with latency 1 and is not evidence that GPU throughput improved by the same amount. The earlier `20261024-115237` Mailbox set was much less repeatable (P95 ranged 0.777–6.952 ms); this rerun's narrower 1.514–1.845 ms range shows why one capture set should not be treated as universal.

The original failure directory `20261024-115028` contains no CSV or log artifacts, so there is no capture to recover from it. The failed attempt stopped at build output, and its visible `Compiling ribbon_plugin_render3d` line did not show a compiler diagnostic. The later `20261024-115237` logs show the exact PowerShell issue: Cargo's ordinary stderr progress was converted into `NativeCommandError` records at the `Tee-Object` pipeline while `$ErrorActionPreference` was `Stop`. Cargo ultimately completed successfully there, but the logs were polluted with formatted error-record metadata.

The runner now converts merged output to plain text, streams it to the console and appends it to the capture-specific log. It reads `$LASTEXITCODE` immediately after Cargo, throws with the exact log path on a real failure, and checks that each report exists and the CSV has the expected post-warmup row count before the comparison can succeed. A PowerShell 7.4.21 probe confirmed ordinary stderr plus exit 0 succeeds, while stderr plus exit 7 preserves exit 7 and the diagnostic in the log. The comparison report now shows frame P50/P95/P99 and P50/P95/P99 for acquisition, graph CPU, presentation and asynchronous GPU metrics. Surface acquisition errors are not currently written as CSV records; the runner catches incomplete successful-frame counts, but the engine does not yet expose a separate dropped/acquisition-error counter.

## Rendering-path findings

`wgpu` is pinned to 25.0.2. Bloom extraction and tone mapping already share an encoder and one submission in the current working tree; their ordered passes preserve the bloom-to-tone-map dependency and both work in window and offscreen paths. The window clear remains necessary for generic frame passes and early-return paths such as a missing camera. The HDR clear remains necessary because `MeshRenderer::render` can return for empty, invalid or fully culled draws and the offscreen caller supplies its own clear color. The tone-map output clear ensures pixels without the fullscreen draw are defined. These clears were not removed based only on normal-scene timing.

Timestamp polling calls `device.poll(PollType::Poll)` and consumes channel results with `try_recv`; it does not wait for unfinished GPU work in the window loop. `PollType::Wait` occurs for explicit synchronous offscreen readback and context destruction. No additional blocking wait was introduced. When the render graph fails, the window path now discards the acquired surface texture and does not present or record that partial frame. Zero-sized windows return before acquisition; Outdated/Lost trigger resize; Timeout is skipped; other surface errors disable the window and are logged.

## Validation performed

- `cargo fmt --check` — passed.
- `cargo check --release -p graphics_showcase` — passed.
- PowerShell parser validation and Python bytecode compilation for the two report scripts — passed.
- The stderr/exit-code probe described above — passed.
- Full six-capture comparison with the exact command and workload in this document — completed; reports, CSVs and logs were generated.

The capture comparison is specific to this adapter, driver, display, build, workload and run conditions. The P50/P95/P99 numbers measure only successful profile rows; dropped or invalid surface frames are not currently counted separately.

## Historical Mailbox revalidation (2026-10-24)

The raw captures in `target/profiles/present-mode-comparison/20261024-021252` and `20261024-120033` were compared directly. Both manifests describe the same workload: release showcase, 1,050 total frames, 60 warmup frames, grid 32, 32 active models, 32 prefetched models, 1 MiB unpinned CPU-cache budget, 300-frame lifecycle, 128-pixel textures and desired frame latency 1. In every CSV the retained frame indices are 61–1051, with 991 unique frame records and 2,973 pass records. All six latest logs confirm `target\\release\\graphics_showcase.exe`, the intended command arguments, RTX 2050 / driver 581.86 / Vulkan / 1280 × 800, and the requested mode and latency were actually selected. The two sets have identical 63-column CSV headers in identical order; timing fields are nanoseconds in the CSV and are rendered as milliseconds.

| Mailbox metric (median of 3 run-level values) | Earlier capture `021252` | Latest capture `120033` | Change |
|---|---:|---:|---:|
| Frame wall mean | 6.734 ms | 0.583 ms | -91.34% |
| Frame wall P50 | 6.753 ms | 0.439 ms | -93.50% |
| Frame wall P95 | 7.092 ms | 1.661 ms | -76.58% |
| Frame wall P99 | 7.653 ms | 2.927 ms | -61.76% |
| Surface acquisition P95 | 6.885 ms | 0.889 ms | -87.09% |

For context, FIFO frame-wall P95 was 13.982 ms before and 13.983 ms in the latest set; FIFO acquisition P95 was 13.314 ms and 13.116 ms. This is strong evidence that the Mailbox measurements differ while the FIFO control is nearly stable, but it does not identify why.

The comparison is **not fully apples-to-apples**. The earlier manifest and CSV establish workload, resolved presentation mode, latency, retained rows and schema, but there are no matching logs. Thus the earlier executable/build profile, adapter/driver, render resolution and actual invocation are not recorded and cannot be verified. The new logs establish those details only for `120033`. The identical CSV schema and unchanged `profiling.rs` timer/CSV implementation between source revision `38973f7` and the current source provide no evidence of a changed metric definition; the surface-acquisition timer still brackets `get_current_texture`. The revision written into a Markdown report is read when that report is generated, not embedded by the running executable, and is not proof of the capture's source or binary. Because the earlier binary and machine/display state were not recorded, changed runtime behavior or an unrecorded environment difference remains possible. The Bloom/tone-mapping submission merge is present in the newer source, but these captures do not prove that it caused the acquisition change; no root cause is confirmed.

Both report utilities now use the same linear-interpolation percentile definition at position `(n - 1) × p`; means are arithmetic means. The comparison summary reports each run's percentile and then the median/range across the three runs, without pooling frames. Earlier per-capture reports used nearest-rank, so values could differ by a few hundredths of a millisecond from the comparison summary (for example, latest Mailbox run 1 frame P95 was 1.688 ms nearest-rank and is 1.661 ms with interpolation). The six latest per-capture reports and summary were regenerated from the unchanged CSVs with the unified definition. The old captures and their reports were left intact.

Latest captures have no malformed or non-finite frame timings, negative timings, duplicate frame indices, skipped GPU timestamp samples, or error/panic/failure lines in their logs. GPU timestamps are asynchronous and deduplicated by sample sequence; the valid unique GPU timing sample count varies by capture and is not expected to equal 991. Neither capture set records surface acquisition errors or dropped frames in a separate counter/CSV row. The row-count guard confirms 991 recorded successful profile frames, but cannot prove that no transient surface error occurred outside the recorded rows. The earlier capture has no logs for an error scan.

The six latest per-run logs record the release executable and arguments in this form (with a mode-specific CSV path and resolved mode for each run):

```powershell
$env:RIBBON_GPU_PRESENT_MODE = 'mailbox' # 'fifo' for FIFO captures
$env:RIBBON_GPU_FRAME_LATENCY = '1'
cargo run --release -p graphics_showcase -- `
    --stress-grid 32 `
    --stress-cycle-frames 300 `
    --frames 1050 `
    --profile-warmup 60 `
    --profile-csv <capture.csv> `
    --stress-multi-asset-count 32 `
    --stress-cache-prefetch-count 32 `
    --asset-memory-budget-mib 1
```

The benchmark runner invocation below reproduces those manifest settings. The earlier capture did not retain a log containing its original command.

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\run_present_mode_comparison.ps1 `
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

The report-only revalidation command used on the existing latest CSVs was:

```powershell
$capture = Join-Path (Get-Location) 'target/profiles/present-mode-comparison/20261024-120033'
Get-ChildItem -LiteralPath $capture -Filter '*-run*.csv' -File | ForEach-Object {
    $report = [System.IO.Path]::ChangeExtension($_.FullName, '.md')
    python .\tools\render_profile_report.py $_.FullName --output $report
}
python .\tools\compare_profile_runs.py --input-dir $capture --output (Join-Path $capture 'present-mode-comparison-summary.md') --budget-ms 16.667 --expected-frames 991
```

No new before/after runtime capture was made: the present machine can run the latest benchmark, but the earlier build and hardware/display state are not recoverable from its artifacts, so a fresh run cannot recreate that historical binary/configuration faithfully. Use a newly logged baseline and after build for the next source-level performance claim.
