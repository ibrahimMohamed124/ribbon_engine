# Ribbon Engine release-candidate checklist

This is a release gate, not a claim that the current archive has already passed every item. Mark a
box complete only after retaining the command output or review record for the exact candidate tree.

## Release identity and source audit

- [ ] Agree the release version and changelog entry; do not tag or publish while workspace packages
      still claim `0.1.x` unless that is the intentional release scheme.
- [ ] Verify `git status --short`, `git diff --check`, tracked/untracked files, and the final diff.
- [ ] Search production sources, manifests, examples, and CI for retired `GpuRenderer`, `GpuPresenter`,
      `ribbon_plugin_rendering::gpu`, `render_gpu_frame_system`, and `RenderBackend` APIs. Hits in the
      explicit retirement regression test and historical documentation must be classified, not blindly
      deleted.
- [ ] Verify `plugins/rendering/tests/legacy_gpu_backend_retired.rs` still fails when the retired
      API/module is reintroduced in an isolated test fixture or when its source guards are deliberately
      challenged. Do not make the guard permissive to silence a failure.
- [ ] Review the deliberate breaking API migration in `docs/RENDERING_API_CONTRACTS.md` against all
      examples and public exports.

## Material/rendering contract

- [ ] `SurfaceShader` is documented and tested as CPU-only; custom albedo/entity callbacks change CPU
      output, and the test does not claim GPU parity.
- [ ] `PbrMaterialDesc`, `TextureData`, `Draw::tint`, and `VoxelBlockColor` document color spaces,
      supported maps/factors, missing-palette fallback, and unsupported callback/per-block texture
      behavior accurately.
- [ ] GPU mesh shader validation and material/base-color texture tests pass.
- [ ] The offscreen dynamic mesh lifecycle test passes on a GPU-equipped runner with
      `RIBBON_REQUIRE_GPU_TESTS=1`, with no `SKIP` output.
- [ ] If product requirements require arbitrary custom surface callbacks or per-block PBR maps on GPU,
      do not sign off based on documentation alone; implement a shared material/shader representation
      and add cross-backend tests as a separately scoped feature.

## Required build/test matrix

- [ ] `git diff --check`
- [ ] `cargo fmt --all -- --check`
- [ ] `cargo check --locked --workspace --all-targets`
- [ ] `cargo clippy --locked --workspace --all-targets -- -D warnings`
- [ ] `cargo build --locked --workspace --examples`
- [ ] `cargo test --locked --workspace`
- [ ] `cargo build --locked --workspace --all-targets --no-default-features`
- [ ] `cargo clippy --locked --workspace --all-targets --no-default-features -- -D warnings`
- [ ] `cargo test --locked --workspace --no-default-features`
- [ ] `cargo build --release --locked --workspace --examples`
- [ ] `python tools/check_deps.py`
- [ ] `python games/studs/tools/fetch_assets.py --check` from the actual Git checkout
- [ ] Review mutually exclusive features before attempting `--all-features`; add supported feature
      combinations to CI explicitly rather than assuming all features can coexist.

## Runtime acceptance on target Windows hardware

- [ ] `cargo run --release -p graphics_showcase`
- [ ] `cargo run --release -p voxel_mesh_showcase`; edit at chunk boundaries and check for seams/stale mesh.
- [ ] `cargo run --release -p studs --no-default-features -- --renderer cpu`
- [ ] `cargo run --release -p studs -- --renderer gpu`
- [ ] Verify WASD, mouse look direction, pitch clamping, resize/minimize/restore, audio, menus, and shutdown.
- [ ] Check GPU adapter initialization, actual rendered pixels, and GPU-resource upload/release behavior;
      adapter initialization alone is not visual acceptance.
- [ ] Capture and review representative PBR/HDR/shadow output; review known renderer limitations in the
      3D plugin README before labeling the feature set production-ready.

## Current honest status

The source-level contract/test/documentation changes in this patch have not been compiled in this
editing environment because `cargo`, `rustc`, and `rustfmt` are unavailable here. Therefore this file
remains an unchecked release gate. The prior Windows results reported before this patch do not validate
these new changes. Do not declare a release candidate complete until the matrix above is run against
this exact patch and target-side runtime checks are recorded.
