[CmdletBinding()]
param(
    [switch]$SkipWorkspaceTests,
    [switch]$SkipClippy,
    [switch]$StopOnFailure
)

$ErrorActionPreference = 'Continue'
$repoRoot = Split-Path -Parent $PSScriptRoot
Set-Location -LiteralPath $repoRoot

# Catch partial overlay application before Cargo reports a confusing missing-field cascade.
$renderStatsPath = Join-Path $repoRoot 'crates/render/src/profiling/types.rs'
if (-not (Test-Path -LiteralPath $renderStatsPath -PathType Leaf)) {
    throw "Missing renderer diagnostics source: $renderStatsPath. Reapply the complete world-streaming overlay."
}
$renderStatsSource = Get-Content -LiteralPath $renderStatsPath -Raw
$requiredStatsFields = @(
    'live_streamed_textures',
    'live_streamed_samplers',
    'streamed_texture_bytes_estimate',
    'generic_gpu_warmup_queued',
    'generic_gpu_warmup_preparing',
    'generic_gpu_warmup_ready',
    'generic_gpu_warmup_failed',
    'generic_gpu_uploads_cumulative',
    'generic_gpu_upload_failures_cumulative',
    'generic_gpu_releases_cumulative',
    'generic_gpu_uploads_this_frame',
    'generic_gpu_upload_bytes_this_frame',
    'streaming_max_activation_latency_ns',
    'streaming_slowest_region_id',
    'streaming_slowest_region_requested_ns',
    'streaming_slowest_region_cpu_loading_ns',
    'streaming_slowest_region_gpu_warmup_ns',
    'streaming_slowest_region_ready_wait_ns',
    'streaming_slowest_region_physics_ns',
    'streaming_slowest_region_activation_ns',
    'streaming_slowest_region_current_state_ns',
    'streaming_slowest_region_last_activation_ns',
    'generic_gpu_resident_bytes_estimate',
    'generic_gpu_queued_bytes_estimate',
    'generic_gpu_budget_overage_bytes',
    'generic_gpu_evictions_cumulative',
    'generic_gpu_memory_pressure_events',
    'generic_gpu_budget_rejections_cumulative',
    'generic_gpu_sampler_count_over_budget'
)
$missingStatsFields = @($requiredStatsFields | Where-Object {
    $renderStatsSource -notmatch ('(?m)^\s*pub\s+' + [regex]::Escape($_) + '\s*:')
})
if ($missingStatsFields.Count -gt 0) {
    throw ("Renderer diagnostics schema is incomplete in {0}: {1}. Reapply the complete world-streaming overlay." -f $renderStatsPath, ($missingStatsFields -join ', '))
}

$coordinatorPath = Join-Path $repoRoot 'plugins/streaming/src/coordinator.rs'
$coordinatorSource = Get-Content -LiteralPath $coordinatorPath -Raw
$requiredCoordinatorMarkers = @(
    'pub struct RegionActivationBreakdown',
    'pub elapsed_ns: u64',
    'scene_doc_revision_changed',
    'slowest_activation'
)
$missingCoordinatorMarkers = @($requiredCoordinatorMarkers | Where-Object { $coordinatorSource -notmatch [regex]::Escape($_) })
if ($missingCoordinatorMarkers.Count -gt 0) {
    throw ("Streaming lifecycle source is incomplete in {0}: {1}. Reapply the complete world-streaming overlay." -f $coordinatorPath, ($missingCoordinatorMarkers -join ', '))
}

$resultsRoot = Join-Path $repoRoot 'test-results'
New-Item -ItemType Directory -Force -Path $resultsRoot | Out-Null
$logPath = Join-Path $resultsRoot 'streaming-validation.log'
$summaryPath = Join-Path $resultsRoot 'streaming-validation-summary.csv'
if (Test-Path -LiteralPath $logPath) { Remove-Item -LiteralPath $logPath -Force }
if (Test-Path -LiteralPath $summaryPath) { Remove-Item -LiteralPath $summaryPath -Force }

$checks = @(
    @{ Name = 'Rust formatting'; Executable = 'cargo'; Arguments = @('fmt', '--all', '--', '--check') },
    @{ Name = 'Streaming plugin all-target check'; Executable = 'cargo'; Arguments = @('check', '-p', 'ribbon_plugin_streaming', '--all-targets') },
    @{ Name = 'Workspace all-target check'; Executable = 'cargo'; Arguments = @('check', '--workspace', '--all-targets') },
    @{ Name = 'AssetServer tests'; Executable = 'cargo'; Arguments = @('test', '-p', 'ribbon_assets', '--all-targets') },
    @{ Name = 'Scene and Prefab tests'; Executable = 'cargo'; Arguments = @('test', '-p', 'ribbon_scene', '--all-targets') },
    @{ Name = 'Render profiling tests'; Executable = 'cargo'; Arguments = @('test', '-p', 'ribbon_render', '--all-targets') },
    @{ Name = 'glTF asset tests'; Executable = 'cargo'; Arguments = @('test', '-p', 'ribbon_plugin_gltf', '--all-targets') },
    @{ Name = 'Render3D and GPU warmup tests'; Executable = 'cargo'; Arguments = @('test', '-p', 'ribbon_plugin_render3d', '--all-targets') },
    @{ Name = 'Physics and Prefab tests'; Executable = 'cargo'; Arguments = @('test', '-p', 'ribbon_plugin_physics', '--all-targets') },
    @{ Name = 'Streaming coordinator tests'; Executable = 'cargo'; Arguments = @('test', '-p', 'ribbon_plugin_streaming', '--all-targets') },
    @{ Name = 'Editor streaming diagnostics tests'; Executable = 'cargo'; Arguments = @('test', '-p', 'ribbon_plugin_editor_tools', '--all-targets') },
    @{ Name = 'ATHAR integration tests'; Executable = 'cargo'; Arguments = @('test', '-p', 'athar', '--all-targets') }
)

if (-not $SkipWorkspaceTests) {
    $checks += @{ Name = 'Workspace tests'; Executable = 'cargo'; Arguments = @('test', '--workspace', '--no-fail-fast') }
}

if (-not $SkipClippy) {
    $checks += @{ Name = 'Workspace Clippy (all targets, warnings denied)'; Executable = 'cargo'; Arguments = @('clippy', '--workspace', '--all-targets', '--', '-D', 'warnings') }
}

$checks += @{ Name = 'Dependency policy'; Executable = 'py'; Arguments = @('-3', 'tools/check_deps.py') }

$results = [System.Collections.Generic.List[object]]::new()
$startedAll = Get-Date

foreach ($check in $checks) {
    $name = $check.Name
    $executable = $check.Executable
    $arguments = $check.Arguments
    $display = $executable + ' ' + ($arguments -join ' ')
    $outputPath = Join-Path $resultsRoot 'streaming-current-command.stdout.tmp'
    $errorPath = Join-Path $resultsRoot 'streaming-current-command.stderr.tmp'
    foreach ($temporaryPath in @($outputPath, $errorPath)) {
        if (Test-Path -LiteralPath $temporaryPath) {
            Remove-Item -LiteralPath $temporaryPath -Force
        }
    }

    Write-Host ''
    Write-Host ('=' * 72) -ForegroundColor Cyan
    Write-Host ('RUNNING: ' + $name) -ForegroundColor Yellow
    Write-Host ('COMMAND: ' + $display) -ForegroundColor DarkGray

    Add-Content -LiteralPath $logPath -Value "`r`n======================================================================"
    Add-Content -LiteralPath $logPath -Value "CHECK: $name"
    Add-Content -LiteralPath $logPath -Value "COMMAND: $display"
    Add-Content -LiteralPath $logPath -Value "STARTED: $((Get-Date).ToString('o'))"

    $timer = [System.Diagnostics.Stopwatch]::StartNew()
    $exitCode = 1
    $status = 'FAIL'
    $message = $null

    $commandInfo = Get-Command $executable -ErrorAction SilentlyContinue
    if ($null -eq $commandInfo) {
        $message = "Required executable '$executable' was not found in PATH."
        Write-Host $message -ForegroundColor Red
        Add-Content -LiteralPath $logPath -Value $message
        $status = 'UNAVAILABLE'
    }
    else {
        try {
            # Start-Process captures native stderr as text rather than turning every stderr
            # line into a PowerShell NativeCommandError record. Its ExitCode is authoritative.
            $process = Start-Process `
                -FilePath $commandInfo.Source `
                -ArgumentList $arguments `
                -WorkingDirectory $repoRoot `
                -NoNewWindow `
                -Wait `
                -PassThru `
                -RedirectStandardOutput $outputPath `
                -RedirectStandardError $errorPath
            $exitCode = $process.ExitCode
            $status = if ($exitCode -eq 0) { 'PASS' } else { 'FAIL' }
        }
        catch {
            $message = $_.Exception.Message
            $exitCode = 1
            $status = 'ERROR'
        }

        foreach ($capturedPath in @($outputPath, $errorPath)) {
            if (Test-Path -LiteralPath $capturedPath) {
                Get-Content -LiteralPath $capturedPath | Tee-Object -FilePath $logPath -Append
                Remove-Item -LiteralPath $capturedPath -Force
            }
        }
        if ($message) {
            Write-Host $message -ForegroundColor Red
            Add-Content -LiteralPath $logPath -Value $message
        }
    }

    $timer.Stop()
    $duration = [math]::Round($timer.Elapsed.TotalSeconds, 2)
    Add-Content -LiteralPath $logPath -Value "STATUS: $status"
    Add-Content -LiteralPath $logPath -Value "EXIT CODE: $exitCode"
    Add-Content -LiteralPath $logPath -Value "DURATION_SECONDS: $duration"

    if ($status -eq 'PASS') {
        Write-Host ("PASS ({0}s)" -f $duration) -ForegroundColor Green
    }
    else {
        Write-Host ("{0} (exit {1})" -f $status, $exitCode) -ForegroundColor Red
    }

    $results.Add([pscustomobject]@{
        Check = $name
        Command = $display
        Status = $status
        ExitCode = $exitCode
        DurationSeconds = $duration
    })

    if ($StopOnFailure -and $status -ne 'PASS') {
        Write-Host 'Stopping after the first failed or unavailable check.' -ForegroundColor Red
        break
    }
}

$results | Export-Csv -LiteralPath $summaryPath -NoTypeInformation -Encoding UTF8
$passed = @($results | Where-Object { $_.Status -eq 'PASS' }).Count
$failed = $results.Count - $passed
$totalSeconds = [math]::Round(((Get-Date) - $startedAll).TotalSeconds, 2)

Write-Host ''
Write-Host '======================= STREAMING VALIDATION =======================' -ForegroundColor Cyan
$results | Format-Table Check, Status, ExitCode, DurationSeconds -AutoSize
Write-Host ("Passed: {0} / {1}" -f $passed, $results.Count)
Write-Host ("Total duration: {0}s" -f $totalSeconds)
Write-Host "Log: $logPath"
Write-Host "Summary: $summaryPath"

if ($failed -gt 0) { exit 1 }
exit 0
