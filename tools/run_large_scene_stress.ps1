[CmdletBinding()]
param(
    [ValidateRange(300, 10000)]
    [int]$Frames = 1800,
    [ValidateRange(0, 2000)]
    [int]$Warmup = 60,
    [object]$GridSides = @(16, 32, 48, 64),
    [ValidateRange(1, 128)]
    [int]$LifecycleSide = 24,
    [ValidateRange(1, 5000)]
    [int]$CycleFrames = 300,
    [ValidateSet('fifo', 'auto-vsync', 'auto-no-vsync', 'fifo-relaxed', 'immediate', 'mailbox')]
    [string]$PresentMode = 'mailbox',
    [ValidateRange(1, 3)]
    [int]$FrameLatency = 1,
    [ValidateRange(0, 128)]
    [int]$MultiAssetCount = 32,
    [ValidateRange(0, 128)]
    [int]$CachePrefetchCount = 32,
    [ValidateRange(8, 512)]
    [int]$StressTextureSize = 128,
    [ValidateRange(1, 4096)]
    [int]$AssetMemoryBudgetMiB = 1,
    [ValidateRange(1, 128)]
    [int]$MultiAssetGridSide = 32,
    [switch]$SkipLifecycle
)

$ErrorActionPreference = 'Stop'
function Invoke-CargoCapture {
    param([string[]]$Arguments, [string]$LogPath, [string]$Description)

    $previousPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        Set-Content -LiteralPath $LogPath -Value '' -Encoding UTF8
        & cargo @Arguments 2>&1 | ForEach-Object {
            $line = $_.ToString()
            Write-Host $line
            [System.IO.File]::AppendAllText(
                $LogPath,
                $line + [Environment]::NewLine,
                [System.Text.UTF8Encoding]::new($false)
            )
        }
        $cargoExitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousPreference
    }
    if ($cargoExitCode -ne 0) {
        throw "$Description failed (exit code $cargoExitCode). See log: $LogPath"
    }
}
if ($Warmup -ge $Frames) {
    throw 'Warmup must be smaller than the steady-scene frame count.'
}
# -File argument binding can pass a comma-separated CLI value as one string (for example,
# "16,32,48"). Normalize both that form and a PowerShell integer array into one validated array.
$normalizedGridSides = [System.Collections.Generic.List[int]]::new()
foreach ($item in @($GridSides)) {
    foreach ($part in ([string]$item -split '[,;\s]+')) {
        if ([string]::IsNullOrWhiteSpace($part)) {
            continue
        }
        $parsedSide = 0
        if (-not [int]::TryParse($part, [ref]$parsedSide)) {
            throw "GridSides must be comma-, semicolon-, or space-separated integers; got '$part'."
        }
        if ($parsedSide -lt 1 -or $parsedSide -gt 128) {
            throw "Every grid side must be between 1 and 128; got $parsedSide."
        }
        $normalizedGridSides.Add($parsedSide)
    }
}
if ($normalizedGridSides.Count -eq 0) {
    throw 'Provide at least one grid side (recommended: 16, 32, 48, 64).'
}
if (@($normalizedGridSides | Sort-Object -Unique).Count -ne $normalizedGridSides.Count) {
    throw 'Grid side values must be unique to avoid overwriting capture files.'
}
if ($MultiAssetCount -eq 0 -and $CachePrefetchCount -gt 0) {
    throw 'CachePrefetchCount must be zero when MultiAssetCount is zero.'
}
if (($MultiAssetCount + $CachePrefetchCount) -gt 256) {
    throw 'MultiAssetCount plus CachePrefetchCount must not exceed 256.'
}
if ($StressTextureSize -band ($StressTextureSize - 1)) {
    throw 'StressTextureSize must be a power of two between 8 and 512.'
}

$repoRoot = Split-Path -Parent $PSScriptRoot
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$captureDir = Join-Path $repoRoot (Join-Path 'target\profiles\large-scene' $stamp)
New-Item -ItemType Directory -Force -Path $captureDir | Out-Null
$manifestPath = Join-Path $captureDir 'large-scene-run-manifest.md'
$manifest = [System.Collections.Generic.List[string]]::new()
$manifest.Add('# Ribbon Engine Large-Scene Stress Runs')
$manifest.Add('')
$manifest.Add("- Generated locally: $(Get-Date -Format o)")
$manifest.Add("- Steady-scene frames: $Frames")
$manifest.Add("- Warmup frames: $Warmup")
$manifest.Add("- Grid sides: $($normalizedGridSides -join ', ')")
$manifest.Add("- Requested surface present mode: $PresentMode")
$manifest.Add("- Desired maximum frame latency: $FrameLatency")
$manifest.Add('- Build profile: Release')
$manifest.Add('- The script did not execute tests; it runs only the requested interactive profiling examples.')
$manifest.Add("- Multi-asset active models: $MultiAssetCount")
$manifest.Add("- Low-priority, unleased prefetch models: $CachePrefetchCount")
$manifest.Add("- Generated source texture size: $StressTextureSize x $StressTextureSize")
$manifest.Add("- Unpinned decoded CPU cache budget: $AssetMemoryBudgetMiB MiB")
$manifest.Add("- Multi-asset grid side: $MultiAssetGridSide")
$manifest.Add('')
$manifest.Add('| Scenario | CSV | Markdown report | Console log |')
$manifest.Add('|---|---|---|')

$previousPresentMode = $env:RIBBON_GPU_PRESENT_MODE
$previousLatency = $env:RIBBON_GPU_FRAME_LATENCY
$hadPreviousPresentMode = Test-Path Env:RIBBON_GPU_PRESENT_MODE
$hadPreviousLatency = Test-Path Env:RIBBON_GPU_FRAME_LATENCY

Push-Location $repoRoot
try {
    $env:RIBBON_GPU_PRESENT_MODE = $PresentMode
    $env:RIBBON_GPU_FRAME_LATENCY = [string]$FrameLatency

    if ($MultiAssetCount -gt 0) {
        $corpusCount = $MultiAssetCount + $CachePrefetchCount
        Write-Host "`n=== Generate multi-asset corpus: $corpusCount distinct GLB assets ===" -ForegroundColor Cyan
        & python tools/generate_asset_stress_corpus.py --count $corpusCount --texture-size $StressTextureSize --output-root target/asset-stress-corpus
        if ($LASTEXITCODE -ne 0) {
            throw "Stress corpus generation failed (exit code $LASTEXITCODE)."
        }
    }

    foreach ($side in $normalizedGridSides) {
        $label = 'grid-{0}x{0}' -f $side
        $csvPath = Join-Path $captureDir "$label.csv"
        $reportPath = Join-Path $captureDir "$label.md"
        $logPath = Join-Path $captureDir "$label.log"
        Write-Host "`n=== Large scene $side x $side ===" -ForegroundColor Cyan
        Write-Host "CSV: $csvPath"

        $cargoArgs = @('run', '--release', '-p', 'graphics_showcase', '--', '--stress-grid', [string]$side, '--frames', [string]$Frames, '--profile-warmup', [string]$Warmup, '--profile-csv', $csvPath)
        Invoke-CargoCapture -Arguments $cargoArgs -LogPath $logPath -Description "Large-scene capture for grid $side"
        & python tools/render_profile_report.py $csvPath --output $reportPath
        if ($LASTEXITCODE -ne 0) {
            throw "Profile report failed for $csvPath (exit code $LASTEXITCODE)."
        }
        $manifest.Add("| $label | ``${label}.csv`` | ``${label}.md`` | ``${label}.log`` |")
    }

    if ($MultiAssetCount -gt 0) {
        $label = 'multi-asset-{0}x{0}-active{1}-prefetch{2}' -f $MultiAssetGridSide, $MultiAssetCount, $CachePrefetchCount
        $csvPath = Join-Path $captureDir "$label.csv"
        $reportPath = Join-Path $captureDir "$label.md"
        $logPath = Join-Path $captureDir "$label.log"
        Write-Host "`n=== Multi-asset steady scene: grid $MultiAssetGridSide, active $MultiAssetCount, prefetch $CachePrefetchCount, budget $AssetMemoryBudgetMiB MiB ===" -ForegroundColor Cyan
        Write-Host "CSV: $csvPath"

        $cargoArgs = @('run', '--release', '-p', 'graphics_showcase', '--', '--stress-grid', [string]$MultiAssetGridSide, '--stress-multi-asset-count', [string]$MultiAssetCount, '--stress-cache-prefetch-count', [string]$CachePrefetchCount, '--asset-memory-budget-mib', [string]$AssetMemoryBudgetMiB, '--frames', [string]$Frames, '--profile-warmup', [string]$Warmup, '--profile-csv', $csvPath)
        Invoke-CargoCapture -Arguments $cargoArgs -LogPath $logPath -Description 'Multi-asset steady capture'
        & python tools/render_profile_report.py $csvPath --output $reportPath
        if ($LASTEXITCODE -ne 0) {
            throw "Multi-asset profile report failed for $csvPath (exit code $LASTEXITCODE)."
        }
        $manifest.Add("| $label | ``${label}.csv`` | ``${label}.md`` | ``${label}.log`` |")
    }

    if (-not $SkipLifecycle) {
        # End away from the N-frame cycle boundary so the final capture usually includes a
        # successful reload/active period as well as multiple inactive periods.
        $lifecycleFrames = $Frames + [Math]::Max(1, [int][Math]::Floor($CycleFrames / 2))
        if ($Warmup -ge $lifecycleFrames) {
            throw 'Warmup must be smaller than lifecycle capture frames.'
        }
        $csvPath = Join-Path $captureDir 'asset-lifecycle.csv'
        $reportPath = Join-Path $captureDir 'asset-lifecycle.md'
        $logPath = Join-Path $captureDir 'asset-lifecycle.log'
        Write-Host "`n=== Asset lifecycle cycle: grid $LifecycleSide, cycle $CycleFrames frames ===" -ForegroundColor Cyan
        Write-Host "CSV: $csvPath"

        $cargoArgs = @('run', '--release', '-p', 'graphics_showcase', '--', '--stress-grid', [string]$LifecycleSide, '--stress-cycle-frames', [string]$CycleFrames, '--frames', [string]$lifecycleFrames, '--profile-warmup', [string]$Warmup, '--profile-csv', $csvPath)
        Invoke-CargoCapture -Arguments $cargoArgs -LogPath $logPath -Description 'Asset lifecycle capture'
        & python tools/render_profile_report.py $csvPath --output $reportPath
        if ($LASTEXITCODE -ne 0) {
            throw "Asset lifecycle report failed for $csvPath (exit code $LASTEXITCODE)."
        }
        $manifest.Add("| asset lifecycle ($LifecycleSide x $LifecycleSide, cycle $CycleFrames) | ``asset-lifecycle.csv`` | ``asset-lifecycle.md`` | ``asset-lifecycle.log`` |")

        if ($MultiAssetCount -gt 0) {
            $multiLifecycleFrames = $Frames + [Math]::Max(1, [int][Math]::Floor($CycleFrames / 2))
            $label = 'multi-asset-lifecycle-{0}x{0}-active{1}-prefetch{2}' -f $MultiAssetGridSide, $MultiAssetCount, $CachePrefetchCount
            $csvPath = Join-Path $captureDir "$label.csv"
            $reportPath = Join-Path $captureDir "$label.md"
            $logPath = Join-Path $captureDir "$label.log"
            Write-Host "`n=== Multi-asset lifecycle: grid $MultiAssetGridSide, cycle $CycleFrames frames, budget $AssetMemoryBudgetMiB MiB ===" -ForegroundColor Cyan
            Write-Host "CSV: $csvPath"

            $cargoArgs = @('run', '--release', '-p', 'graphics_showcase', '--', '--stress-grid', [string]$MultiAssetGridSide, '--stress-multi-asset-count', [string]$MultiAssetCount, '--stress-cache-prefetch-count', [string]$CachePrefetchCount, '--asset-memory-budget-mib', [string]$AssetMemoryBudgetMiB, '--stress-cycle-frames', [string]$CycleFrames, '--frames', [string]$multiLifecycleFrames, '--profile-warmup', [string]$Warmup, '--profile-csv', $csvPath)
            Invoke-CargoCapture -Arguments $cargoArgs -LogPath $logPath -Description 'Multi-asset lifecycle capture'
            & python tools/render_profile_report.py $csvPath --output $reportPath
            if ($LASTEXITCODE -ne 0) {
                throw "Multi-asset lifecycle profile report failed for $csvPath (exit code $LASTEXITCODE)."
            }
            $manifest.Add("| $label | ``${label}.csv`` | ``${label}.md`` | ``${label}.log`` |")
        }
    }

    $manifest.Add('')
    $manifest.Add('## Reading the captures')
    $manifest.Add('')
    $manifest.Add('- For a steady grid, scene glTF placements should equal SIDE × SIDE while GPU model-cache entries stay bounded by the distinct asset generations.')
    $manifest.Add('- Instance-buffer capacity should grow with workload, then shrink after at least 120 low-demand frames once placements disappear.')
    $manifest.Add('- For lifecycle capture, correlate console generation messages with model upload/release counters and material texture bytes. Reload-frame spikes are expected; separate them from active steady-state periods.')
    $manifest.Add('- Multi-asset captures activate distinct textured GLBs while prefetch assets have low cache-retention priority and no strong lease. Budget enforcement should evict unpinned assets before active leased assets.')
    $manifest.Add('- The CPU asset budget is a soft cap for unpinned decoded assets; pinned bytes can exceed it by design. Estimates use loader-reported capacities and are not allocator/RSS telemetry.')
    $manifest.Add('- Generated assets are under target/asset-stress-corpus and are regenerated before the run. They are not checked-in assets.')
    $manifest.Add('- These runs are machine-, driver-, resolution-, and scene-specific. GPU resource bytes are estimates, not driver-reported VRAM.')
    $manifest.Add('')
    $manifest.Add('## Adapter and surface logs')
    $manifest.Add('')
    foreach ($log in Get-ChildItem -LiteralPath $captureDir -Filter '*.log' -File | Sort-Object Name) {
        $manifest.Add("### $($log.Name)")
        $manifest.Add('')
        $logLines = @(Get-Content -LiteralPath $log.FullName | Where-Object { $_ -match '^\[GPU ADAPTER\]|^\[GPU SURFACE\]' })
        if ($logLines.Count -eq 0) {
            $manifest.Add('- No GPU adapter/surface configuration lines were captured.')
        } else {
            foreach ($line in $logLines) {
                $manifest.Add("- ``$line``")
            }
        }
        $manifest.Add('')
    }
    Set-Content -LiteralPath $manifestPath -Value ($manifest.ToArray()) -Encoding utf8
    Write-Host "`nLarge-scene stress captures are complete." -ForegroundColor Green
    Write-Host "Run manifest: $manifestPath"
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
