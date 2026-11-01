# Ribbon Engine — Streaming Validation Hotfix (2026-11-01)

This hotfix applies to the already-updated Ribbon Engine checkout containing the Production Streaming Overlay and the Render3D duplicate-import fix.

## Included source fixes

1. `plugins/streaming/src/coordinator.rs`
   - Publishes a completed Region activation's lifetime-maximum latency and slowest-region identity to aggregate diagnostics immediately, so `snapshot()` is correct even when read between coordinator ticks.
   - Makes the dormant-reactivation timing test deterministic by explicitly measuring its `PreparingPhysics` interval. The new activation attempt correctly excludes CPU/GPU reload time while retaining the physics-preparation duration.

2. `plugins/editor_tools/src/lib.rs`
   - Passes `&str` to the editor UI label API for the last GPU device-loss reason, resolving the reported `String` versus `&str` type mismatch.

## Apply

Extract this ZIP over the project root (preserve the directory structure), or copy the two files into the matching paths. It replaces only those two files and does not remove files.

## Validate

Run:

```powershell
cargo fmt --all -- --check
cargo check -p ribbon_plugin_streaming --all-targets
cargo check --workspace --all-targets
cargo test -p ribbon_plugin_streaming --all-targets
cargo test -p ribbon_plugin_editor_tools --all-targets
cargo test -p athar --all-targets
cargo test --workspace --no-fail-fast
cargo clippy --workspace --all-targets -- -D warnings
py -3 tools/check_deps.py
.\tools\validate_streaming.ps1
```

## Validation honesty

The source was statically inspected in the artifact-preparation environment. Rust/Cargo/rustfmt are not installed there, so this hotfix has NOT been compiled or tested here. Do not treat the hotfix as passing until the Windows validation commands complete successfully.
