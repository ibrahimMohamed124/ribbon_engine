# Ribbon Engine — Streaming Stress Harness Clippy Fix

This is a two-file follow-up patch for the streaming stress harness. Apply it on top of the previously installed Streaming Stress Improvements overlay.

## Apply

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\apply_streaming_stress_clippy_fix.ps1 -ProjectRoot "D:\path\to\Ribbon_Engine"
```

The installer validates both source hashes before modifying the project, backs up the replaced files under `backups/streaming-stress-clippy-<timestamp>`, copies the two files, then verifies their SHA-256 hashes.

## Fixes

- Replace a redundant forwarding closure with the function item directly.
- Replace `chunks_exact_mut(4)` with checked four-byte slice chunking via `as_chunks_mut::<4>()`.
- Use `usize::is_multiple_of(2)` for the route direction parity check.
- Collapse consecutive newline/carriage-return replacements into one multi-pattern replacement.

No runtime behavior is intentionally changed. The pixel buffer remains required to be a multiple of four bytes; a debug assertion documents that invariant.

## Validate

```powershell
cargo fmt --all
cargo fmt --all -- --check
cargo clippy -p ribbon_streaming_stress --all-targets -- -D warnings
cargo test -p ribbon_streaming_stress --all-targets
```

The supplied prior log shows the `ribbon_plugin_gltf`, `ribbon_plugin_render3d`, and `ribbon_streaming_stress` checks/tests passed. Clippy failed only on these four lints. This patch has not been compiled or run by Cargo/Clippy in the artifact-preparation environment.
