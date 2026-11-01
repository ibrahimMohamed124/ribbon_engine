# Apply the Ribbon Streaming Stress Harness

This overlay adds a self-contained engine example. It does not create or modify a game.

## Apply on Windows

1. Extract this ZIP into a temporary folder, preserving its directory structure.
2. From PowerShell, run:

```powershell
powershell -ExecutionPolicy Bypass -File .\apply_streaming_stress_examples.ps1 -ProjectRoot "D:\path\to\Ribbon_Engine"
```

The apply script copies only the ten listed project files and makes timestamped backups of any files it replaces under `backups/streaming-stress-harness/` in the project root.

## Validate and run

```powershell
cargo fmt --all -- --check
cargo check -p ribbon_streaming_stress --all-targets
cargo test -p ribbon_streaming_stress --all-targets
cargo clippy -p ribbon_streaming_stress --all-targets -- -D warnings
py -3 tools/check_deps.py
```

Then run the complete headless + real-GPU stress pass:

```powershell
.\examples\streaming_stress\run_streaming_stress.ps1
```

For a machine without an available graphics adapter/display, run the CPU-only portion:

```powershell
.\examples\streaming_stress\run_streaming_stress.ps1 -HeadlessOnly
```

The full run opens a native GPU window. Keep it visible and open until it exits. Cargo build, GLB generation, headless and GPU phases are tracked separately with stdout/stderr and process-memory samples. Output is written under `target/streaming-stress-runs/<timestamp>/`. `-TailFrames` must be at least 48 to give region and physics cleanup a bounded settle phase.

GPU memory is reported as renderer allocation estimates, not exact physical VRAM. Headless mode intentionally reports GPU values as unavailable rather than simulated.

## PowerShell runner parser fix (v3)

The v3 runner uses single-quoted PowerShell literals for Markdown summary lines. This avoids PowerShell interpreting Markdown backticks as escape characters (notably a closing backtick immediately before a double quote), which caused a parser error near the final artifact list in v2. No runtime test semantics or engine code changed in this fix.
