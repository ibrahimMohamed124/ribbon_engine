[CmdletBinding()]
param(
    [ValidateRange(1, 10)]
    [int]$Runs = 3,
    [ValidateRange(300, 10000)]
    [int]$Frames = 1800,
    [ValidateRange(0, 2000)]
    [int]$Warmup = 120,
    [switch]$IncludeNoVSync
)

$ErrorActionPreference = 'Stop'
if ($Warmup -ge $Frames) {
    throw 'Warmup must be smaller than the total frame count.'
}
$repoRoot = Split-Path -Parent $PSScriptRoot
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$captureDir = Join-Path $repoRoot (Join-Path 'target\profiles\surface-ab' $stamp)
New-Item -ItemType Directory -Force -Path $captureDir | Out-Null

$previousPresentMode = $env:RIBBON_GPU_PRESENT_MODE
$previousLatency = $env:RIBBON_GPU_FRAME_LATENCY
$hadPreviousPresentMode = Test-Path Env:RIBBON_GPU_PRESENT_MODE
$hadPreviousLatency = Test-Path Env:RIBBON_GPU_FRAME_LATENCY

$configurations = @(
    [pscustomobject]@{ Name = 'fifo-latency1'; Mode = 'fifo'; Latency = '1' },
    [pscustomobject]@{ Name = 'fifo-latency2'; Mode = 'fifo'; Latency = '2' }
)
if ($IncludeNoVSync) {
    $configurations += [pscustomobject]@{ Name = 'auto-no-vsync-latency2'; Mode = 'auto-no-vsync'; Latency = '2' }
}

Push-Location $repoRoot
try {
    for ($run = 1; $run -le $Runs; $run++) {
        # Alternate order to reduce systematic bias from heat, clocks and background load.
        if (($run % 2) -eq 0) {
            $runConfigurations = @($configurations[($configurations.Count - 1)..0])
        } else {
            $runConfigurations = @($configurations)
        }

        foreach ($configuration in $runConfigurations) {
            $runText = '{0:D2}' -f $run
            $csvPath = Join-Path $captureDir "$($configuration.Name)-run$runText.csv"
            $reportPath = Join-Path $captureDir "$($configuration.Name)-run$runText.md"

            $env:RIBBON_GPU_PRESENT_MODE = $configuration.Mode
            $env:RIBBON_GPU_FRAME_LATENCY = $configuration.Latency
            Write-Host "`n=== $($configuration.Name), run $run/$Runs ===" -ForegroundColor Cyan
            Write-Host "CSV: $csvPath"

            & cargo run --release -p graphics_showcase -- --frames $Frames --profile-warmup $Warmup --profile-csv $csvPath
            if ($LASTEXITCODE -ne 0) {
                throw "Capture failed for $($configuration.Name), run $run (exit code $LASTEXITCODE)."
            }

            & python tools/render_profile_report.py $csvPath --output $reportPath
            if ($LASTEXITCODE -ne 0) {
                throw "Per-capture report failed for $csvPath (exit code $LASTEXITCODE)."
            }
        }
    }

    $summaryPath = Join-Path $captureDir 'surface-ab-summary.md'
    & python tools/compare_profile_runs.py --input-dir $captureDir --output $summaryPath
    if ($LASTEXITCODE -ne 0) {
        throw "Repeated-capture summary failed (exit code $LASTEXITCODE)."
    }

    Write-Host "`nSurface A/B captures are complete." -ForegroundColor Green
    Write-Host "Summary: $summaryPath"
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
