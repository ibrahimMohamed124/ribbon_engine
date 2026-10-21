$ErrorActionPreference = 'Stop'
$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Push-Location $RepoRoot
try {
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        throw 'cargo was not found. Install/select the Rust toolchain before validating this workspace.'
    }
    if (-not (Get-Command python -ErrorAction SilentlyContinue)) {
        throw 'python was not found. Install Python 3 before running tools/check_deps.py.'
    }

    # Keep the commands serial and non-incremental to reduce memory pressure on Windows.
    $env:CARGO_INCREMENTAL = '0'
    Write-Host '== Rust formatting ==' -ForegroundColor Cyan
    cargo fmt --all -- --check
    if ($LASTEXITCODE -ne 0) { throw "cargo fmt --all -- --check failed with exit code $LASTEXITCODE" }

    Write-Host '== Clippy (all targets, warnings denied) ==' -ForegroundColor Cyan
    cargo clippy --workspace --all-targets -j 1 -- -D warnings
    if ($LASTEXITCODE -ne 0) { throw "cargo clippy failed with exit code $LASTEXITCODE" }

    Write-Host '== Workspace tests (default features) ==' -ForegroundColor Cyan
    cargo test --workspace -j 1
    if ($LASTEXITCODE -ne 0) { throw "cargo test --workspace failed with exit code $LASTEXITCODE" }

    Write-Host '== Workspace tests (no default features) ==' -ForegroundColor Cyan
    cargo test --workspace --no-default-features -j 1
    if ($LASTEXITCODE -ne 0) { throw "cargo test --workspace --no-default-features failed with exit code $LASTEXITCODE" }

    Write-Host '== Dependency boundary check ==' -ForegroundColor Cyan
    python tools/check_deps.py
    if ($LASTEXITCODE -ne 0) { throw "tools/check_deps.py failed with exit code $LASTEXITCODE" }

    Write-Host '== Native horror_slice interactive run ==' -ForegroundColor Cyan
    Write-Host 'Close the game window normally after checking room orientation, floor/wall textures, key, locked door, save lantern, monster silhouette and flashlight shadows.' -ForegroundColor Yellow
    cargo run -p horror_slice
    if ($LASTEXITCODE -ne 0) { throw "horror_slice exited with code $LASTEXITCODE" }

    $Screenshot = Join-Path $RepoRoot 'target/spotlight_corridor.png'
    if (Test-Path $Screenshot) {
        Write-Host "Spotlight corridor screenshot found: $Screenshot" -ForegroundColor Green
    } else {
        Write-Host 'No target/spotlight_corridor.png found yet. The headless screenshot test may have skipped because no usable GPU adapter was available; inspect cargo test output.' -ForegroundColor Yellow
    }
    Write-Host 'Validation command sequence finished. Interactive visual review is still a human/manual acceptance check.' -ForegroundColor Green
}
finally {
    Pop-Location
}
