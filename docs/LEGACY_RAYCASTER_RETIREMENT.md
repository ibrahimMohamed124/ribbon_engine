# Legacy GPU Raycaster Retirement

## Final decision and architecture boundary

The legacy GPU voxel raycaster in `plugins/rendering/src/gpu/voxel.rs` has been removed. It was an independent GPU raymarching renderer, not a mesh-generation helper and not a dependency of `ribbon_plugin_render3d`.

The supported rendering roles are now:

- `ribbon_plugin_render3d`: the engine-owned GPU mesh renderer. `Render3dPlugin` renders through the native-surface path; `OffscreenVoxelRenderer` consumes `ChunkedVoxelWorld` updates, uploads replacement chunk meshes, and releases GPU meshes for empty chunks.
- `ribbon_plugin_rendering::cpu::render::raycaster`: retained CPU voxel renderer for supported software/reference/fallback use. Its implementation, CPU material/shader API, and gameplay ray-query behavior were not removed by this change.
- `ribbon_plugin_rendering::ecs::RenderingPlugin`: CPU-only bridge for the existing legacy `RenderScene` / `VoxelWorldRes` API. It no longer promises a GPU rendering option.
- `games/studs`: a game-local `StudsRendererMode` keeps the user's `--renderer cpu|gpu|auto` option separate from the legacy plugin API. STUDS GPU mode continues to use `ribbon_plugin_render3d::OffscreenVoxelRenderer`, and its CPU mode remains available.

## Code and API retired

- Removed `plugins/rendering/src/gpu/voxel.rs` and `plugins/rendering/src/gpu/mod.rs`.
- Removed the old WGSL shader files under `plugins/rendering/shaders/voxel/`.
- Removed the `gpu` feature and the `wgpu`, `pollster`, and `bytemuck` optional dependencies from `plugins/rendering/Cargo.toml`. `ribbon_render` and `ribbon_plugin_render3d` retain their own GPU dependencies; those are still required by the current mesh renderer.
- Removed the `ribbon_plugin_rendering::gpu` public module and the `GpuRenderer` / `GpuPresenter` exports.
- Removed the old `render_gpu_frame_system`, `GpuBackend`, and `RenderBackend::{Cpu, Gpu}` selector from `plugins/rendering/src/ecs.rs`.
- Removed `RenderSettings.backend`. `RenderSettings` now configures only `width` and `height` for the CPU ECS renderer.
- Removed the old CPU-versus-legacy-GPU parity integration test. Its assertions were specific to the deleted raymarcher and must not be presented as parity coverage for the mesh renderer.
- Updated `examples/editor_demo`, `examples/voxel_room`, and STUDS setup. `voxel_room` documents its current CPU-only legacy-scene scope and directs GPU voxel-mesh exploration to `voxel_mesh_showcase`.

This is an intentional public API break for consumers of `ribbon_plugin_rendering::gpu`, `RenderBackend::Gpu`, or `RenderSettings.backend`. Migrate GPU rendering to `ribbon_plugin_render3d`; use `RenderSettings { width, height }` when using the retained CPU `RenderingPlugin`. The `--renderer cpu|gpu|auto` option belongs to STUDS and remains supported there through `StudsRendererMode`.

## Regression coverage

- `plugins/rendering/tests/legacy_gpu_backend_retired.rs` guards that the removed GPU source/shaders/parity test stay absent, the `ribbon_plugin_rendering` Cargo manifest does not reintroduce the old `gpu` feature or its private dependencies, and active workspace Rust source does not reference retired backend symbols. It also checks the generic CPU ECS bridge and that STUDS selects `OffscreenVoxelRenderer` without coupling its backend mode to the old ECS API.
- STUDS unit tests cover default renderer selection across feature combinations, explicit CPU selection, unsupported GPU requests in no-GPU builds, and CLI handling of `cpu`, `gpu`, `auto`, and invalid backend values.
- `plugins/render3d/src/offscreen.rs` adds an adapter-dependent offscreen integration test which exercises initial mesh upload, no-op edits, boundary-neighbor invalidation, mesh replacement, eviction of empty chunks, removal of the final dynamic mesh, repeated empty frames, and re-add/remove cycles that reuse GPU mesh slots. It checks visible pixels, empty-frame output, upload/release counters, and the live mesh table (accounting for the renderer's permanent built-in cube). On a machine with no working GPU adapter, this test prints an explicit `SKIP` message and exits early, mirroring the repository's existing GPU-test convention; CI sets `RIBBON_REQUIRE_GPU_TESTS=1` to turn adapter initialization failure into a hard failure.
- Existing `plugins/voxel` tests cover revisions, no-op writes, out-of-bounds edits, and empty update generation. A six-direction boundary regression checks positive/negative X, Y and Z faces. Existing `plugins/render3d` tests cover shader validation, depth testing, transforms, texture/material appearance, and offscreen GPU output.
- The removed ignored test about custom CPU `SurfaceShader::albedo` / `entity_surface` behavior was an empty placeholder documenting a known contract difference. The legacy backend it referred to no longer exists. This change does **not** claim that custom CPU `SurfaceShader` callbacks are shared with the mesh renderer: CPU callbacks remain CPU-specific, while mesh rendering consumes engine mesh/material/palette data. Applications needing identical custom surface shading across both renderers require a separate material/shader contract design.

## Validation status for this overlay

Static/source, TOML, documentation, and ZIP validation can be checked in the overlay packaging environment. Rust compilation, Clippy, unit tests, and GPU integration tests must be rerun on the target Windows project because this editing environment does not provide `cargo` or `rustc`. Do not treat earlier successful builds as validation of this new removal change.

Recommended target validation from `D:\exp\ribbon_engine\engine`:

```powershell
git diff --check
cargo fmt --all -- --check
cargo check --locked --workspace --all-targets
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo build --locked --workspace --examples
cargo test --locked --workspace
cargo build --locked --workspace --all-targets --no-default-features
cargo clippy --locked --workspace --all-targets --no-default-features -- -D warnings
cargo test --locked --workspace --no-default-features
cargo build --release --locked --workspace --examples
```

On the GPU-equipped target, make adapter availability a hard requirement for the offscreen integration test so it cannot silently skip:

```powershell
$env:RIBBON_REQUIRE_GPU_TESTS = "1"
cargo test --locked -p ribbon_plugin_render3d dynamic_chunk_meshes_upload_replace_and_evict_without_stale_gpu_entries -- --nocapture
$testExit = $LASTEXITCODE
Remove-Item Env:RIBBON_REQUIRE_GPU_TESTS
if ($testExit -ne 0) { throw "Required GPU mesh lifecycle test failed: $testExit" }
cargo test --locked -p ribbon_plugin_rendering --test legacy_gpu_backend_retired -- --nocapture
```

Confirm the mesh lifecycle test executes without the `SKIP` message and passes. Also smoke-test `graphics_showcase`, `voxel_mesh_showcase`, STUDS CPU mode, and STUDS GPU mode. `cargo clean`, `git clean`, and destructive resets are not part of this validation.
