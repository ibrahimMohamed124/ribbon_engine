# Ribbon Engine — Streaming Test Fixture Fix v3

## Observed Windows results before this patch

- `cargo check --workspace --all-targets`: PASS.
- `cargo test -p ribbon_scene --all-targets`: 9 passed, 0 failed.
- `cargo test -p ribbon_plugin_streaming --all-targets`: 34 passed, 8 failed.
- All 8 failures panic at `required world resource ribbon_plugin_streaming::asset_adapters::StreamingAssetAdapters is missing`.
- The Streaming test target reports 7 groups of unused prefab imports.

## Root cause

`StreamingPlugin::build` installs `StreamingAssetAdapters` into the application world. The affected unit tests instantiate `StreamingCoordinator` directly with `World::new()` and invoke `tick()` without installing the same plugin-owned resource. Coordinator behavior therefore panics in the test fixture before the intended state-machine assertion runs.

## Changes in v3

- Added one shared `test_world() -> World` fixture in `coordinator.rs` that inserts `StreamingAssetAdapters::default()`.
- Updated all 17 direct test world constructions to use the fixture.
- Removed seven stale `ribbon_scene` import groups that caused unused-import warnings; retained `Transform3D` imports used by those tests.
- No production runtime logic or `StreamingPlugin::build` registration behavior changed.

## Verification required

Run on the Windows repository after applying the overlay:

```powershell
cargo test -p ribbon_plugin_streaming --all-targets
cargo clippy -p ribbon_plugin_streaming --all-targets -- -D warnings
cargo test --workspace --no-fail-fast
cargo clippy --workspace --all-targets -- -D warnings
.\tools\validate_streaming.ps1
```

The v3 patch has not been compiled in the artifact-preparation environment because Cargo/Rust is unavailable there. A passing result must be established by the Windows rerun.
