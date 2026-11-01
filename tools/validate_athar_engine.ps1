$ErrorActionPreference = 'Stop'

# wgpu/naga/Rapier can compile concurrently with other large crates in this workspace.
# Serial compilation plus disabled debug info for validation avoids the OOM/cascading E0463
# errors seen during `cargo test --workspace` on memory-constrained machines.
$env:CARGO_BUILD_JOBS = '1'
$env:CARGO_PROFILE_DEV_DEBUG = '0'
$env:CARGO_PROFILE_TEST_DEBUG = '0'

function Invoke-CargoStep {
    param([Parameter(Mandatory = $true)][string[]]$Arguments)
    Write-Host "`n> cargo $($Arguments -join ' ')" -ForegroundColor Cyan
    & cargo @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "cargo $($Arguments -join ' ') failed with exit code $LASTEXITCODE"
    }
}

Invoke-CargoStep @('fmt', '--all', '--', '--check')
Invoke-CargoStep @('check', '-p', 'athar')
Invoke-CargoStep @('test', '-p', 'athar')
Invoke-CargoStep @('test', '--workspace', '--no-fail-fast')
Invoke-CargoStep @('clippy', '-p', 'athar', '--all-targets', '--', '-D', 'warnings')

Write-Host "`nATHAR targeted checks and workspace tests passed." -ForegroundColor Green
