# Validation Log Follow-up — October 22, 2026

This note records the follow-up for the Windows validation output supplied after the access-declared scheduler update.

## Findings from the supplied log

- `cargo fmt --check` reported formatting-only differences in `crates/engine_assets/src/lib.rs`, `crates/engine_schedule/src/lib.rs`, `examples/horror_slice/src/main.rs`, `games/studs/examples/realistic_bench.rs`, `games/studs/tests/golden.rs`, and `games/studs/tests/renderer.rs`. Those reported formatting differences are corrected in this archive; no test cases were added or changed semantically.
- `cargo clippy --workspace --all-targets -- -D warnings` completed successfully in the supplied log.
- The unit/integration test groups displayed before doctests reported success, including the existing ignored GPU `SurfaceShader` limitation.
- Both `cargo test --workspace -j 1` and `cargo test --workspace --no-default-features -j 1` stopped at the `engine_app` doctest with `unresolved import engine_ecs::SyncResource`.
- `python tools/check_deps.py` completed successfully in the supplied log.

## `SyncResource` doctest diagnosis

The current source has `pub use world::{Resources, SyncResource, World};` in `crates/engine_ecs/src/lib.rs`, and `crates/engine_app/src/lib.rs` intentionally re-exports `SyncResource`. Therefore the observed rustdoc error is consistent with a stale compiled `engine_ecs` artifact, especially if this archive was extracted over an earlier checkout while retaining its existing `target` directory and build timestamps. The log alone cannot prove the exact cause, so a clean rebuild is required to confirm it.

From the project root on Windows, run once:

```powershell
cargo clean
```

Then rerun the validation commands in `docs/ACCESS_DECLARED_PARALLEL_SCHEDULER.md`. `cargo clean` removes only generated build artifacts under `target`; it does not change source files or add tests.

## Verification boundary

No Rust compilation, test suite, or runtime was executed to prepare this follow-up. The source changes here are limited to the rustfmt differences shown in the supplied log and this documentation note. Treat the two workspace test commands as unverified until they complete successfully after the clean rebuild.

## Follow-up: clippy `redundant_locals` finding

A subsequent run of `cargo clippy --workspace --all-targets -- -D warnings` reported
`clippy::redundant_locals` in `crates/engine_schedule/src/lib.rs`: the scoped worker closure
rebound `world_ref` to itself before spawning. The redundant local binding has been removed;
the `&World` shared reference is captured directly by the `move` closure. This is a source-only
fix and adds no tests. Clippy has not been rerun on this archive, so this finding remains pending
confirmation by the user-side command.

The most recently shared `cargo test --workspace -j 1` excerpt stops while starting the first
zero-test executable and does not include the command's final exit status. Its completion cannot
be inferred from that excerpt.


## Follow-up: asset clippy findings

A later Windows clippy run reported three denied warnings in `crates/engine_assets/src/lib.rs`:

- `clippy::type_complexity` for the erased-asset removal callback map. The callback signature is now expressed through the private `AssetRemover` type alias.
- `clippy::question_mark` in `AssetServer::get_mut`. The stale-handle early return now uses `self.get(handle)?` while preserving the same `None` behavior.
- `clippy::while_let_loop` in `poll_pending_loads`. Completion draining now uses `while let Ok(completion) = receiver.try_recv()`, still stopping on either an empty or disconnected channel.

These are source-only cleanup changes; no tests were added or run. Re-run formatting, clippy, and both workspace test commands on the resulting archive to confirm the full validation state.


## Follow-up: render3d doctest cannot resolve GPU/profiling exports

A later Windows doctest reported unresolved imports for `engine_render::gpu::{TransientTextureDescriptor, TransientTextureHandle, TransientTexturePool}` and `engine_render::profiling`. The source in this archive declares `pub mod profiling` unconditionally, declares `pub mod gpu` behind the `gpu` feature, and defines the transient-pool public types in `crates/engine_render/src/gpu.rs`. The workspace-wide `--no-default-features` validation command can disable default features for workspace packages, so `plugin_render3d` now explicitly enables `engine_render`'s `gpu` feature rather than relying on that dependency's default feature.

The imports from `engine_render::profiling` are not feature-gated in source. If rustdoc still reports that module as absent, use a fresh extraction directory and run `cargo clean` before retrying; that symptom is consistent with an old `engine_render` artifact or a mixed/partially overwritten checkout. The source tree and this archive were not built or tested while preparing this fix, so the user-side commands must confirm the diagnosis.

## Follow-up: repeated `plugin_render3d` doctest import errors

The supplied source archive contains all of the following public API declarations:

- `crates/engine_render/src/lib.rs`: unconditional `pub mod profiling;` and `pub use profiling::{...}`.
- `crates/engine_render/src/lib.rs`: `#[cfg(feature = "gpu")] pub mod gpu;`.
- `crates/engine_render/src/gpu.rs`: public `TransientTextureDescriptor`, `TransientTextureHandle`, and `TransientTexturePool` definitions.
- `plugins/render3d/Cargo.toml`: an explicit `engine_render` dependency with `default-features = false, features = ["gpu"]`.

Thus the reported combination (including missing the unconditional `profiling` module) is not explained by a missing declaration in these sources. It points to a stale or mismatched compiled dependency, or to an extraction that did not replace the source files expected by Cargo. This repack refreshes source/archive timestamps to make Cargo notice the source package when it is extracted over an existing checkout.

Recommended Windows recovery, from the exact project root (`D:\exp\ribbon_engine\engine`):

```powershell
# Verify the source being compiled has the expected declarations.
Select-String .\crates\engine_render\src\lib.rs -Pattern 'pub mod profiling','pub mod gpu','pub use profiling'
Select-String .\crates\engine_render\src\gpu.rs -Pattern 'pub struct TransientTextureDescriptor','pub struct TransientTextureHandle','pub struct TransientTexturePool'
Select-String .\plugins\render3d\Cargo.toml -Pattern 'engine_render.*features.*gpu'

# Remove all generated Rust build artifacts, then rebuild the doctest first.
cargo clean
cargo test -p plugin_render3d --doc -v
```

If these exact source declarations are present and a clean targeted doctest still reports them missing, capture the `-v` rustdoc command and the new artifact names; investigate Cargo feature/unit selection or a mixed checkout next. No Rust commands or tests were run while preparing this archive.



## Follow-up: GPU profiling integration compile errors

The latest Windows validation output identified three source errors in `crates/engine_render/src/gpu_window.rs`: the completed-frame tuple was declared with three elements while four values were recorded, and `World` has no `try_resource_mut` method. The tuple now explicitly contains frame time, graph timing, target-byte estimate, and readback-buffer-byte estimate. After initializing `RenderDiagnostics` when absent, the presenter mutably borrows it through the existing `World::resource_mut` API.

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
claim for all adapters. The `engine_render::GpuContext` desired maximum frame latency default is now `1`; FIFO remains the default present
mode and environment overrides remain available. The independently configured `plugin_rendering` GPU voxel surface
was not profiled by this capture and retains its previous fixed latency `2`. The separate `AutoNoVsync` run resolved to `immediate` and
reported frame-wall P95 1.210 ms, but is explicitly not a default because it can tear and may use more power.

Added `tools/compare_profile_runs.py` and `tools/run_surface_ab.ps1` to collect repeated captures and compare
run-level P95/P99 distributions before treating the result as general. No tests were added or executed.
