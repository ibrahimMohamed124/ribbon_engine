# Validation follow-up — 2026-10-24

## Results reported from the Windows checkout

The latest local command output reports:

- `cargo fmt --all -- --check`: passed.
- `cargo check --locked --workspace --all-targets`: passed.
- `cargo build --locked --workspace --examples`: passed.
- `cargo clippy --locked --workspace --all-targets -- -D warnings`: failed on `RenderDiagnostics::record_gpu_timing` with `clippy::too_many_arguments` (10 input fields plus `&mut self`).
- `cargo test --locked --workspace`, `cargo build --locked --workspace --all-targets --no-default-features`, and `cargo test --locked --workspace --no-default-features`: reported multiple `E0463` missing-crate errors and incompatible build-artifact diagnostics, including `found staticlib std instead of rlib or dylib`, an `ash` metadata-version mismatch, and unavailable `rlib` forms for dependencies.

## Source fix

`RenderDiagnostics::record_gpu_timing` now accepts one `RenderGpuTimingSummary` value instead of ten independent arguments. This groups together values that describe one published diagnostics snapshot and resolves the specific Clippy argument-count warning without suppressing the lint. The render3d call site constructs that value while preserving the two independent sequence numbers for the mesh/shadow and post-processing profilers.

## How to revalidate safely

The error output does not establish the root cause of the workspace test/build artifact failures. Because the commands reused the checkout's shared `target` directory and some diagnostics indicate mismatched compiled artifacts, validate again with isolated target directories. Do not delete the existing `target` directory: it contains performance captures and other retained artifacts.

Run from the repository root in PowerShell:

```powershell
$env:CARGO_BUILD_JOBS = '2'
$env:CARGO_TARGET_DIR = Join-Path $env:TEMP 'ribbon-engine-validation-default'

cargo fmt --all -- --check
if ($LASTEXITCODE -ne 0) { throw 'rustfmt failed' }

cargo check --locked --workspace --all-targets
if ($LASTEXITCODE -ne 0) { throw 'workspace check failed' }

cargo clippy --locked --workspace --all-targets -- -D warnings
if ($LASTEXITCODE -ne 0) { throw 'workspace Clippy failed' }

cargo build --locked --workspace --examples
if ($LASTEXITCODE -ne 0) { throw 'workspace example build failed' }

cargo test --locked --workspace
if ($LASTEXITCODE -ne 0) { throw 'workspace tests failed' }

$env:CARGO_TARGET_DIR = Join-Path $env:TEMP 'ribbon-engine-validation-no-default'
cargo build --locked --workspace --all-targets --no-default-features
if ($LASTEXITCODE -ne 0) { throw 'no-default-features build failed' }

cargo test --locked --workspace --no-default-features
if ($LASTEXITCODE -ne 0) { throw 'no-default-features tests failed' }

Remove-Item Env:CARGO_TARGET_DIR
Remove-Item Env:CARGO_BUILD_JOBS
```

The isolated directories are outside the repository's existing `target` folder so the recorded benchmark captures are preserved. If the missing-crate/staticlib diagnostics recur with these fresh directories, capture the first failing command's complete output (preferably with `cargo -vv`) and inspect the actual Rust toolchain selection and environment (`rustc --version --verbose`, `cargo --version`, `Get-Command cargo`, `Get-Command rustc`, `Get-ChildItem Env:RUST*`). Do not assume that the errors are merely stale artifacts until this clean-target comparison is complete.

## Limitations

This follow-up is based on the command output supplied by the developer and a source-level change. Rust tooling is not installed in the review environment, so the Clippy fix and isolated-target validation sequence have not been compiled or executed here. The workspace must not be treated as release-ready until Clippy, default-feature tests, no-default-feature builds/tests, and runtime/visual checks pass on the target checkout.
