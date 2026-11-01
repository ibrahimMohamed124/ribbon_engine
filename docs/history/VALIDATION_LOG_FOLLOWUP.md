# Validation Log Follow-up — October 22, 2026

This note records the follow-up for the Windows validation output supplied after the access-declared scheduler update.

## Findings from the supplied log

- `cargo fmt --check` reported formatting-only differences in `crates/assets/src/lib.rs`, `crates/schedule/src/lib.rs`, `games/athar/src/main.rs`, `games/studs/examples/realistic_bench.rs`, `games/studs/tests/golden.rs`, and `games/studs/tests/renderer.rs`. Those reported formatting differences are corrected in this archive; no test cases were added or changed semantically.
- `cargo clippy --workspace --all-targets -- -D warnings` completed successfully in the supplied log.
- The unit/integration test groups displayed before doctests reported success, including the existing ignored GPU `SurfaceShader` limitation.
- Both `cargo test --workspace -j 1` and `cargo test --workspace --no-default-features -j 1` stopped at the `ribbon_app` doctest with `unresolved import ribbon_ecs::SyncResource`.
- `python tools/check_deps.py` completed successfully in the supplied log.

## `SyncResource` doctest diagnosis

The current source has `pub use world::{Resources, SyncResource, World};` in `crates/ecs/src/lib.rs`, and `crates/app/src/lib.rs` intentionally re-exports `SyncResource`. Therefore the observed rustdoc error is consistent with a stale compiled `ribbon_ecs` artifact, especially if this archive was extracted over an earlier checkout while retaining its existing `target` directory and build timestamps. The log alone cannot prove the exact cause, so a clean rebuild is required to confirm it.

From the project root on Windows, run once:

```powershell
cargo clean
```

Then rerun the validation commands in `docs/architecture/ACCESS_DECLARED_PARALLEL_SCHEDULER.md`. `cargo clean` removes only generated build artifacts under `target`; it does not change source files or add tests.

## Verification boundary

No Rust compilation, test suite, or runtime was executed to prepare this follow-up. The source changes here are limited to the rustfmt differences shown in the supplied log and this documentation note. Treat the two workspace test commands as unverified until they complete successfully after the clean rebuild.

## Follow-up: clippy `redundant_locals` finding

A subsequent run of `cargo clippy --workspace --all-targets -- -D warnings` reported
`clippy::redundant_locals` in `crates/schedule/src/lib.rs`: the scoped worker closure
rebound `world_ref` to itself before spawning. The redundant local binding has been removed;
the `&World` shared reference is captured directly by the `move` closure. This is a source-only
fix and adds no tests. Clippy has not been rerun on this archive, so this finding remains pending
confirmation by the user-side command.

The most recently shared `cargo test --workspace -j 1` excerpt stops while starting the first
zero-test executable and does not include the command's final exit status. Its completion cannot
be inferred from that excerpt.


## Follow-up: asset clippy findings

A later Windows clippy run reported three denied warnings in `crates/assets/src/lib.rs`:

- `clippy::type_complexity` for the erased-asset removal callback map. The callback signature is now expressed through the private `AssetRemover` type alias.
- `clippy::question_mark` in `AssetServer::get_mut`. The stale-handle early return now uses `self.get(handle)?` while preserving the same `None` behavior.
- `clippy::while_let_loop` in `poll_pending_loads`. Completion draining now uses `while let Ok(completion) = receiver.try_recv()`, still stopping on either an empty or disconnected channel.

These are source-only cleanup changes; no tests were added or run. Re-run formatting, clippy, and both workspace test commands on the resulting archive to confirm the full validation state.


## Follow-up: render3d doctest cannot resolve GPU/profiling exports

A later Windows doctest reported unresolved imports for `ribbon_render::gpu::{TransientTextureDescriptor, TransientTextureHandle, TransientTexturePool}` and `ribbon_render::profiling`. The source in this archive declares `pub mod profiling` unconditionally, declares `pub mod gpu` behind the `gpu` feature, and defines the transient-pool public types in `crates/render/src/gpu.rs`. The workspace-wide `--no-default-features` validation command can disable default features for workspace packages, so `ribbon_plugin_render3d` now explicitly enables `ribbon_render`'s `gpu` feature rather than relying on that dependency's default feature.

The imports from `ribbon_render::profiling` are not feature-gated in source. If rustdoc still reports that module as absent, use a fresh extraction directory and run `cargo clean` before retrying; that symptom is consistent with an old `ribbon_render` artifact or a mixed/partially overwritten checkout. The source tree and this archive were not built or tested while preparing this fix, so the user-side commands must confirm the diagnosis.

## Follow-up: repeated `ribbon_plugin_render3d` doctest import errors

The supplied source archive contains all of the following public API declarations:

- `crates/render/src/lib.rs`: unconditional `pub mod profiling;` and `pub use profiling::{...}`.
- `crates/render/src/lib.rs`: `#[cfg(feature = "gpu")] pub mod gpu;`.
- `crates/render/src/gpu.rs`: public `TransientTextureDescriptor`, `TransientTextureHandle`, and `TransientTexturePool` definitions.
- `plugins/render3d/Cargo.toml`: an explicit `ribbon_render` dependency with `default-features = false, features = ["gpu"]`.

Thus the reported combination (including missing the unconditional `profiling` module) is not explained by a missing declaration in these sources. It points to a stale or mismatched compiled dependency, or to an extraction that did not replace the source files expected by Cargo. This repack refreshes source/archive timestamps to make Cargo notice the source package when it is extracted over an existing checkout.

Recommended Windows recovery, from the exact project root (`D:\exp\ribbon_engine\engine`):

```powershell
# Verify the source being compiled has the expected declarations.
Select-String .\crates\render\src\lib.rs -Pattern 'pub mod profiling','pub mod gpu','pub use profiling'
Select-String .\crates\render\src\gpu.rs -Pattern 'pub struct TransientTextureDescriptor','pub struct TransientTextureHandle','pub struct TransientTexturePool'
Select-String .\plugins\render3d\Cargo.toml -Pattern 'ribbon_render.*features.*gpu'

# Remove all generated Rust build artifacts, then rebuild the doctest first.
cargo clean
cargo test -p ribbon_plugin_render3d --doc -v
```

If these exact source declarations are present and a clean targeted doctest still reports them missing, capture the `-v` rustdoc command and the new artifact names; investigate Cargo feature/unit selection or a mixed checkout next. No Rust commands or tests were run while preparing this archive.



## Follow-up: GPU profiling integration compile errors

The latest Windows validation output identified three source errors in `crates/render/src/gpu_window.rs`: the completed-frame tuple was declared with three elements while four values were recorded, and `World` has no `try_resource_mut` method. The tuple now explicitly contains frame time, graph timing, target-byte estimate, and readback-buffer-byte estimate. After initializing `RenderDiagnostics` when absent, the presenter mutably borrows it through the existing `World::resource_mut` API.

The same unsupported `try_resource_mut` call in `plugins/render3d/src/lib.rs` was replaced with an existence check followed by `resource_mut`; `Render3dPlugin` depends on `GpuWindowPlugin`, which installs the diagnostics resource. Formatting differences reported for the GPU lifetime/profiling files were also corrected manually to match the supplied rustfmt output. No tests were added or executed, and Cargo/clippy have not been rerun; verify this archive with `cargo fmt --check`, clippy, and both workspace test configurations.

## Follow-up: first performance baseline points to frame-phase diagnosis

The first user-side release captures showed frame-wall P95 around 19.8–21.2 ms, while render-graph CPU
P95 was around 0.9–1.0 ms. Asynchronous shadow-plus-mesh GPU P95 was around 0.16–0.17 ms for
`graphics_showcase` and 0.43–0.46 ms for `voxel_mesh_showcase`. Those measurements do not justify
starting with draw-call or shader optimization: the bulk of the frame-wall tail was outside the measured
GPU interval and graph-node CPU timings, but the original CSV did not separate surface acquisition from
presentation. New captures now record `surface_acquire_ns` and `present_ns`; the Markdown report also
computes the per-frame residual after subtracting acquisition, graph, and presentation wall durations.
Use a fresh capture to identify which phase dominates before changing rendering behavior.

The user-side release build also reported `AssetServer::last_poll` as dead code when neither debug
assertions nor the `hot_reload` feature is enabled. The field, its initializer, and `Instant` import are
now gated by the same configuration that compiles in the only code paths that read it. This avoids a
release-only dead-code warning without changing debug/hot-reload polling behavior.

No Rust build, Cargo command, runtime, or test was executed during this follow-up. No test files were
changed or added; perform the user's normal workspace verification before accepting the phase measurements.


## Surface-acquisition experiment follow-up (2026-10-22)

The submitted Release capture reported frame-wall P95 20.130 ms and surface-acquire P95 18.782 ms, while render-graph CPU P95 was 1.491 ms and `present()` P95 0.182 ms. This points to surface-acquisition pacing as the next experiment, not proof of a specific driver wait. Added opt-in `RIBBON_GPU_PRESENT_MODE` and `RIBBON_GPU_FRAME_LATENCY` settings; at that point, FIFO/2 was still the default pending user-side measurements. The resolved present mode and frame-latency hint are included in CSV and report metadata. No automated tests were added and no Cargo command was run for this change.


## Surface-latency experiment result and default adjustment (2026-10-22)

The user-provided Windows Release captures contain 1,681 retained frames per FIFO configuration. FIFO/2 recorded
frame-wall mean 6.731 ms, P50 1.194 ms, P95 21.359 ms, P99 21.821 ms, and peak 24.908 ms; its surface-acquire
P95 was 20.272 ms, and about 25% of retained frame-wall samples exceeded 16.667 ms. FIFO/1 recorded mean
6.694 ms, P50 6.131 ms, P95 14.069 ms, P99 14.563 ms, and peak 16.366 ms; surface-acquire P95 was
13.269 ms, with no samples above 16.667 ms in that capture. Thus FIFO/1 reduced frame-wall P95 by 34.1% and
surface-acquire P95 by 34.5%, while average frame time remained effectively unchanged and median wall time
increased. This supports smoother tail pacing on the tested machine, not higher mean throughput or a universal
claim for all adapters. The `ribbon_render::GpuContext` desired maximum frame latency default is now `1`; FIFO remains the default present
mode and environment overrides remain available. The independently configured `studs_cpu_renderer` GPU voxel surface
was not profiled by this capture and retains its previous fixed latency `2`. The separate `AutoNoVsync` run resolved to `immediate` and
reported frame-wall P95 1.210 ms, but is explicitly not a default because it can tear and may use more power.

Added `tools/perf/compare_profile_runs.py` and `tools/perf/run_surface_ab.ps1` to collect repeated captures and compare
run-level P95/P99 distributions before treating the result as general. No tests were added or executed.


## Follow-up: large-scene stress compile diagnostics (2026-10-23)

The latest user-provided Windows output exposed two source issues in the large-scene stress archive:

- `clippy::type_complexity` flagged the positional `Option<(...)>` used to carry the completed window-frame measurements. It is now represented by the private named-field `CompletedGpuWindowFrame` struct, making the measured values and resource snapshots explicit.
- In the earlier pre-relocation snapshot, `ribbon_plugin_render3d::offscreen::OffscreenVoxelRenderer::resource_stats` initialized `RenderResourceStats` without the fields introduced for scene/cache/instance diagnostics. The initializer now uses a default for fields that do not apply to this renderer, copies the renderer instance-buffer counters, and reports the currently uploaded chunk meshes and their cumulative upload/release counts. The earlier offscreen path incremented release counts only when a mesh handle was actually released and counted each successful non-empty mesh upload. The current implementation is game-owned at `games/studs/crates/studs_voxel_gpu/src/lib.rs`.

No tests were added or changed. In accordance with the user's request, Cargo, compilation, tests, and runtime were not executed while preparing this correction. Run `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, both workspace test configurations, and `python tools/check_deps.py` on this archive before considering it fully validated.


## Follow-up: model-revision cache key type clarity (2026-10-23)

The latest user-provided `cargo clippy --workspace --all-targets -- -D warnings` output flagged the nested cache-key/revision tuple type in `plugins/render3d/src/lib.rs`. Added the private `ModelAssetKey` and `ModelAssetRevision` aliases and reused `ModelAssetKey` for the pass-owned asset-revision map. This keeps the same `(asset id, generation)` identity and revision data while making the inferred collections easier to read; it does not suppress the lint.

The pasted `cargo test --workspace -j 1` output ends during dependency compilation and does not include a result, so no conclusion is drawn about that command. No tests were added or modified. Cargo, Clippy, compilation, and runtime were not executed while preparing this update. Please rerun the same validation commands against this archive to check for any subsequent diagnostics.

## Follow-up: graphics showcase approximate-constant lint (2026-10-23)

The latest user-provided `cargo clippy --workspace --all-targets -- -D warnings` output flagged the
rotation step `0.392_699_1` in `examples/graphics_showcase/src/main.rs` as an approximation of
`f32::consts::FRAC_PI_8`. The showcase now uses `std::f32::consts::FRAC_PI_8` directly, preserving
the intended angle step while avoiding `clippy::approx_constant`; the lint was not suppressed.

No tests were added or changed. Cargo, Clippy, compilation, tests, and runtime were not executed while
preparing this update. Please rerun `cargo fmt --check` and
`cargo clippy --workspace --all-targets -- -D warnings` on this archive; any subsequent diagnostics
remain unverified until the user-side commands complete.


## Follow-up: multi-asset CPU cache budget and profiling (2026-10-23)

Added loader-provided decoded CPU byte estimates and a soft budget for unleased cache entries. `ModelAsset` includes the capacities of its top-level and nested geometry/image vectors; `BytesLoader` and `TextLoader` include their backing storage capacity. `AssetServer::cpu_memory_stats` reports resident, pinned and unpinned estimated bytes, budget pressure and cumulative budget evictions. `request_with_priority`, `retry_with_priority`, and `request_ref_with_priority` specify cache-retention priority; lower-priority unpinned entries are evicted before higher-priority entries, with FIFO successful-load age as the tie-break. Strong leases remain protected. This is not a strict process-memory cap and does not yet bound decoded-but-not-committed worker results by bytes.

The profiling report now renders exact byte values alongside human-readable units and separates decoded CPU cache data from GPU allocation estimates. The native render plugin reads the full CPU asset cache snapshot only when `RenderProfilePlugin` is installed, avoiding an O(number of cached paths) scan on ordinary render frames. The large-scene harness can generate many unique textured GLBs, place multiple unique assets in one scene, prefetch unleased lower-retention-priority assets, apply a CPU cache budget, and repeat asset lifecycle cycles. `-GridSides` normalization accepts both comma-separated command-line values and integer arrays passed directly by PowerShell.

No test files were added or changed. Rust/Cargo commands, the engine runtime, and the stress capture were not executed in this environment. Static checks only are performed before delivery; target Windows validation remains mandatory.


## Follow-up: user-side multi-asset CPU-cache stress review (2026-10-23)

The user's Release run completed the single-model grids, the 32-active/32-prefetch multi-asset scene, and both lifecycle scenarios. The new multi-asset capture reached 1,024 placements and reported a tracked GPU/resource peak estimate of 26.97 MiB. For the standard grids, frame-wall P95 was 14.327 ms (16×16), 13.776 ms (32×32), and 13.299 ms (48×48); graph CPU P95 rose from 2.309 ms to 2.715 ms across 16×16 to 48×48. The multi-asset lifecycle capture had a 122.965 ms frame-wall peak while its P95 was 13.858 ms. The single-grid 16×16 capture also showed a 159.322 ms peak against a 14.327 ms P95. These isolated peaks should be matched to individual CSV frames before attributing them to rendering, decode, or cache eviction; the console summary alone cannot identify the cause.

The multi-asset startup printed `[RENDER EXTRACT] FAIL` on the first frame while the asset leases were explicitly waiting for asynchronous decode, and then activated the complete scene later in the run. That first-frame failure was premature diagnostic output, not proof that the later render extraction failed. The showcase now defers an empty extraction result while stress-scene activation is pending, and allows a few frames for the extractor to observe newly spawned placements before it reports a failure. Activation also prints a CPU-cache snapshot with resident, pinned, unpinned, budget and over-budget byte values.

The mixed model asset generations during lifecycle reload are compatible with partial budget eviction: evicted assets receive a new generation on the next request, while assets that stayed resident can legitimately reuse their existing generation. The cache budget is still soft for pinned assets; this capture does not establish a process-wide memory ceiling. The user-side logs report that the unpinned residency after the lifecycle budget enforcement was 1,018,800 bytes against a 1,048,576-byte budget, with cumulative budget evictions increasing from 17 to 49 and then 66 over the later cycle.

No tests were added or modified. Cargo, compilation, tests and runtime were not executed in this environment. This follow-up is based on the supplied user-side logs plus a source-only correction to the asynchronous extraction diagnostic; rebuild and rerun the multi-asset capture on Windows to verify the new `PASS` line and cache snapshot.


## Follow-up: multi-asset GPU upload burst pacing (2026-10-23)

The user-provided CSV for `multi-asset-lifecycle-32x32-active32-prefetch32` isolated frame 602 as the long frame: frame wall 43.8668 ms, render-graph CPU 43.7298 ms, and `mesh pass [0]` CPU 43.5411 ms. Surface acquisition/presentation for that frame were 0.0637/0.0650 ms. On the same frame, GPU model cache entries increased from 0 to 32, and the cumulative model upload counter increased by 32 (32 to 64), with 32 additional live meshes/materials. This is strong correlation between the spike and synchronous model GPU resource creation, not evidence that GPU execution itself was slow. Per-row GPU samples are asynchronous and are not treated as perfectly aligned with the same frame.

`ribbon_plugin_render3d` now caps first-use model cache population at two distinct model asset keys per render frame. Missing placements beyond that limit are deferred to subsequent render frames; cached models continue to draw. Added `docs/architecture/GPU_ASSET_UPLOAD_PACING.md` with the evidence and target-side verification requirements. This limit is count-based, not a strict per-frame time budget, and voxel uploads are not changed by this targeted fix.

No tests were added or modified. Rust/Cargo commands, the engine runtime and stress captures were not executed in this environment. Run `cargo fmt --check` and `cargo clippy --workspace --all-targets -- -D warnings`, then rerun the same multi-asset lifecycle capture on Windows and verify the per-frame upload delta and the absence of a comparable mesh-pass stall.


## Adapter observability and FIFO/Mailbox repeat comparison (2026-10-24)

The primary `ribbon_render::GpuContext` now emits a `[GPU ADAPTER]` line from the actual `wgpu::AdapterInfo`, including name, vendor/device IDs, device type, driver strings, backend, surface compatibility and timestamp-query support. The repeated present-mode helper saves each native process' console output to a per-capture `.log`, then copies adapter and surface lines into its manifest. This distinguishes the adapter selected by `wgpu` from the list of devices exposed by Windows.

User-provided, target-side Release results for three runs each at FIFO/1 and Mailbox/1 resolved to their requested modes. Median run-level frame-wall P95 was 13.982 ms for FIFO and 7.092 ms for Mailbox; surface-acquisition P95 was 13.314 and 6.885 ms. Frame-wall mean was 6.481 and 6.734 ms respectively, so the result indicates lower tail latency rather than higher average throughput. Graph CPU P95 also differed (1.773 vs 0.757 ms), so the evidence does not establish that surface acquisition alone caused the entire difference. Results are specific to the tested machine, display, driver, build and stress workload. The previous logs showed that Windows exposes an NVIDIA GeForce RTX 2050 and an Intel UHD adapter, but did not identify which adapter `wgpu` selected.

The primary native-surface default is set to `mailbox` based on these repeat captures, with existing FIFO fallback/overrides retained. The large-scene stress helper's default is aligned to Mailbox; FIFO remains explicitly available for comparison. On render-graph failure, the GPU window path now drops the partially-rendered surface frame rather than presenting it and recording a zero-duration graph sample as if it were successful. No automated tests were added and no Cargo build, test or runtime command was executed while preparing this update; target-side validation remains outstanding.
