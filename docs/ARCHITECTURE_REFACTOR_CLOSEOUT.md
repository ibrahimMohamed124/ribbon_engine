# Architecture refactor closeout

Date: 2026-10-21

## Structural changes made

- Removed the obsolete `engine/` subtree and its seven `voxelworks-*` workspace packages, including the old immediate-mode `voxelworks-minimal` app. New code now has one supported application architecture under `crates/`, `plugins/`, `examples/`, and `games/`.
- Migrated the final STUDS benchmark/test imports from the compatibility CPU/platform shims to `ribbon_plugin_rendering::cpu` and `ribbon_plugin_audio::rodio_backend`. The `audio` feature guard stays in place, so decode coverage is not forced into `--no-default-features` builds.
- Removed legacy entries and edges from `tools/allowed_deps.toml`; it now has exactly one explicit policy entry for each active workspace package. `tools/check_deps.py` rejects retired `voxelworks-*` packages and enforces dependency direction from package ownership paths, even if someone attempts to add a forbidden edge to the allowlist.
- Pruned the removed local packages and their unused dependency records from `Cargo.lock`. The lockfile remains valid TOML and all remaining lock dependency references resolve.
- Updated CI to validate optional features on active packages, removed all retired-package commands, retained cross-platform build/test and GPU parity coverage, and added a source check against reintroducing the retired package family.
- Retired the unused `build_support/windows_icon.rs` compatibility alias. All executable build scripts use the one canonical `build_support/windows/icon.rs` helper; runtime window/taskbar icon configuration stays in `ribbon_platform`.
- Updated the architecture, contribution, icon, and Phase 9 documentation. `docs/BASELINE.md`, `docs/LEGACY_ARCHITECTURE.md`, and the relevant parts of the realism plan are marked as historical where they retain old package names.
- Added `docs/SCALABILITY_ROADMAP.md` to separate implemented guarantees from the next major performance/ownership projects.

## Compatibility preserved

- No STUDS golden assertion expression or expected hash was changed.
- No renderer thresholds or existing test expectations were changed.
- No new tests were added as part of this cleanup.
- The old minimal immediate-mode app was dropped instead of maintaining two application/window/rendering models in parallel. Current examples already cover ECS, voxel rendering, GPU windows, glTF mesh rendering, editor integration, and the Horror Slice scene.

## Explicitly not claimed as solved

Removing legacy architecture and cleaning dependency/CI boundaries improves maintainability, but it does not make every runtime subsystem scale without further work. The current engine still has these known limits:

1. The first access-declared parallel scheduler layer is now implemented: legacy `&mut World` systems remain serial barriers, while compatible `SystemConfig::parallel` callbacks can share conflict-free batches. The declarations are a programmer contract and the scheduler keeps a serial fallback. The new layer still needs target-side compilation, serial-vs-parallel comparison, and performance profiling before it should be treated as accepted.
2. At the 2026-10-21 closeout, asset loading was synchronous and no strong-reference eviction policy existed. The follow-up asset-streaming milestone below adds an async initial-load path and opt-in lease-based eviction; that follow-up still requires target-side verification.
3. Transform propagation now skips clean-branch composition and unchanged global writes using snapshot-diff invalidation; detecting edits still requires an O(N) scan while local fields remain public, and target-side profiling is pending. See `docs/TRANSFORM_INVALIDATION.md`.
4. `RenderGraph` validates dependencies and ordering but has no transient GPU resource allocator/aliasing or GPU timestamp profiler.
5. `ribbon_plugin_rendering` still owns voxel scene/material data, CPU/GPU voxel rendering, and an ECS bridge. Its split/rename is deliberately deferred until those public API ownership boundaries have a concrete design.

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

## Transform invalidation implementation

Date: 2026-10-22

- Added per-world snapshot caches for 2D/voxel `Transform` propagation and quaternion-based `Transform3D` propagation.
- Detects added, removed, and changed local transforms and parent links without relying on change ticks, preserving existing code that mutates public fields through ECS queries.
- Invalidates descendants over the union of previous and current parent graphs, so reparenting/removal updates affected branches; clean ancestors use cached global values.
- Resolves dirty entities in stable entity order, retains the separate 2D and 3D composition conventions, writes global components only when values differ, and removes a derived global when its corresponding local transform is removed.
- Added `docs/TRANSFORM_INVALIDATION.md` and updated the scalability roadmap, architecture overview, and `ribbon_scene` README.
- No tests were added, in accordance with the project instruction. The source was not built, formatted, linted, benchmarked, or tested in this environment. The user's earlier Windows verification results cover the pre-change archive only, not this transform-invalidation change.

### Remaining qualification

The caches avoid full hierarchy resolution and unchanged global writes, but current public mutable fields require an O(N) snapshot scan every propagation pass. Performance improvement is therefore an expected optimization for sparse changes, not a measured claim; run the documented target-side verification and benchmark before treating the milestone as fully validated. A later ECS change-tick or controlled-mutation design could remove the full scan, but must preserve compatibility with current `query_mut` callers.



## Asset lifetime and streaming implementation

Date: 2026-10-22

- Added `AssetId`, weak generation-aware `Handle<T>`, strong `AssetRef<T>` leases, reference accounting, explicit `Unloaded` state, generation-safe eviction, revision tracking, and retry support while retaining the existing synchronous API.
- Added bounded worker-backed IO/CPU decoding for regular requests and bounded decoded completions, lifecycle events, and an owner-thread commit path. GPU upload remains inside the rendering plugin.
- Kept a failed hot reload's last-good asset resident and observable through `ReloadFailed`/`last_reload_error`; a successful replacement keeps its generation and advances its revision.
- Migrated the `model_viewer` and `horror_slice` examples to asynchronous GLTF requests with explicit strong leases and load diagnostics.
- Added `docs/ASSET_LIFETIME_STREAMING.md` and updated the roadmap, architecture overview, and `ribbon_assets` README.
- No new tests were written. No Rust compiler, formatter, linter, runtime, test, or benchmark command was run in this environment; the new API and integration still require Windows-side verification.

### Remaining qualification

The initial-load path and hot-reload read/decode are asynchronous for AssetPlugin/request users; modification timestamp polling still runs on the server thread. Queue counts are bounded, but byte budgets, priority, cancellation, progress events, LRU eviction, and asset dependency-graph scheduling are not included. Automatic eviction defaults off to preserve clients that still keep only weak handles. The milestone should not be considered validated until the workspace command matrix passes on the target checkout.
