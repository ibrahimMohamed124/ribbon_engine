# Ribbon Engine — World Streaming gap-completion cumulative overlay

This is a cumulative overlay based on the previously applied v3 streaming overlay. It contains the complete SceneDoc/Prefab/Physics/World Partition/GPU streaming continuation and fix chain plus the latest lifecycle phase tracing, GPU residency admission budgets, SceneDoc revision reload handling, Region-local render visibility staging/commit, editor/ATHAR integration, and stress-harness updates. See [`README.md`](README.md) and [`GAP_COMPLETION_REPORT.md`](../../streaming/GAP_COMPLETION_REPORT.md) for its scope and remaining architectural limits.

## Apply

Extract this ZIP outside the Ribbon checkout and run:

```powershell
powershell -ExecutionPolicy Bypass -File .\apply_world_streaming_overlay.ps1 -ProjectRoot "D:\path\to\Ribbon_Engine"
```

The helper skips identical files and backs up files it replaces. It does not delete files.

## v4 changes

- The Render3D prefab path round-trip test now explicitly loads `models/room.glb` into the restore-side `AssetServer` before invoking prefab restore. `PrefabCodecRegistry` deliberately requires CPU-ready dependencies and does not load synchronously during ECS restoration; the test now follows the same prerequisite contract as SceneDoc streaming.
- Generic GPU warmup queue processing uses `while let Some(...)` instead of a `loop` + `let-else` break, resolving the Clippy `while_let_loop` diagnostic without changing upload-budget checks or accounting.

## Recommended validation

```powershell
cargo test -p ribbon_plugin_render3d prefab::tests::model_instance_round_trips_as_a_path_not_a_runtime_handle -- --exact
cargo test -p ribbon_plugin_render3d --all-targets
cargo clippy -p ribbon_plugin_render3d --all-targets -- -D warnings
cargo test --workspace --no-fail-fast
cargo clippy --workspace --all-targets -- -D warnings
py -3 tools/check_deps.py
```

Then run the complete validation script:

```powershell
.\tools\validate_streaming.ps1
```

## Validation honesty

The preceding Windows run is recorded in `WORLD_STREAMING_RENDER3D_FIX_REPORT.md`. The v4 source edits have not been compiled in the artifact-preparation environment because it does not provide Rust/Cargo or PowerShell. Do not treat v4 as passing until the Windows commands above succeed.


## Gap-completion scope and current validation

The cumulative overlay now includes Region phase traces and slowest-attempt CSV breakdown, generic GPU total/class admission budgets with lease-safe eviction/rejection counters, same-handle SceneDoc revision invalidation, editor transition/World Partition panels, and configurable 20-cycle/512-region stress runs with optional real-GPU pressure tests. It now stages non-preview Region visuals and SceneDoc prefab entities until that Region reaches its Active barrier, then reveals the Region batch before render extraction. This is Region-local, not a multi-Region Scene-wide transaction. It still does not add unsupported shader/pipeline/animation resource types or pretend to implement complete device recreation. The latest gap-completion and Region visibility-staging source delta has not been compiled/tested in this preparation environment; run `tools/validate_streaming.ps1` on Windows and the two soak commands in [`GAP_COMPLETION_REPORT.md`](../../streaming/GAP_COMPLETION_REPORT.md).


