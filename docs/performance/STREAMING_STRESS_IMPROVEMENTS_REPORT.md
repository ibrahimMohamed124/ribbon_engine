# Ribbon Engine — Streaming Stress & Model Upload Improvements

## Why this patch exists

The RTX 2050/Vulkan stress run passed functionally, but its renderer profile showed a CPU-heavy `model GPU resource creation/submission` phase: p50 16.591 ms, p95 17.272 ms, p99 18.026 ms, max 18.727 ms across 186 model-upload samples. The samples aligned with single-model uploads of approximately 1.4 MB. The old renderer path generated a complete image mip chain while creating GPU model materials on the render thread.

The same run also exposed test-harness gaps: a 96/95 activation counter at route turnaround, generic standalone GPU leases were still held when release metrics were captured, and the stress route reached only 1/4 concurrent active regions. The Headless memory sampler used 250 ms intervals, producing one sample for a short phase.

## Changes

### 1. Move glTF material mip preparation off the render frame

- `plugins/gltf` now prepares sRGB and linear material mip levels as part of `GltfLoader::load`/`parse`, which runs on the AssetServer decode worker in the normal asynchronous loading path.
- sRGB base-color/emissive textures are filtered in linear light; metallic-roughness, normal, and occlusion maps are filtered as linear data.
- Only material images referenced by placed scene primitives are prepared. Unused file materials no longer force mip generation.
- The mip algorithm preserves the renderer's prior filter rules and is covered by a parity test against the renderer compatibility path.
- `ModelAsset::estimated_size_bytes` includes retained mip-chain CPU capacities. This intentionally makes CPU residency estimates account for the additional worker-prepared data (roughly one-third of each used image per transfer-function role).
- `ModelAsset::estimated_gpu_upload_bytes` now counts each unique image/transfer-function variant once, uses actual prepared mip byte lengths when available, and calculates exact fallback mip dimensions for manually created assets.
- Renderer model upload consumes prepared mip levels. The existing public `upload_pbr_material_with_maps` and standalone texture path retain their compatibility mip generator for callers without prepared levels.
- Renderer-side validation rejects malformed, incomplete, or incorrectly sized prepared mip levels before creating GPU objects.

This targets CPU work seen in the existing model-upload phase. It does **not** move `wgpu` object creation or queue writes off the renderer-owning thread. No performance improvement is claimed until the same hardware stress run is repeated.

### 2. Correct and strengthen the stress harness

- Region route activation counters now count only actual transitions to `Active`; the shared turnaround endpoint is not counted twice. For 48 regions and two traversal cycles, the expected count is 95.
- Both Headless and GPU phases explicitly activate four overlapping regions concurrently, hold them active, report their activation latency and peak concurrency, and verify eventual unload.
- GPU stress drops its standalone Texture/Sampler leases after their requests reach terminal states, then continues rendering so renderer-owned orphan collection can run. The GPU phase passes this check only when the expected release count is observed and live standalone texture/sampler counts and estimated bytes return to zero.
- GPU profile sample capacity includes the extra concurrency phase.
- Frame CSV includes current concurrent-region count and generic GPU release/residency counters (30 columns total).
- PowerShell process memory sampling defaults to 25 ms and remains configurable with `-MemorySampleIntervalMs`; it records the directly tracked child process, not all descendants.

## Validation added

- glTF parser test confirms parsed images contain the expected full mip chain.
- Worker-prepared sRGB mip levels are compared against the renderer's previous compatibility filter.
- Test confirms only placed-material image roles receive mips and the CPU size estimate includes them.
- Test confirms shared image data is counted once per transfer-function variant for GPU upload budgeting.
- Render3D validation tests reject malformed prepared mip dimensions/lengths.
- Existing stress harness tests verify CSV column count and route turnaround activation count.

## Apply

Use `apply_streaming_improvements.ps1` from the extracted overlay directory:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\apply_streaming_improvements.ps1 -ProjectRoot "D:\path\to\Ribbon_Engine"
```

The script validates all source hashes before modifying the project, creates a timestamped backup under `backups`, copies the files, and validates each destination SHA-256.

## Required Windows validation

Run from the workspace root:

```powershell
cargo fmt --all
cargo fmt --all -- --check
cargo check -p ribbon_plugin_gltf --all-targets
cargo check -p ribbon_plugin_render3d --all-targets
cargo check -p ribbon_streaming_stress --all-targets
cargo test -p ribbon_plugin_gltf --all-targets
cargo test -p ribbon_plugin_render3d --all-targets
cargo test -p ribbon_streaming_stress --all-targets
cargo clippy -p ribbon_plugin_gltf --all-targets -- -D warnings
cargo clippy -p ribbon_plugin_render3d --all-targets -- -D warnings
cargo clippy -p ribbon_streaming_stress --all-targets -- -D warnings
py -3 tools/check_deps.py
```

Then rerun the same stress configuration for a fair comparison:

```powershell
.\examples\streaming_stress\run_streaming_stress.ps1 `
  -Regions 48 -Cycles 2 -Assets 64 -AssetBytesKiB 256 `
  -TextureSize 512 -FramesPerRegion 24 -TailFrames 120 `
  -UploadsPerFrame 2 -UploadBudgetMiB 2 -MemorySampleIntervalMs 25
```

Compare `model GPU resource creation/submission` p50/p95/p99, frame wall p95/p99, worker decode time, CPU residency, four-region peak/hold, activation counts, generic resources released, and final live texture/sampler counts. The new result is the only evidence that will show whether the render-thread CPU phase improved on the RTX 2050.

## Verification limits

The current artifact-preparation environment does not provide Cargo, rustc/rustfmt, or PowerShell. Static checks performed for this overlay include parsing the Cargo TOML manifests, confirming workspace membership and package identity, checking 30 CSV fields/placeholders, checking all four PowerShell sampling hooks, verifying source file presence, and checking Rust delimiter balance. Rust compilation, Rustfmt, Clippy, PowerShell parsing, and hardware GPU execution have **not** been run on this patch in this environment.
