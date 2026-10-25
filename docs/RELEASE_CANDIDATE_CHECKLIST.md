# Ribbon Engine Release Candidate Checklist

This checklist records release gates; it does not assert they passed merely because code compiles.

## Local correctness

- [ ] `git diff --check` is clean and the reviewed diff contains no unrelated changes.
- [ ] `cargo fmt --all -- --check` passes.
- [ ] `cargo check --locked --workspace --all-targets` passes.
- [ ] `cargo clippy --locked --workspace --all-targets -- -D warnings` passes.
- [ ] `cargo test --locked --workspace` passes.
- [ ] No-default-feature build, Clippy, and tests pass.
- [ ] Release examples build succeeds.
- [ ] Dependency policy and retired-backend source audit pass.

## Interactive showcase verification

- [ ] `graphics_showcase`: in-window control guide is readable; Space, arrows and R work; right-mouse drag orbits in the expected direction; mouse wheel zoom is bounded; resize/focus loss does not corrupt the camera.
- [ ] `voxel_mesh_showcase`: WASD/R/F selection stays in bounds; E/Q edit one cell per press; C changes the material; T resets deterministically; marker tracks the target; panel reports real world/chunk and renderer-published metrics.
- [ ] Editing `(7, 5, 0)` exercises chunk-face neighbors; compare revision/remesh counts and GPU mesh counters after remove/re-add.
- [ ] Resize/minimize/restore both showcase windows and confirm the overlay clips safely and rendering resumes.

## GPU and material contract

- [ ] Required offscreen GPU mesh lifecycle test actually executes on a supported adapter; it must not silently skip.
- [ ] Manual GPU visual check confirms material textures, PBR lighting, shadows, tone mapping, mesh replacement and empty-chunk eviction.
- [ ] The documented CPU-only `SurfaceShader` callback limitation remains explicit unless GPU behavior and regression coverage are added.

## CI workflow handling

- [ ] CI YAML is statically parsed and all referenced paths/commands reviewed.
- [ ] Existing triggers, permissions, concurrency, and runner assumptions are preserved unless reviewed changes require otherwise.
- [ ] CI definitions cover formatting, default/no-default build and test, Clippy, examples, interactive-showcase unit tests, and retired-backend source audit.
- [ ] The GitHub Actions checks for the candidate commit complete successfully; record any unavailable platform/GPU checks explicitly.

## Release metadata

- [ ] Root and example READMEs describe real controls and renderer behavior.
- [ ] Changelog lists the changes and the removal of old APIs where applicable.
- [ ] Known limitations and platform-specific GPU requirements are documented.
- [ ] Version/tag is not advanced to `v1.0.0` until all required release gates are satisfied.
