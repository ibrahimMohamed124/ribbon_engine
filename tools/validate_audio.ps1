$ErrorActionPreference = 'Continue'
$repoRoot = Split-Path -Parent $PSScriptRoot
Push-Location $repoRoot

$checks = @(
    @{ Name = 'Resolve dependencies and check audio targets (also updates Cargo.lock)'; Args = @('check', '-p', 'ribbon_plugin_audio', '--all-targets') },
    @{ Name = 'Apply standard Rust formatting'; Args = @('fmt', '--all') },
    @{ Name = 'Formatting verification'; Args = @('fmt', '--all', '--check') },
    @{ Name = 'Audio crate: all targets and all features'; Args = @('test', '-p', 'ribbon_plugin_audio', '--all-targets', '--all-features') },
    @{ Name = 'Audio crate: no default features'; Args = @('test', '-p', 'ribbon_plugin_audio', '--no-default-features') },
    @{ Name = 'Audio crate: no-default-features all-target compile'; Args = @('check', '-p', 'ribbon_plugin_audio', '--no-default-features', '--all-targets') },
    @{ Name = 'Audio example compile'; Args = @('check', '-p', 'ribbon_plugin_audio', '--examples') },
    @{ Name = 'STUDS audio integration tests'; Args = @('test', '-p', 'studs', '--test', 'audio') },
    @{ Name = 'Workspace all-target check'; Args = @('check', '--workspace', '--all-targets') },
    @{ Name = 'Workspace all-target tests'; Args = @('test', '--workspace', '--all-targets') },
    @{ Name = 'Audio showcase smoke run (audio device optional)'; Args = @('run', '-p', 'ribbon_plugin_audio', '--example', 'audio_engine_showcase') }
)

$results = @()
foreach ($check in $checks) {
    Write-Host "`n============================================================" -ForegroundColor Cyan
    Write-Host ("RUNNING: " + $check.Name) -ForegroundColor Cyan
    Write-Host ("cargo " + ($check.Args -join ' '))
    $cargoArgs = $check.Args
    & cargo @cargoArgs
    $exitCode = $LASTEXITCODE
    $status = if ($exitCode -eq 0) { 'PASS' } else { 'FAIL' }
    $results += [PSCustomObject]@{ Status = $status; ExitCode = $exitCode; Check = $check.Name }
    Write-Host ("RESULT: {0} (exit {1})" -f $status, $exitCode) -ForegroundColor $(if ($exitCode -eq 0) { 'Green' } else { 'Red' })
}

Write-Host "`n===================== FINAL VALIDATION =====================" -ForegroundColor Cyan
$results | Format-Table -AutoSize
$failed = @($results | Where-Object { $_.Status -ne 'PASS' })
Pop-Location
if ($failed.Count -gt 0) {
    Write-Host ("{0} validation command(s) failed. Review the first compiler/test error above." -f $failed.Count) -ForegroundColor Red
    exit 1
}
Write-Host 'All requested validation commands passed.' -ForegroundColor Green
exit 0
