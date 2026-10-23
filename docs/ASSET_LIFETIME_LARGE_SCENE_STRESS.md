# Asset lifetime and large-scene stress

Date: 2026-10-23

## Scope

This milestone adds an opt-in target-side workload to make model lifetime, GPU cache residency, instance
buffer high-water behavior, and scene scale observable with the existing profile CSV. The default
`graphics_showcase` remains the normal small showcase; the stress scene is only enabled by CLI options.

## Large-scene workload

`graphics_showcase --stress-grid SIDE` creates `SIDE * SIDE` placements of one glTF asset (maximum side
128 / 16,384 placements). All placements refer to one `(AssetId, generation)` and therefore are expected
to share one GPU model-cache entry, its primitive buffers, material slots and content-addressed textures.
The renderer still extracts each placement and produces per-placement draw instances, which exercises ECS
extraction, frustum culling, sorting, instance-buffer sizing and rendering at a controlled scale.

The work count is intentionally bounded. Increase `SIDE` gradually (for example 16, 32, 48, 64) and
capture each run separately instead of beginning at the maximum. Keep window size, resolution, Release
profile, scene, warmup and frame count fixed when comparing results.

For a repeatable sweep of recommended sizes plus an asset-lifecycle capture, use
`tools/run_large_scene_stress.ps1`. It creates a timestamped output folder under `target/profiles/large-scene/`,
keeps one CSV and Markdown report per scenario, and writes a `large-scene-run-manifest.md` listing the outputs.
The script runs profiling examples only; it does not run tests. It sets and restores the requested surface
configuration (`-PresentMode fifo` and `-FrameLatency 1` by default) so a previous PowerShell environment
setting cannot silently change the capture. Override the sizes and capture bounds with `-GridSides`,
`-Frames`, `-Warmup`, `-LifecycleSide`, and `-CycleFrames`; use `-SkipLifecycle` to omit the reload-cycle capture.

## CPU and GPU asset lifetime cycle

Add `--stress-cycle-frames N` along with `--stress-grid SIDE` to toggle the stress grid periodically:

1. Remove only the currently tagged stress placements.
2. Drop the stress resource's strong `AssetRef<ModelAsset>` lease.
3. Call `AssetServer::evict_unused`; generation changes on the next request and stale handles must not
   resolve to the replacement asset.
4. `Render3dPlugin` treats a `ModelInstance` with an evicted/stale weak handle as nonresident even if the
   entity lingers in a just-produced snapshot. It releases GPU model cache entries once no resident model
   key is referenced, and drops material slots/last strong texture references. The weak content cache must
   not pin those texture allocations.
5. Request a new lease asynchronously, wait for the worker result, then respawn the grid using the new
   handle generation.

The console marks each deactivate, load request, activation and failure, and includes the actual model asset ID
and generation separately from the stress-grid generation. A failed reload leaves no stress placements and is
retried on a later cycle. `AssetServer` auto-eviction is enabled only in stress mode; the normal showcase does
not change its asset-server policy. Stress mode does not synchronously decode the unused demo-triangle GLB.

## Instance-buffer retention

The mesh renderer grows its visible/shadow instance buffers immediately when more capacity is required.
To avoid retaining a large-scene high-water allocation forever, it now counts consecutive frames at
one-quarter capacity or less and shrinks after 120 such frames when capacity exceeds 256. The new
capacity is the next power of two for current demand, with a minimum of 16. The cooldown is a hysteresis
policy: short camera/frustum fluctuations do not trigger repeated GPU buffer replacement. When no draw
items are present, the render pass still services the shrink policy.

## New profile fields

The CSV and Markdown profile report expose:

- extracted built-in mesh, glTF model and voxel placement counts;
- current GPU model-cache and voxel-mesh cache entries;
- cumulative model/voxel GPU upload and release counts;
- current/peak instance-buffer capacity, plus cumulative instance-buffer grow/shrink events.

These values allow a large-scene capture to distinguish high placement counts from GPU resource counts. The
report displays latest and peak values for placement counts, cache entries and instance-buffer capacity: a
capture may legitimately finish inside an inactive cycle, with latest counts at zero but a nonzero workload peak.
For a grid of repeated instances, placement count should scale with `SIDE * SIDE` while model cache entries
should remain one when the asset is resident. In an inactive churn interval, model cache entries and
unique material texture bytes should fall after the render pass consumes the no-placement snapshot. The
current renderer itself remains alive with its built-in cube/white material and fixed shadow targets, so
not all renderer memory is expected to return to zero.

## Target-side validation

No tests were added and no Cargo command or runtime was executed while preparing this milestone. Validate
on the target Windows checkout first:

```powershell
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace -j 1
cargo test --workspace --no-default-features -j 1
python tools/check_deps.py
```

Then run a steady large scene and a churn cycle. The automated sweep is optional; the minimal manual commands are:

```powershell
cargo run --release -p graphics_showcase -- --stress-grid 32 --frames 1800 --profile-warmup 60 --profile-csv target/profiles/large-scene.csv
python tools/render_profile_report.py target/profiles/large-scene.csv --output target/profiles/large-scene.md

cargo run --release -p graphics_showcase -- --stress-grid 24 --stress-cycle-frames 300 --frames 1800 --profile-warmup 60 --profile-csv target/profiles/asset-lifecycle.csv
python tools/render_profile_report.py target/profiles/asset-lifecycle.csv --output target/profiles/asset-lifecycle.md
```

Acceptance observations are: the extractor reports the expected placement count; model cache entries remain
bounded; the CPU asset eviction/reload log reports generation transitions; GPU model releases rise when the
grid disappears and uploads rise when it returns; texture bytes fall when the last model material is
released; and instance-buffer capacity eventually shrinks during an inactive period. Timing spikes during
intentional reload frames are expected and should be separated from active steady-state measurements.

## Multi-asset and CPU-cache-budget stress (added 2026-10-23)

`tools/generate_asset_stress_corpus.py` generates deterministic self-contained GLB models under
`target/asset-stress-corpus/models/`. Every file varies geometry dimensions, metallic/roughness factors,
base color and an embedded textured PNG; generated files are disposable and are not checked into `assets/`.
The generator accepts 1–256 models and texture sizes from 8 to 512 pixels (power of two).

New `graphics_showcase` options enable a multi-asset scene:

- `--stress-multi-asset-count N` uses N distinct generated models, assigning them round-robin across the grid.
- `--stress-cache-prefetch-count N` requests extra assets without strong leases at low cache-retention priority.
- `--asset-memory-budget-mib N` sets the soft byte budget for unleased decoded CPU assets.
- `--stress-cycle-frames N` can be combined with those options to drop all active leases, observe cache eviction, reload models asynchronously, and respawn with the new generations.

The profiling report separates CPU decoded-asset cache estimates from renderer/GPU allocation estimates and shows resident/pinned/unpinned bytes, configured budget, unpinned overage, CPU asset counts and cumulative budget evictions. Byte rows show exact integer bytes alongside KiB/MiB values instead of displaying sub-MiB allocations as `0.00 MiB`.

For the async multi-asset mode, the first render-extraction diagnostic now waits while model activation is pending. Once the scene activates, it allows a few frames for the extraction snapshot to observe the new placements before reporting an empty-scene failure. The activation log also prints the current estimated resident/pinned/unpinned CPU bytes and cache-budget pressure; the initial `resident=0` line means workers have not committed decoded assets yet, not that the asset corpus uses no memory.

Lifecycle generation numbers are per asset path, separate from the stress-grid generation. If budget enforcement evicts only some assets, the next activation can legitimately contain both older resident asset generations and newer generations for reloaded assets. That mixed list is evidence of selective cache eviction/reuse, not by itself a stale-handle failure.

The budget is soft: active strong leases are protected and can make pinned resident bytes exceed the configured unpinned-cache budget. Loader estimates include known collection capacities and decoded image payloads but are not RSS/allocator telemetry; the budget excludes in-flight decode scratch and queued decoded-but-not-committed assets. Cache-retention priorities affect eviction order only, not the worker queue scheduling order. The stress test does not emulate camera-centered world streaming or cancellation of obsolete in-flight requests.

To run the original 16/32/48 grid sweep plus the new multi-asset and lifecycle captures from PowerShell, use:

```powershell
powershell -ExecutionPolicy Bypass -File .\tools\run_large_scene_stress.ps1 `
    -GridSides 16,32,48 `
    -Frames 900 `
    -Warmup 60 `
    -LifecycleSide 24 `
    -CycleFrames 300 `
    -MultiAssetCount 32 `
    -CachePrefetchCount 32 `
    -AssetMemoryBudgetMiB 1 `
    -MultiAssetGridSide 32 `
    -StressTextureSize 128
```

The script now normalizes `GridSides` whether PowerShell passes a comma-separated value or an integer array. It writes the corpus manifest under `target/asset-stress-corpus/` and each capture/report into a timestamped profile folder. GPU byte estimates remain separate from CPU cache estimates and continue to exclude driver allocator/layout overhead. The profile's cumulative renderer cache counters describe one renderer instance and reset if the render pass/device is recreated. Target-side correctness, memory measurements, visible-material review and before/after captures are still required before declaring production-scale readiness.
