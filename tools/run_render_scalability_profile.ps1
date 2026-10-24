$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$out = Join-Path $repo "target/profiles/render-scalability/$stamp"
$captures = Join-Path $out 'captures'
New-Item -ItemType Directory -Force -Path $captures | Out-Null
$exe = Join-Path $repo 'target/release/graphics_showcase.exe'
$frames = 600
$warmup = 60
$expectedRows = $frames - $warmup + 1
$revision = (git -C $repo rev-parse HEAD).Trim()
$dirty = [bool](git -C $repo status --porcelain)
$exeHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $exe).Hash
$scenes = @(
    [pscustomobject]@{ Name = 'small'; Side = 8; Instances = 64 },
    [pscustomobject]@{ Name = 'medium'; Side = 24; Instances = 576 },
    [pscustomobject]@{ Name = 'large'; Side = 40; Instances = 1600 }
)
$manifest = Join-Path $out 'capture-manifest.md'
@(
    '# Rendering scalability capture manifest',
    '',
    "- UTC start: $([DateTime]::UtcNow.ToString('o'))",
    "- Source revision: ``$revision``; dirty at capture: $dirty",
    "- Executable: ``$exe``",
    "- Executable SHA-256: ``$exeHash``",
    "- Profile: release; resolution configured by showcase: 1280x800",
    "- Frames: $frames; warmup: $warmup; expected retained frame rows per run: $expectedRows",
    '- Stress workload is static-camera, one shared checker model/material, one sun, fixed transforms, no lifecycle cycling.',
    '- Scene sizes are 8x8 (64 placements), 24x24 (576), and 40x40 (1,600).',
    '- FIFO and Mailbox requested with desired maximum frame latency 1; actual mode and adapter recorded in logs.',
    '- Five repetitions per scene/presentation combination, with scene/mode order rotated per repetition.',
    '',
    '| Order | Scene | Placements | Mode | CSV | Log | Report | Exit | Valid rows |',
    '|---:|---|---:|---|---|---|---|---:|---:|'
) | Set-Content -LiteralPath $manifest -Encoding utf8

$previousMode = $env:RIBBON_GPU_PRESENT_MODE
$previousLatency = $env:RIBBON_GPU_FRAME_LATENCY
$hadMode = Test-Path Env:RIBBON_GPU_PRESENT_MODE
$hadLatency = Test-Path Env:RIBBON_GPU_FRAME_LATENCY
try {
    $order = 0
    for ($repeat = 1; $repeat -le 5; $repeat++) {
        $sequence = [System.Collections.Generic.List[object]]::new()
        $base = @(
            [pscustomobject]@{ Scene = $scenes[0]; Mode = 'fifo' },
            [pscustomobject]@{ Scene = $scenes[1]; Mode = 'mailbox' },
            [pscustomobject]@{ Scene = $scenes[2]; Mode = 'fifo' },
            [pscustomobject]@{ Scene = $scenes[0]; Mode = 'mailbox' },
            [pscustomobject]@{ Scene = $scenes[1]; Mode = 'fifo' },
            [pscustomobject]@{ Scene = $scenes[2]; Mode = 'mailbox' }
        )
        if (($repeat % 2) -eq 0) { [array]::Reverse($base) }
        foreach ($item in $base) { $sequence.Add($item) }

        foreach ($item in $sequence) {
            $order++
            $scene = $item.Scene
            $mode = $item.Mode
            $stem = '{0}-{1}-run{2:D2}' -f $scene.Name, $mode, $repeat
            $csv = Join-Path $captures "$stem.csv"
            $log = Join-Path $captures "$stem.log"
            $report = Join-Path $captures "$stem.md"
            $args = @('--stress-grid', [string]$scene.Side, '--profile-static-scene', '--frames', [string]$frames, '--profile-warmup', [string]$warmup, '--profile-csv', $csv)
            $command = '"' + $exe + '" ' + ($args -join ' ')
            @(
                "capture=$stem",
                "order=$order",
                "repeat=$repeat",
                "started_utc=$([DateTime]::UtcNow.ToString('o'))",
                "source_revision=$revision",
                "source_dirty=$dirty",
                'variant=current-render-scalability-instrumented',
                "executable=$exe",
                "executable_sha256=$exeHash",
                "RIBBON_GPU_PRESENT_MODE=$mode",
                'RIBBON_GPU_FRAME_LATENCY=1',
                "resolution=1280x800",
                "scene=$($scene.Name);grid=$($scene.Side)x$($scene.Side);model_placements=$($scene.Instances);distinct_models=1",
                "warmup_frames=$warmup",
                "requested_total_frames=$frames",
                "expected_frame_rows=$expectedRows",
                "command=$command",
                'exit_code=pending',
                ''
            ) | Set-Content -LiteralPath $log -Encoding utf8

            $env:RIBBON_GPU_PRESENT_MODE = $mode
            $env:RIBBON_GPU_FRAME_LATENCY = '1'
            Push-Location $repo
            try {
                & $exe @args 2>&1 | Tee-Object -FilePath $log -Append
                $code = $LASTEXITCODE
            }
            finally { Pop-Location }
            Add-Content -LiteralPath $log -Value "capture_process_exit_code=$code"
            if ($code -ne 0) {
                Add-Content -LiteralPath $manifest -Value "| $order | $($scene.Name) | $($scene.Instances) | $mode | ``$csv`` | ``$log`` | — | $code | 0 |"
                throw "Capture process failed: $stem (exit $code)"
            }

            $frameRows = @(Import-Csv -LiteralPath $csv | Where-Object record_type -eq 'frame')
            $valid = $frameRows.Count -eq $expectedRows
            if ($valid) {
                for ($i = 1; $i -lt $frameRows.Count; $i++) {
                    if ([long]$frameRows[$i].frame -ne ([long]$frameRows[$i - 1].frame + 1)) { $valid = $false; break }
                }
            }
            if (-not $valid) { throw "Invalid frame row count or noncontiguous frame numbers: $stem ($($frameRows.Count)/$expectedRows)" }
            $last = $frameRows[-1]
            if ($last.surface_present_mode -ne $mode -or [int]$last.surface_frame_latency -ne 1) {
                throw "Capture configuration mismatch: $stem (mode=$($last.surface_present_mode), latency=$($last.surface_frame_latency))"
            }
            if (-not [bool]::Parse($last.gpu_timestamp_queries_supported)) { throw "GPU timestamps unsupported in capture $stem" }

            & python (Join-Path $repo 'tools/render_profile_report.py') $csv --output $report
            if ($LASTEXITCODE -ne 0) { throw "Report generation failed: $stem" }
            Add-Content -LiteralPath $manifest -Value "| $order | $($scene.Name) | $($scene.Instances) | $mode | ``$csv`` | ``$log`` | ``$report`` | $code | $($frameRows.Count) |"
            Write-Host "[$order/30] $stem rows=$($frameRows.Count) exit=$code"
        }
    }
}
finally {
    if ($hadMode) { $env:RIBBON_GPU_PRESENT_MODE = $previousMode } else { Remove-Item Env:RIBBON_GPU_PRESENT_MODE -ErrorAction SilentlyContinue }
    if ($hadLatency) { $env:RIBBON_GPU_FRAME_LATENCY = $previousLatency } else { Remove-Item Env:RIBBON_GPU_FRAME_LATENCY -ErrorAction SilentlyContinue }
}

Write-Host "Capture artifacts: $out"
