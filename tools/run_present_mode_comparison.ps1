[CmdletBinding()]
param(
    [ValidateRange(1, 10)]
    [int]$Runs = 3,
    [ValidateRange(300, 10000)]
    [int]$Frames = 1050,
    [ValidateRange(0, 2000)]
    [int]$Warmup = 60,
    [ValidateRange(1, 128)]
    [int]$GridSide = 32,
    [ValidateRange(0, 128)]
    [int]$MultiAssetCount = 32,
    [ValidateRange(0, 128)]
    [int]$CachePrefetchCount = 32,
    [ValidateRange(1, 4096)]
    [int]$AssetMemoryBudgetMiB = 1,
    [ValidateRange(30, 3000)]
    [int]$CycleFrames = 300,
    [ValidateRange(8, 512)]
    [int]$StressTextureSize = 128,
    [ValidateRange(1, 3)]
    [int]$FrameLatency = 1,
    [ValidateRange(1.0, 1000.0)]
    [double]$FrameBudgetMs = 16.667
)

$ErrorActionPreference = 'Stop'
if ($Warmup -ge $Frames) {
    throw 'Warmup must be smaller than the total frame count.'
}
$expectedFrameRows = $Frames - $Warmup + 1
if (($MultiAssetCount + $CachePrefetchCount) -gt 256) {
    throw 'MultiAssetCount plus CachePrefetchCount must not exceed 256.'
}
if ($StressTextureSize -band ($StressTextureSize - 1)) {
    throw 'StressTextureSize must be a power of two between 8 and 512.'
}
if ($CachePrefetchCount -gt 0 -and $MultiAssetCount -eq 0) {
    throw 'CachePrefetchCount must be zero when MultiAssetCount is zero.'
}
$repoRoot = Split-Path -Parent $PSScriptRoot
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$captureDir = Join-Path $repoRoot (Join-Path 'target\profiles\present-mode-comparison' $stamp)
$suffix = 1
while (Test-Path -LiteralPath $captureDir) {
    $captureDir = Join-Path $repoRoot (Join-Path 'target\profiles\present-mode-comparison' ("{0}-{1:D2}" -f $stamp, $suffix))
    $suffix++
}
New-Item -ItemType Directory -Force -Path $captureDir | Out-Null
$manifestPath = Join-Path $captureDir 'present-mode-comparison-manifest.md'
$manifest = [System.Collections.Generic.List[string]]::new()
$manifest.Add('# Ribbon Engine FIFO vs Mailbox Comparison')
$manifest.Add('')
$manifest.Add("- Generated locally: $(Get-Date -Format o)")
$manifest.Add('- Comparison design: repeated, alternating-order, same-workload capture; no tests are executed.')
$manifest.Add("- Repeats per requested mode: $Runs")
$manifest.Add("- Total frames per capture: $Frames")
$manifest.Add("- Warmup frames excluded: $Warmup")
$manifest.Add("- Grid side: $GridSide; active unique models: $MultiAssetCount; unleased prefetch models: $CachePrefetchCount")
$manifest.Add("- Unpinned decoded CPU cache budget: $AssetMemoryBudgetMiB MiB")
$manifest.Add("- Lifecycle period: $CycleFrames frames")
$manifest.Add("- Generated texture size: $StressTextureSize x $StressTextureSize")
$manifest.Add("- Desired maximum frame latency for both modes: $FrameLatency")
$manifest.Add("- Frame budget reference: $FrameBudgetMs ms")
$manifest.Add('- Keep window size, display settings, power state and background workload unchanged throughout the capture set.')
$manifest.Add('- Each capture saves a console log including the selected GPU adapter and resolved surface settings.')
$manifest.Add('- Mailbox is compared only if every mailbox capture resolves to mailbox. Unsupported/fallback captures invalidate the direct A/B comparison.')
$manifest.Add('')
$manifest.Add('| Capture | Requested mode | Resolved mode | Latency | CSV | Per-capture report | Console log |')
$manifest.Add('|---|---|---|---:|---|---|---|')

$previousPresentMode = $env:RIBBON_GPU_PRESENT_MODE
$previousLatency = $env:RIBBON_GPU_FRAME_LATENCY
$hadPreviousPresentMode = Test-Path Env:RIBBON_GPU_PRESENT_MODE
$hadPreviousLatency = Test-Path Env:RIBBON_GPU_FRAME_LATENCY
$configurations = @(
    [pscustomobject]@{ Name = 'fifo'; Mode = 'fifo' },
    [pscustomobject]@{ Name = 'mailbox'; Mode = 'mailbox' }
)

Push-Location $repoRoot
try {
    $corpusCount = $MultiAssetCount + $CachePrefetchCount
    if ($corpusCount -gt 0) {
        & python tools/generate_asset_stress_corpus.py --count $corpusCount --texture-size $StressTextureSize --output-root target/asset-stress-corpus
        if ($LASTEXITCODE -ne 0) {
            throw "Stress corpus generation failed (exit code $LASTEXITCODE)."
        }
    }

    for ($run = 1; $run -le $Runs; $run++) {
        if (($run % 2) -eq 0) {
            $runConfigurations = @($configurations[1], $configurations[0])
        } else {
            $runConfigurations = @($configurations)
        }

        foreach ($configuration in $runConfigurations) {
            $runText = '{0:D2}' -f $run
            $csvName = "$($configuration.Name)-run$runText.csv"
            $csvPath = Join-Path $captureDir $csvName
            $reportName = "$($configuration.Name)-run$runText.md"
            $reportPath = Join-Path $captureDir $reportName
            $logName = "$($configuration.Name)-run$runText.log"
            $logPath = Join-Path $captureDir $logName

            $env:RIBBON_GPU_PRESENT_MODE = $configuration.Mode
            $env:RIBBON_GPU_FRAME_LATENCY = [string]$FrameLatency

            Write-Host "Running $($configuration.Name) capture $run/$Runs. CSV: $csvPath"
            $cargoArgs = @(
                'run', '--release', '-p', 'graphics_showcase', '--',
                '--stress-grid', [string]$GridSide,
                '--stress-cycle-frames', [string]$CycleFrames,
                '--frames', [string]$Frames,
                '--profile-warmup', [string]$Warmup,
                '--profile-csv', $csvPath
            )
            if ($MultiAssetCount -gt 0) {
                $cargoArgs += @(
                    '--stress-multi-asset-count', [string]$MultiAssetCount,
                    '--stress-cache-prefetch-count', [string]$CachePrefetchCount,
                    '--asset-memory-budget-mib', [string]$AssetMemoryBudgetMiB
                )
            }
            # Cargo writes ordinary progress to stderr. Convert each merged pipeline
            # item to plain text so PowerShell does not format it as NativeCommandError.
            # Keep the preference local and read LASTEXITCODE immediately after Cargo.
            $previousErrorActionPreference = $ErrorActionPreference
            try {
                $ErrorActionPreference = 'Continue'
                Set-Content -LiteralPath $logPath -Value '' -Encoding UTF8
                & cargo @cargoArgs 2>&1 | ForEach-Object {
                    $line = $_.ToString()
                    Write-Host $line
                    [System.IO.File]::AppendAllText(
                        $logPath,
                        $line + [Environment]::NewLine,
                        [System.Text.UTF8Encoding]::new($false)
                    )
                }
                $cargoExitCode = $LASTEXITCODE
            }
            finally {
                $ErrorActionPreference = $previousErrorActionPreference
            }
            if ($cargoExitCode -ne 0) {
                throw "Cargo failed with exit code $cargoExitCode for requested mode $($configuration.Mode), run $run. See log: $logPath"
            }

            & python tools/render_profile_report.py $csvPath --output $reportPath
            if ($LASTEXITCODE -ne 0) {
                throw "Per-capture report failed for $csvPath (exit code $LASTEXITCODE)."
            }
            if (-not (Test-Path -LiteralPath $reportPath -PathType Leaf) -or (Get-Item -LiteralPath $reportPath).Length -eq 0) {
                throw "Per-capture report is missing or empty: $reportPath"
            }

            $profileRows = @(Import-Csv -LiteralPath $csvPath | Where-Object { $_.record_type -eq 'frame' })
            if ($profileRows.Count -ne $expectedFrameRows) {
                throw "Capture has $($profileRows.Count) frame rows; expected $expectedFrameRows after warmup: $csvPath"
            }
            $resolvedModes = @($profileRows | Select-Object -ExpandProperty surface_present_mode -Unique | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
            $resolvedMode = if ($resolvedModes.Count -eq 1) { [string]$resolvedModes[0] } elseif ($resolvedModes.Count -eq 0) { 'not recorded' } else { ($resolvedModes -join ',') }
            $resolvedLatencies = @($profileRows | Select-Object -ExpandProperty surface_frame_latency -Unique | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
            $resolvedLatency = if ($resolvedLatencies.Count -eq 1) { [string]$resolvedLatencies[0] } elseif ($resolvedLatencies.Count -eq 0) { 'not recorded' } else { ($resolvedLatencies -join ',') }

            $manifest.Add("| $($configuration.Name)-run$runText | $($configuration.Mode) | $resolvedMode | $resolvedLatency | ``$csvName`` | ``$reportName`` | ``$logName`` |")
            if ($resolvedMode -ne $configuration.Mode) {
                $manifest.Add('')
                $manifest.Add("<!-- WARNING: requested $($configuration.Mode) but capture resolved to $resolvedMode for $csvName -->")
                Write-Warning "Requested $($configuration.Mode), but capture resolved to '$resolvedMode'. The comparison is not a valid direct A/B for this mode."
            }
            if ($resolvedLatencies.Count -ne 1 -or $resolvedLatency -ne [string]$FrameLatency) {
                $manifest.Add('')
                $manifest.Add("<!-- WARNING: requested latency $FrameLatency but capture recorded '$resolvedLatency' for $csvName -->")
                Write-Warning "Capture $csvName did not consistently record the desired frame-latency value $FrameLatency."
            }
        }
    }

    $summaryPath = Join-Path $captureDir 'present-mode-comparison-summary.md'
    & python tools/compare_profile_runs.py --input-dir $captureDir --output $summaryPath --budget-ms $FrameBudgetMs --expected-frames $expectedFrameRows
    if ($LASTEXITCODE -ne 0) {
        throw "Comparison report generation failed (exit code $LASTEXITCODE)."
    }
    if (-not (Test-Path -LiteralPath $summaryPath -PathType Leaf) -or (Get-Item -LiteralPath $summaryPath).Length -eq 0) {
        throw "Comparison report is missing or empty: $summaryPath"
    }

    $manifest.Add('')
    $manifest.Add('## Adapter and surface logs')
    $manifest.Add('')
    $manifest.Add('Adapter details are the exact values reported by the running `wgpu` process. The Windows device list alone does not identify the adapter selected for this surface.')
    $manifest.Add('')
    foreach ($log in Get-ChildItem -LiteralPath $captureDir -Filter '*.log' -File | Sort-Object Name) {
        $manifest.Add("### $($log.Name)")
        $manifest.Add('')
        $logLines = @(Get-Content -LiteralPath $log.FullName | Where-Object { $_ -match '^\[GPU ADAPTER\]|^\[GPU SURFACE\]' })
        if ($logLines.Count -eq 0) {
            $manifest.Add('- No `[GPU ADAPTER]` line was captured. Confirm the engine build contains adapter logging.')
        } else {
            foreach ($line in $logLines) {
                $manifest.Add("- ``$line``")
            }
        }
        $manifest.Add('')
    }
    $manifest.Add('## Output files')
    $manifest.Add('')
    $manifest.Add("- Comparison summary: ``$([System.IO.Path]::GetFileName($summaryPath))``")
    $manifest.Add('- Each capture has a CSV, Markdown render-profile report and console log.')
    $manifest.Add('- Summary deltas use run-level percentiles across repeats; frame rows are not pooled across separate runs.')
    $manifest.Add('- Inspect resolved present modes before interpreting deltas. A requested Mailbox capture resolving to FIFO is not evidence about Mailbox behavior.')
    Set-Content -LiteralPath $manifestPath -Value ($manifest.ToArray()) -Encoding UTF8

    Write-Host "Present-mode comparison captures complete. Summary: $summaryPath"
    Write-Host "Manifest: $manifestPath"
}
finally {
    if ($hadPreviousPresentMode) {
        $env:RIBBON_GPU_PRESENT_MODE = $previousPresentMode
    } else {
        Remove-Item Env:RIBBON_GPU_PRESENT_MODE -ErrorAction SilentlyContinue
    }
    if ($hadPreviousLatency) {
        $env:RIBBON_GPU_FRAME_LATENCY = $previousLatency
    } else {
        Remove-Item Env:RIBBON_GPU_FRAME_LATENCY -ErrorAction SilentlyContinue
    }
    Pop-Location
}
