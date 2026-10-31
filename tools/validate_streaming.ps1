[CmdletBinding()]
param(
    [switch]$SkipWorkspaceTests,
    [switch]$SkipClippy
)

$ErrorActionPreference = 'Continue'
$repoRoot = Split-Path -Parent $PSScriptRoot
Set-Location -LiteralPath $repoRoot

$checks = @(
    @{ Name = 'Rust formatting'; Run = { cargo fmt --all -- --check } },
    @{ Name = 'Streaming plugin all-target check'; Run = { cargo check -p ribbon_plugin_streaming --all-targets } },
    @{ Name = 'Workspace all-target check'; Run = { cargo check --workspace --all-targets } },
    @{ Name = 'AssetServer tests'; Run = { cargo test -p ribbon_assets } },
    @{ Name = 'Scene tests'; Run = { cargo test -p ribbon_scene } },
    @{ Name = 'Render core profiling tests'; Run = { cargo test -p ribbon_render } },
    @{ Name = 'glTF model upload estimate tests'; Run = { cargo test -p ribbon_plugin_gltf } },
    @{ Name = 'Render3D tests'; Run = { cargo test -p ribbon_plugin_render3d } },
    @{ Name = 'Editor tools tests'; Run = { cargo test -p ribbon_plugin_editor_tools } },
    @{ Name = 'Physics tests'; Run = { cargo test -p ribbon_plugin_physics } },
    @{ Name = 'Streaming coordinator tests'; Run = { cargo test -p ribbon_plugin_streaming } },
    @{ Name = 'ATHAR tests'; Run = { cargo test -p athar } }
)

if (-not $SkipWorkspaceTests) {
    $checks += @{ Name = 'Workspace tests'; Run = { cargo test --workspace --no-fail-fast } }
}

if (-not $SkipClippy) {
    $checks += @(
        @{ Name = 'Workspace Clippy (all targets, warnings denied)'; Run = { cargo clippy --workspace --all-targets -- -D warnings } }
    )
}

$checks += @{ Name = 'Dependency policy'; Run = { py -3 tools/check_deps.py } }

$results = [System.Collections.Generic.List[object]]::new()
foreach ($check in $checks) {
    Write-Host ''
    Write-Host ('=' * 64)
    Write-Host ('RUNNING: ' + $check.Name)
    try {
        & $check.Run
        $code = $LASTEXITCODE
        if ($null -eq $code) { $code = 0 }
    }
    catch {
        Write-Host $_ -ForegroundColor Red
        $code = 1
    }
    $status = if ($code -eq 0) { 'PASS' } else { 'FAIL' }
    Write-Host ("RESULT: {0} (exit {1})" -f $status, $code)
    $results.Add([pscustomobject]@{ Status = $status; ExitCode = $code; Check = $check.Name })
}

Write-Host ''
Write-Host '===================== FINAL VALIDATION ====================='
$results | Format-Table -AutoSize
$failed = @($results | Where-Object { $_.Status -ne 'PASS' })
if ($failed.Count -gt 0) {
    Write-Host ("{0} validation command(s) failed." -f $failed.Count) -ForegroundColor Red
    exit 1
}
Write-Host 'All streaming validation commands passed.' -ForegroundColor Green
