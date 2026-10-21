# Architecture refactor closeout

Date: 2026-10-21

## Structural changes made

- Removed the obsolete `engine/` subtree and its seven `voxelworks-*` workspace packages, including the old immediate-mode `voxelworks-minimal` app. New code now has one supported application architecture under `crates/`, `plugins/`, `examples/`, and `games/`.
- Migrated the final STUDS benchmark/test imports from the compatibility CPU/platform shims to `plugin_rendering::cpu` and `plugin_audio::rodio_backend`. The `audio` feature guard stays in place, so decode coverage is not forced into `--no-default-features` builds.
- Removed legacy entries and edges from `tools/allowed_deps.toml`; it now has exactly one explicit policy entry for each active workspace package. `tools/check_deps.py` rejects retired `voxelworks-*` packages and enforces dependency direction from package ownership paths, even if someone attempts to add a forbidden edge to the allowlist.
- Pruned the removed local packages and their unused dependency records from `Cargo.lock`. The lockfile remains valid TOML and all remaining lock dependency references resolve.
- Updated CI to validate optional features on active packages, removed all retired-package commands, retained cross-platform build/test and GPU parity coverage, and added a source check against reintroducing the retired package family.
- Retired the unused `build_support/windows_icon.rs` compatibility alias. All executable build scripts use the one canonical `build_support/windows/icon.rs` helper; runtime window/taskbar icon configuration stays in `engine_platform`.
- Updated the architecture, contribution, icon, and Phase 9 documentation. `docs/BASELINE.md`, `docs/LEGACY_ARCHITECTURE.md`, and the relevant parts of the realism plan are marked as historical where they retain old package names.
- Added `docs/SCALABILITY_ROADMAP.md` to separate implemented guarantees from the next major performance/ownership projects.

## Compatibility preserved

- No STUDS golden assertion expression or expected hash was changed.
- No renderer thresholds or existing test expectations were changed.
- No new tests were added as part of this cleanup.
- The old minimal immediate-mode app was dropped instead of maintaining two application/window/rendering models in parallel. Current examples already cover ECS, voxel rendering, GPU windows, glTF mesh rendering, editor integration, and the Horror Slice scene.

## Explicitly not claimed as solved

Removing legacy architecture and cleaning dependency/CI boundaries improves maintainability, but it does not make every runtime subsystem scale without further work. The current engine still has these known limits:

1. Schedule systems run serially and receive exclusive `&mut World`. Safe parallel scheduling needs declared component/resource access and conflict validation.
2. Asset loading is synchronous; handles are non-owning identifiers and storage has no strong-reference retention/eviction policy.
3. Global transforms are recomputed each `PostUpdate`; correct dirty propagation needs explicit change ticks or mutation/invalidation contracts.
4. `RenderGraph` validates dependencies and ordering but has no transient GPU resource allocator/aliasing or GPU timestamp profiler.
5. `plugin_rendering` still owns voxel scene/material data, CPU/GPU voxel rendering, and an ECS bridge. Its split/rename is deliberately deferred until those public API ownership boundaries have a concrete design.

These are recorded as ordered design milestones in `docs/SCALABILITY_ROADMAP.md`; they should be implemented separately with benchmarks and contract coverage, not hidden behind an architectural claim.

## Verification status

Source-level checks performed while preparing this archive:

- All 33 workspace members have manifests, unique package names, and entries in `tools/allowed_deps.toml`.
- Static dependency traversal across normal, build, dev, optional, and target-specific manifest sections passes against the allowlist.
- `Cargo.toml`, `Cargo.lock`, all active package manifests, `tools/allowed_deps.toml`, and `.github/workflows/ci.yml` parse successfully.
- The lockfile contains no retired `voxelworks-*` package records; all current workspace package records exist in the lockfile.
- The STUDS golden assertion expressions were compared before/after and are unchanged.
- The source archive contains no `.github-actions-runner/`, `.git/`, `target/`, or generated delivery ZIPs.

Rust formatting, Clippy, Rust compilation, tests, GPU rendering, and CI were **not run in this environment** because the Rust toolchain (`cargo`) is unavailable here. Run the documented command matrix in a Rust-enabled Windows checkout before merging this archive.
