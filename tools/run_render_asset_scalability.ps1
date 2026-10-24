$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$out = Join-Path $repo "target/profiles/render-scalability-assets/$stamp"
$captures = Join-Path $out 'captures'
New-Item -ItemType Directory -Force -Path $captures | Out-Null
$exe = Join-Path $repo 'target/release/graphics_showcase.exe'
$corpus = Join-Path $repo 'target/asset-stress-corpus'
$modelFiles = @(Get-ChildItem -LiteralPath (Join-Path $corpus 'models') -Filter 'stress_*.glb' | Sort-Object Name)
if ($modelFiles.Count -lt 32) { throw "Asset corpus requires at least 32 existing models; found $($modelFiles.Count)" }
$corpusManifestHash = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $corpus 'asset-corpus-manifest.md')).Hash
$frames = 600
$warmup = 60
$expectedRows = $frames - $warmup + 1
$revision = (git -C $repo rev-parse HEAD).Trim()
$dirty = [bool](git -C $repo status --porcelain)
$exeHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $exe).Hash
$assetCounts = @(1, 8, 32)
$manifest = Join-Path $out 'capture-manifest.md'
@(
    '# Rendering asset and texture scalability capture manifest', '',
    "- UTC start: $([DateTime]::UtcNow.ToString('o'))",
    "- Source revision: ``$revision``; dirty at capture: $dirty",
    "- Executable: ``$exe``; SHA-256: ``$exeHash``",
    "- Existing corpus: ``$corpus``; 64 distinct 128x128-texture GLBs; manifest SHA-256: ``$corpusManifestHash``",
    '- Workload fixed at 24x24 (576) placements, 1280x800, static camera/transforms/lighting, one sun; active unique GLB count varies 1/8/32.',
    "- Frames: $frames; warmup: $warmup; expected retained frame rows per run: $expectedRows",
    '- Release build; FIFO/Mailbox; desired maximum frame latency 1; five captures per combination with alternating order.', '',
    '| Order | Active assets | Mode | CSV | Log | Report | Exit | Valid rows |',
    '|---:|---:|---|---|---|---|---:|---:|'
) | Set-Content -LiteralPath $manifest -Encoding utf8
$previousMode = $env:RIBBON_GPU_PRESENT_MODE
$previousLatency = $env:RIBBON_GPU_FRAME_LATENCY
$hadMode = Test-Path Env:RIBBON_GPU_PRESENT_MODE
$hadLatency = Test-Path Env:RIBBON_GPU_FRAME_LATENCY
try {
    $order = 0
    for ($repeat = 1; $repeat -le 5; $repeat++) {
        $sequence = @(
            [pscustomobject]@{ Count = 1; Mode = 'fifo' },
            [pscustomobject]@{ Count = 8; Mode = 'mailbox' },
            [pscustomobject]@{ Count = 32; Mode = 'fifo' },
            [pscustomobject]@{ Count = 1; Mode = 'mailbox' },
            [pscustomobject]@{ Count = 8; Mode = 'fifo' },
            [pscustomobject]@{ Count = 32; Mode = 'mailbox' }
        )
        if (($repeat % 2) -eq 0) { [array]::Reverse($sequence) }
        foreach ($item in $sequence) {
            $order++
            $count = $item.Count
            $mode = $item.Mode
            $stem = 'assets{0:D2}-{1}-run{2:D2}' -f $count, $mode, $repeat
            $csv = Join-Path $captures "$stem.csv"
            $log = Join-Path $captures "$stem.log"
            $report = Join-Path $captures "$stem.md"
            $args = @('--stress-grid', '24', '--profile-static-scene', '--stress-multi-asset-count', [string]$count, '--frames', [string]$frames, '--profile-warmup', [string]$warmup, '--profile-csv', $csv)
            $command = '"' + $exe + '" ' + ($args -join ' ')
            @(
                "capture=$stem", "order=$order", "repeat=$repeat",
                "started_utc=$([DateTime]::UtcNow.ToString('o'))",
                "source_revision=$revision", "source_dirty=$dirty",
                'variant=current-render-asset-scalability-instrumented',
                "executable=$exe", "executable_sha256=$exeHash",
                "corpus_manifest_sha256=$corpusManifestHash", "corpus_model_count=$($modelFiles.Count)",
                'texture_size=128x128', "active_unique_assets=$count", 'model_placements=576',
                "RIBBON_GPU_PRESENT_MODE=$mode", 'RIBBON_GPU_FRAME_LATENCY=1', 'resolution=1280x800',
                "warmup_frames=$warmup", "requested_total_frames=$frames", "expected_frame_rows=$expectedRows",
                "command=$command", 'exit_code=pending', ''
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
            if ($code -ne 0) { throw "Capture process failed: $stem (exit $code)" }
            $frameRows = @(Import-Csv -LiteralPath $csv | Where-Object record_type -eq 'frame')
            if ($frameRows.Count -ne $expectedRows) { throw "Invalid frame count: $stem ($($frameRows.Count)/$expectedRows)" }
            for ($i = 1; $i -lt $frameRows.Count; $i++) {
                if ([long]$frameRows[$i].frame -ne ([long]$frameRows[$i - 1].frame + 1)) { throw "Noncontiguous frames: $stem" }
            }
            $last = $frameRows[-1]
            if ($last.surface_present_mode -ne $mode -or [int]$last.surface_frame_latency -ne 1) { throw "Mode/latency mismatch: $stem" }
            if ([int]$last.scene_model_placements -ne 576 -or [int]$last.gpu_model_cache_entries -ne $count) { throw "Workload/cache mismatch: $stem (placements=$($last.scene_model_placements), cache=$($last.gpu_model_cache_entries))" }
            if ([int]$last.gpu_timestamp_samples_skipped -ne 0) { throw "GPU samples skipped: $stem" }
            & python (Join-Path $repo 'tools/render_profile_report.py') $csv --output $report
            if ($LASTEXITCODE -ne 0) { throw "Report generation failed: $stem" }
            Add-Content -LiteralPath $manifest -Value "| $order | $count | $mode | ``$csv`` | ``$log`` | ``$report`` | $code | $($frameRows.Count) |"
            Write-Host "[$order/30] $stem rows=$($frameRows.Count) uploads=$($last.model_gpu_uploads_cumulative) texture_bytes=$($last.material_texture_bytes_estimate)"
        }
    }
}
finally {
    if ($hadMode) { $env:RIBBON_GPU_PRESENT_MODE = $previousMode } else { Remove-Item Env:RIBBON_GPU_PRESENT_MODE -ErrorAction SilentlyContinue }
    if ($hadLatency) { $env:RIBBON_GPU_FRAME_LATENCY = $previousLatency } else { Remove-Item Env:RIBBON_GPU_FRAME_LATENCY -ErrorAction SilentlyContinue }
}
Write-Host "Asset scalability capture artifacts: $out"
