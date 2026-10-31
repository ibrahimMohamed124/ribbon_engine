[CmdletBinding()]
param(
    [ValidateRange(2, 256)] [int]$Regions = 48,
    [ValidateRange(1, 10)] [int]$Cycles = 2,
    [ValidateRange(2, 256)] [int]$Assets = 64,
    [ValidateRange(1, 16384)] [int]$AssetBytesKiB = 256,
    [ValidateRange(8, 512)] [int]$TextureSize = 512,
    [ValidateRange(1, 600)] [int]$FramesPerRegion = 24,
    [ValidateRange(48, 10000)] [int]$TailFrames = 120,
    [ValidateRange(1, 64)] [int]$UploadsPerFrame = 2,
    [ValidateRange(1, 4096)] [int]$UploadBudgetMiB = 2,
    [ValidateRange(1, 120)] [int]$GpuTimeoutMinutes = 10,
    [ValidateRange(10, 1000)] [int]$MemorySampleIntervalMs = 25,
    [string]$OutputRoot = "target/streaming-stress-runs",
    [switch]$HeadlessOnly,
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$ProjectRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
Set-Location $ProjectRoot

function ConvertTo-WindowsArgument {
    param([Parameter(Mandatory = $true)][string]$Value)

    if ($Value.Length -gt 0 -and $Value -notmatch '[\s"]') {
        return $Value
    }

    $builder = New-Object System.Text.StringBuilder
    [void]$builder.Append('"')
    $backslashes = 0
    foreach ($character in $Value.ToCharArray()) {
        if ($character -eq '\') {
            $backslashes++
            continue
        }
        if ($character -eq '"') {
            if ($backslashes -gt 0) {
                [void]$builder.Append(('\' * (2 * $backslashes)))
            }
            [void]$builder.Append('\"')
            $backslashes = 0
            continue
        }
        if ($backslashes -gt 0) {
            [void]$builder.Append(('\' * $backslashes))
            $backslashes = 0
        }
        [void]$builder.Append($character)
    }
    if ($backslashes -gt 0) {
        [void]$builder.Append(('\' * (2 * $backslashes)))
    }
    [void]$builder.Append('"')
    return $builder.ToString()
}

function Invoke-TrackedProcess {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$FilePath,
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [Parameter(Mandatory = $true)][string]$WorkingDirectory,
        [Parameter(Mandatory = $true)][string]$OutputDirectory,
        [Parameter(Mandatory = $true)][int]$TimeoutSeconds,
        [Parameter(Mandatory = $true)][int]$SampleIntervalMs
    )

    New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null
    $stdoutPath = Join-Path $OutputDirectory "$Name.stdout.log"
    $stderrPath = Join-Path $OutputDirectory "$Name.stderr.log"
    $memoryPath = Join-Path $OutputDirectory "process-memory.csv"
    $argumentLine = ($Arguments | ForEach-Object { ConvertTo-WindowsArgument ([string]$_) }) -join ' '

    Write-Host "`n[$Name] $FilePath $argumentLine" -ForegroundColor Cyan

    # Use System.Diagnostics.Process directly. Start-Process can return a process wrapper
    # whose ExitCode is empty on some PowerShell/rustup combinations, even after waiting.
    # Redirected async readers prevent stdout/stderr pipe saturation from blocking Cargo.
    $startInfo = New-Object System.Diagnostics.ProcessStartInfo
    $startInfo.FileName = $FilePath
    $startInfo.Arguments = $argumentLine
    $startInfo.WorkingDirectory = $WorkingDirectory
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true

    $process = New-Object System.Diagnostics.Process
    $process.StartInfo = $startInfo
    if (-not $process.Start()) {
        throw "Unable to start process '$FilePath'."
    }

    $stdoutTask = $process.StandardOutput.ReadToEndAsync()
    $stderrTask = $process.StandardError.ReadToEndAsync()
    $samples = New-Object 'System.Collections.Generic.List[object]'
    $watch = [System.Diagnostics.Stopwatch]::StartNew()
    $timedOut = $false
    $exitCode = -1
    $processError = $null

    # Capture an immediate sample so short-lived phases do not incorrectly report 0 MiB.
    try {
        $process.Refresh()
        $samples.Add([pscustomobject]@{
            elapsed_ms = 0.0
            working_set_bytes = [int64]$process.WorkingSet64
            private_bytes = [int64]$process.PrivateMemorySize64
        })
    }
    catch {
        # A very short-lived process may exit before its first memory counters are readable.
    }

    try {
        while (-not $process.WaitForExit($SampleIntervalMs)) {
            try {
                $process.Refresh()
                if (-not $process.HasExited) {
                    $samples.Add([pscustomobject]@{
                        elapsed_ms = [math]::Round($watch.Elapsed.TotalMilliseconds, 1)
                        working_set_bytes = [int64]$process.WorkingSet64
                        private_bytes = [int64]$process.PrivateMemorySize64
                    })
                }
            }
            catch {
                # The process can exit between WaitForExit, Refresh and reading its counters.
            }

            if ($watch.Elapsed.TotalSeconds -ge $TimeoutSeconds) {
                $timedOut = $true
                Write-Host "[$Name] timeout after $TimeoutSeconds seconds; terminating process." -ForegroundColor Red
                try { $process.Kill() } catch { }
                break
            }
        }

        # WaitForExit without a timeout also flushes process exit notifications.
        $process.WaitForExit()
        $stdout = $stdoutTask.GetAwaiter().GetResult()
        $stderr = $stderrTask.GetAwaiter().GetResult()

        [System.IO.File]::WriteAllText($stdoutPath, $stdout, [System.Text.UTF8Encoding]::new($false))
        [System.IO.File]::WriteAllText($stderrPath, $stderr, [System.Text.UTF8Encoding]::new($false))

        if ($timedOut) {
            $exitCode = 124
        }
        else {
            $rawExitCode = $process.ExitCode
            if ($null -eq $rawExitCode) {
                throw "Process '$Name' exited, but Windows did not provide an exit code."
            }
            $exitCode = [int]$rawExitCode
        }
    }
    catch {
        $processError = $_.Exception.Message
        if (-not $process.HasExited) {
            try { $process.Kill() } catch { }
            try { $process.WaitForExit() } catch { }
        }
        if ($null -eq $stdout) { $stdout = '' }
        if ($null -eq $stderr) { $stderr = '' }
        $stderr = ($stderr.TrimEnd() + "`r`nRUNNER ERROR: $processError`r`n")
        [System.IO.File]::WriteAllText($stdoutPath, $stdout, [System.Text.UTF8Encoding]::new($false))
        [System.IO.File]::WriteAllText($stderrPath, $stderr, [System.Text.UTF8Encoding]::new($false))
        if (-not $timedOut) { $exitCode = -1 }
    }
    finally {
        $watch.Stop()
        try { $process.Dispose() } catch { }
    }

    if ($samples.Count -eq 0) {
        $samples.Add([pscustomobject]@{
            elapsed_ms = [math]::Round($watch.Elapsed.TotalMilliseconds, 1)
            working_set_bytes = 0L
            private_bytes = 0L
        })
    }
    $samples | Export-Csv -Path $memoryPath -NoTypeInformation -Encoding UTF8

    if (Test-Path $stdoutPath) {
        Get-Content $stdoutPath | ForEach-Object { Write-Host $_ }
    }
    if (Test-Path $stderrPath) {
        Get-Content $stderrPath | ForEach-Object { Write-Host $_ -ForegroundColor DarkYellow }
    }

    $peakWorkingSet = [int64](($samples | Measure-Object -Property working_set_bytes -Maximum).Maximum)
    $peakPrivate = [int64](($samples | Measure-Object -Property private_bytes -Maximum).Maximum)
    $result = [pscustomobject]@{
        Name = $Name
        ExitCode = $exitCode
        Status = $(if ($exitCode -eq 0) { "PASS" } else { "FAIL" })
        DurationSeconds = [math]::Round($watch.Elapsed.TotalSeconds, 2)
        PeakWorkingSetBytes = $peakWorkingSet
        PeakPrivateBytes = $peakPrivate
        MemorySamples = $samples.Count
        Stdout = $stdoutPath
        Stderr = $stderrPath
        MemoryCsv = $memoryPath
        RunnerError = $processError
    }
    Write-Host "[$Name] $($result.Status), exit=$exitCode, duration=$($result.DurationSeconds)s, peak working set=$([math]::Round($peakWorkingSet / 1MB, 1)) MiB, peak private=$([math]::Round($peakPrivate / 1MB, 1)) MiB" -ForegroundColor $(if ($exitCode -eq 0) { "Green" } else { "Red" })
    return $result
}

$runId = Get-Date -Format "yyyyMMdd-HHmmss"
$fullOutputRoot = if ([System.IO.Path]::IsPathRooted($OutputRoot)) {
    [System.IO.Path]::GetFullPath($OutputRoot)
} else {
    [System.IO.Path]::GetFullPath((Join-Path $ProjectRoot $OutputRoot))
}
$runRoot = Join-Path $fullOutputRoot $runId
$corpusRoot = [System.IO.Path]::GetFullPath((Join-Path $ProjectRoot "target/streaming-stress-corpus"))
New-Item -ItemType Directory -Force -Path $runRoot | Out-Null

Write-Host "===============================================================" -ForegroundColor Cyan
Write-Host " RIBBON ENGINE - STREAMING STRESS HARNESS" -ForegroundColor Cyan
Write-Host "===============================================================" -ForegroundColor Cyan
Write-Host "Project: $ProjectRoot"
Write-Host "Output:  $runRoot"
Write-Host "Regions=$Regions Cycles=$Cycles Assets=$Assets Texture=${TextureSize}px"

$results = New-Object 'System.Collections.Generic.List[object]'
$headlessOutput = Join-Path $runRoot "headless"
$commonArguments = @(
    "--regions", "$Regions",
    "--cycles", "$Cycles",
    "--assets", "$Assets",
    "--asset-bytes-kib", "$AssetBytesKiB",
    "--frames-per-region", "$FramesPerRegion",
    "--tail-frames", "$TailFrames",
    "--texture-size", "$TextureSize",
    "--uploads-per-frame", "$UploadsPerFrame",
    "--upload-budget-mib", "$UploadBudgetMiB"
)

if (-not $SkipBuild) {
    $cargoCommand = Get-Command cargo -ErrorAction Stop
    $buildOutput = Join-Path $runRoot "build"
    Write-Host "`n[build] cargo build --release -p ribbon_streaming_stress" -ForegroundColor Yellow
    $buildResult = Invoke-TrackedProcess `
        -Name "build" `
        -FilePath $cargoCommand.Source `
        -Arguments @("build", "--release", "-p", "ribbon_streaming_stress") `
        -WorkingDirectory $ProjectRoot `
        -OutputDirectory $buildOutput `
        -TimeoutSeconds 1200 `
        -SampleIntervalMs $MemorySampleIntervalMs
    $results.Add($buildResult)
    if ($buildResult.ExitCode -ne 0) {
        throw "Cargo release build failed with exit code $($buildResult.ExitCode); inspect $($buildResult.Stderr)"
    }
}

$executable = Join-Path $ProjectRoot "target\release\ribbon_streaming_stress.exe"
if (-not (Test-Path $executable)) {
    throw "Stress harness executable not found: $executable. Run without -SkipBuild first."
}

$headlessArgs = @("--mode", "headless") + $commonArguments + @("--output-dir", $headlessOutput)
$headlessResult = Invoke-TrackedProcess `
    -Name "headless" `
    -FilePath $executable `
    -Arguments $headlessArgs `
    -WorkingDirectory $ProjectRoot `
    -OutputDirectory $headlessOutput `
    -TimeoutSeconds 600 `
    -SampleIntervalMs $MemorySampleIntervalMs
$results.Add($headlessResult)

if (-not $HeadlessOnly) {
    $generator = Join-Path $ProjectRoot "tools\assets\generate_asset_stress_corpus.py"
    if (-not (Test-Path $generator)) {
        throw "GLB stress-corpus generator not found: $generator"
    }
    $pythonCommand = Get-Command py -ErrorAction Stop
    $generatorOutput = Join-Path $runRoot "corpus-generator"
    Write-Host "`n[corpus] generating $Assets GLB models with ${TextureSize}px textures" -ForegroundColor Yellow
    $generatorResult = Invoke-TrackedProcess `
        -Name "corpus-generator" `
        -FilePath $pythonCommand.Source `
        -Arguments @("-3", $generator, "--count", "$Assets", "--texture-size", "$TextureSize", "--output-root", $corpusRoot) `
        -WorkingDirectory $ProjectRoot `
        -OutputDirectory $generatorOutput `
        -TimeoutSeconds 600 `
        -SampleIntervalMs $MemorySampleIntervalMs
    $results.Add($generatorResult)
    if ($generatorResult.ExitCode -ne 0) {
        throw "GLB stress-corpus generation failed with exit code $($generatorResult.ExitCode); inspect $($generatorResult.Stderr)"
    }

    $gpuOutput = Join-Path $runRoot "gpu"
    $gpuArgs = @("--mode", "gpu") + $commonArguments + @(
        "--asset-root", $corpusRoot,
        "--output-dir", $gpuOutput,
        "--require-gpu"
    )
    Write-Host "`n[GPU] A real native render window will open. Keep it open until the harness exits." -ForegroundColor Yellow
    $gpuResult = Invoke-TrackedProcess `
        -Name "gpu" `
        -FilePath $executable `
        -Arguments $gpuArgs `
        -WorkingDirectory $ProjectRoot `
        -OutputDirectory $gpuOutput `
        -TimeoutSeconds ($GpuTimeoutMinutes * 60) `
        -SampleIntervalMs $MemorySampleIntervalMs
    $results.Add($gpuResult)
}

$summaryPath = Join-Path $runRoot "run-summary.md"
$lines = New-Object 'System.Collections.Generic.List[string]'
$lines.Add("# Ribbon Engine Streaming Stress Run")
$lines.Add("")
$lines.Add("- Run ID: $runId")
$lines.Add("- Project: $ProjectRoot")
$lines.Add("- Regions / cycles: $Regions / $Cycles")
$lines.Add("- Generated GLB assets: $Assets")
$lines.Add("- Upload budget: $UploadsPerFrame operations / $UploadBudgetMiB MiB per rendered frame")
$lines.Add("- Process memory sampling interval: $MemorySampleIntervalMs ms")
$lines.Add("")
$lines.Add("| Phase | Result | Exit code | Duration (s) | Peak working set (MiB) | Peak private bytes (MiB) |")
$lines.Add("|---|---:|---:|---:|---:|---:|")
foreach ($item in $results) {
    $workingMiB = [math]::Round($item.PeakWorkingSetBytes / 1MB, 2)
    $privateMiB = [math]::Round($item.PeakPrivateBytes / 1MB, 2)
    $lines.Add("| $($item.Name) | $($item.Status) | $($item.ExitCode) | $($item.DurationSeconds) | $workingMiB | $privateMiB |")
}
$lines.Add("")
$lines.Add("## Measurement semantics")
$lines.Add("")
$lines.Add('- `process-memory.csv` samples this harness process working set and private bytes every ' + $MemorySampleIntervalMs + ' ms; it is OS process memory, not a per-resource allocation ledger.')
$lines.Add("- The headless/GPU reports record decoded AssetServer CPU residency estimates, pinned/unpinned bytes, active worker counts, completion backlog, and source-read/decode timing reported by the AssetServer.")
$lines.Add("- The GPU report records renderer allocation estimates and actual upload diagnostics; GPU bytes are estimates, not driver-reported physical VRAM.")
$lines.Add("- GPU timestamp values are asynchronous and only available when the adapter supports timestamp queries.")
$lines.Add("- The headless phase deliberately leaves GPU measurements blank; it does not simulate successful GPU uploads.")
$lines.Add("")
$lines.Add("## Artifacts")
$lines.Add("")
$lines.Add('- `headless/headless-summary.md`, `headless/headless-frames.csv`, `headless/process-memory.csv`')
if (-not $HeadlessOnly) {
    $lines.Add('- `gpu/gpu-summary.md`, `gpu/gpu-frames.csv`, `gpu/render-profile.csv`, `gpu/process-memory.csv')
}
$lines.Add("")
$lines.Add('A phase passes only when its process exits with code 0. GPU phase requires a real rendered frame because this script always passes `--require-gpu`.')
[System.IO.File]::WriteAllLines($summaryPath, $lines, [System.Text.UTF8Encoding]::new($false))

$results | Format-Table Name, Status, ExitCode, DurationSeconds, MemorySamples -AutoSize
Write-Host "`nSummary: $summaryPath" -ForegroundColor Cyan
Write-Host "All artifacts: $runRoot" -ForegroundColor Cyan

$failed = @($results | Where-Object { $_.ExitCode -ne 0 })
if ($failed.Count -gt 0) {
    exit 1
}
Write-Host "STREAMING STRESS: ALL REQUESTED PHASES PASSED" -ForegroundColor Green
exit 0
