[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string] $CsvPath,

    [string] $OutputPath,

    [ValidateRange(1, 100)]
    [int] $SlowFrameCount = 10,

    [ValidateRange(1, 100)]
    [int] $MaxUploadsPerFrame = 2
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$culture = [System.Globalization.CultureInfo]::InvariantCulture

function Get-OptionalNumber {
    param(
        [Parameter(Mandatory = $true)] [object] $Row,
        [Parameter(Mandatory = $true)] [string] $Column
    )

    $property = $Row.PSObject.Properties[$Column]
    if ($null -eq $property) { return $null }
    $raw = [string]$property.Value
    if ([string]::IsNullOrWhiteSpace($raw)) { return $null }

    $number = 0.0
    $styles = [System.Globalization.NumberStyles]::Float -bor [System.Globalization.NumberStyles]::AllowThousands
    if (-not [double]::TryParse($raw, $styles, $script:culture, [ref]$number)) {
        throw "Could not parse numeric value in CSV column '$Column': '$raw'."
    }
    return $number
}

function Format-Number {
    param([AllowNull()][object] $Value, [int] $Digits = 3)
    if ($null -eq $Value) {
        return 'n/a'
    }
    $Value = [double]$Value
    if ([double]::IsNaN($Value) -or [double]::IsInfinity($Value)) {
        return 'n/a'
    }
    return $Value.ToString(('0.' + ('0' * $Digits)), $script:culture)
}

function Format-Bytes {
    param([AllowNull()][object] $Bytes)
    if ($null -eq $Bytes) { return 'n/a' }
    $Bytes = [double]$Bytes
    if ($Bytes -ge 1MB) { return "$(Format-Number ($Bytes / 1MB) 2) MiB ($([long]$Bytes) B)" }
    if ($Bytes -ge 1KB) { return "$(Format-Number ($Bytes / 1KB) 2) KiB ($([long]$Bytes) B)" }
    return "$([long]$Bytes) B"
}

function Get-Stats {
    param(
        [Parameter(Mandatory = $true)] [object[]] $Items,
        [Parameter(Mandatory = $true)] [string] $Property
    )

    $values = @(
        foreach ($item in $Items) {
            $value = $item.PSObject.Properties[$Property].Value
            if ($null -ne $value) { [double]$value }
        }
    )
    if ($values.Count -eq 0) {
        return [pscustomobject]@{ Count = 0; Mean = $null; P50 = $null; P95 = $null; P99 = $null; Peak = $null }
    }

    $sorted = @($values | Sort-Object)
    $mean = ($values | Measure-Object -Average).Average
    $p50Index = [Math]::Max(0, [Math]::Ceiling(0.50 * $sorted.Count) - 1)
    $p95Index = [Math]::Max(0, [Math]::Ceiling(0.95 * $sorted.Count) - 1)
    $p99Index = [Math]::Max(0, [Math]::Ceiling(0.99 * $sorted.Count) - 1)
    return [pscustomobject]@{
        Count = $values.Count
        Mean = [double]$mean
        P50 = [double]$sorted[$p50Index]
        P95 = [double]$sorted[$p95Index]
        P99 = [double]$sorted[$p99Index]
        Peak = [double]$sorted[-1]
    }
}

$resolvedCsv = (Resolve-Path -LiteralPath $CsvPath).Path
$allRows = @(Import-Csv -LiteralPath $resolvedCsv)
if ($allRows.Count -eq 0) { throw "CSV contains no rows: $resolvedCsv" }

$columns = @($allRows[0].PSObject.Properties.Name)
$requiredColumns = @(
    'frame', 'frame_cpu_ns', 'graph_cpu_ns', 'surface_acquire_ns', 'present_ns',
    'model_gpu_uploads_cumulative', 'model_gpu_releases_cumulative'
)
$missing = @($requiredColumns | Where-Object { $columns -notcontains $_ })
if ($missing.Count -gt 0) {
    throw "CSV is missing required column(s): $($missing -join ', ')"
}

if ($columns -contains 'record_type') {
    $rawFrameRows = @($allRows | Where-Object { $_.record_type -eq 'frame' })
    $rawPassRows = @($allRows | Where-Object { $_.record_type -eq 'pass' })
}
else {
    $rawFrameRows = @($allRows | Where-Object { -not [string]::IsNullOrWhiteSpace([string]$_.frame_cpu_ns) })
    $rawPassRows = @()
}
if ($rawFrameRows.Count -eq 0) { throw "CSV contains no frame samples: $resolvedCsv" }

$sampleList = New-Object 'System.Collections.Generic.List[object]'
foreach ($row in $rawFrameRows) {
    $frameValue = Get-OptionalNumber -Row $row -Column 'frame'
    if ($null -eq $frameValue) { continue }

    $uploadsValue = Get-OptionalNumber -Row $row -Column 'model_gpu_uploads_cumulative'
    $releasesValue = Get-OptionalNumber -Row $row -Column 'model_gpu_releases_cumulative'
    $frameWallNs = Get-OptionalNumber -Row $row -Column 'frame_cpu_ns'
    $graphNs = Get-OptionalNumber -Row $row -Column 'graph_cpu_ns'
    $acquireNs = Get-OptionalNumber -Row $row -Column 'surface_acquire_ns'
    $presentNs = Get-OptionalNumber -Row $row -Column 'present_ns'
    $gpuShadowMeshNs = Get-OptionalNumber -Row $row -Column 'gpu_shadow_and_mesh_ns'
    $gpuMeshNs = Get-OptionalNumber -Row $row -Column 'gpu_mesh_pass_ns'

    $sampleList.Add([pscustomobject]@{
        Frame = [long]$frameValue
        FrameWallMs = if ($null -ne $frameWallNs) { $frameWallNs / 1000000.0 } else { $null }
        GraphCpuMs = if ($null -ne $graphNs) { $graphNs / 1000000.0 } else { $null }
        SurfaceAcquireMs = if ($null -ne $acquireNs) { $acquireNs / 1000000.0 } else { $null }
        PresentMs = if ($null -ne $presentNs) { $presentNs / 1000000.0 } else { $null }
        GpuShadowMeshMs = if ($null -ne $gpuShadowMeshNs) { $gpuShadowMeshNs / 1000000.0 } else { $null }
        GpuMeshMs = if ($null -ne $gpuMeshNs) { $gpuMeshNs / 1000000.0 } else { $null }
        UploadsCumulative = if ($null -ne $uploadsValue) { [long]$uploadsValue } else { $null }
        ReleasesCumulative = if ($null -ne $releasesValue) { [long]$releasesValue } else { $null }
        ModelCacheEntries = Get-OptionalNumber -Row $row -Column 'gpu_model_cache_entries'
        Placements = Get-OptionalNumber -Row $row -Column 'scene_model_placements'
        LiveMeshes = Get-OptionalNumber -Row $row -Column 'live_meshes'
        LiveMaterials = Get-OptionalNumber -Row $row -Column 'live_materials'
        TotalBytes = Get-OptionalNumber -Row $row -Column 'total_bytes_estimate'
        ResidentCpuBytes = Get-OptionalNumber -Row $row -Column 'cpu_asset_resident_bytes_estimate'
        PinnedCpuBytes = Get-OptionalNumber -Row $row -Column 'cpu_asset_pinned_bytes_estimate'
        UnpinnedCpuBytes = Get-OptionalNumber -Row $row -Column 'cpu_asset_unpinned_bytes_estimate'
        CpuBudgetBytes = Get-OptionalNumber -Row $row -Column 'cpu_asset_budget_bytes'
        CpuOverBudgetBytes = Get-OptionalNumber -Row $row -Column 'cpu_asset_over_budget_bytes'
        CpuLoadedCount = Get-OptionalNumber -Row $row -Column 'cpu_asset_loaded_count'
        CpuPinnedCount = Get-OptionalNumber -Row $row -Column 'cpu_asset_pinned_count'
        CpuBudgetEvictions = Get-OptionalNumber -Row $row -Column 'cpu_asset_budget_evictions_cumulative'
        InstanceCapacity = Get-OptionalNumber -Row $row -Column 'renderer_instance_capacity'
        GpuSampleSequence = if ($row.PSObject.Properties['gpu_sample_sequence']) { [string]$row.gpu_sample_sequence } else { '' }
        SurfacePresentMode = if ($row.PSObject.Properties['surface_present_mode']) { [string]$row.surface_present_mode } else { '' }
        SurfaceFrameLatency = Get-OptionalNumber -Row $row -Column 'surface_frame_latency'
        CpuBudgetEnabled = if ($row.PSObject.Properties['cpu_asset_budget_enabled']) { [string]$row.cpu_asset_budget_enabled } else { '' }
    })
}

$samples = @($sampleList.ToArray() | Sort-Object -Property Frame)
if ($samples.Count -eq 0) { throw "CSV contains no valid frame samples: $resolvedCsv" }

$previousUploads = $null
$uploadEventList = New-Object 'System.Collections.Generic.List[object]'
$uploadDeltaList = New-Object 'System.Collections.Generic.List[object]'
foreach ($sample in $samples) {
    if ($null -eq $sample.UploadsCumulative) { continue }
    if ($null -eq $previousUploads) {
        # The first captured value is the baseline; uploads during warmup are not visible as per-frame deltas.
        $delta = 0L
    }
    else {
        $delta = [long]$sample.UploadsCumulative - [long]$previousUploads
        if ($delta -lt 0) {
            $uploadEventList.Add([pscustomobject]@{
                Frame = $sample.Frame; Delta = $delta; Cumulative = $sample.UploadsCumulative
                FrameWallMs = $sample.FrameWallMs; GraphCpuMs = $sample.GraphCpuMs
                ModelCacheEntries = $sample.ModelCacheEntries; Placements = $sample.Placements
                Note = 'Cumulative counter decreased; inspect possible reset or counter semantics.'
            })
        }
        elseif ($delta -gt 0) {
            $uploadEventList.Add([pscustomobject]@{
                Frame = $sample.Frame; Delta = $delta; Cumulative = $sample.UploadsCumulative
                FrameWallMs = $sample.FrameWallMs; GraphCpuMs = $sample.GraphCpuMs
                ModelCacheEntries = $sample.ModelCacheEntries; Placements = $sample.Placements
                Note = if ($delta -gt $MaxUploadsPerFrame) { 'Exceeds configured per-sample limit.' } else { '' }
            })
        }
    }
    $uploadDeltaList.Add([pscustomobject]@{ Frame = $sample.Frame; Delta = $delta })
    $previousUploads = [long]$sample.UploadsCumulative
}

$maxUploadDelta = 0L
if ($uploadDeltaList.Count -gt 0) {
    $maxUploadDelta = [long](($uploadDeltaList | Measure-Object -Property Delta -Maximum).Maximum)
}
$uploadCheck = if ($maxUploadDelta -le $MaxUploadsPerFrame) { 'PASS' } else { 'REVIEW' }
$uploadEvents = @($uploadEventList.ToArray() | Sort-Object -Property Frame)
$slowSamples = @($samples | Sort-Object -Property FrameWallMs -Descending | Select-Object -First $SlowFrameCount)

$gpuSamples = @($samples)
if ($samples.Count -gt 0 -and $samples[0].GpuSampleSequence -ne '') {
    # GPU timestamps are asynchronous: a previous result may repeat across frames.
    # Count each submission sequence once when summarizing GPU execution time.
    $gpuSamples = @(
        $samples |
            Where-Object { $_.GpuSampleSequence -ne '' } |
            Group-Object -Property GpuSampleSequence |
            ForEach-Object { $_.Group[0] }
    )
}

$frameMetrics = @(
    [pscustomobject]@{ Label = 'Frame wall'; Property = 'FrameWallMs'; Items = $samples },
    [pscustomobject]@{ Label = 'Render graph CPU'; Property = 'GraphCpuMs'; Items = $samples },
    [pscustomobject]@{ Label = 'Surface acquisition'; Property = 'SurfaceAcquireMs'; Items = $samples },
    [pscustomobject]@{ Label = 'Surface presentation'; Property = 'PresentMs'; Items = $samples },
    [pscustomobject]@{ Label = 'GPU shadow + mesh (deduplicated submissions)'; Property = 'GpuShadowMeshMs'; Items = $gpuSamples },
    [pscustomobject]@{ Label = 'GPU mesh pass (deduplicated submissions)'; Property = 'GpuMeshMs'; Items = $gpuSamples }
)

$passSummary = @()
if ($rawPassRows.Count -gt 0 -and ($columns -contains 'pass_name') -and ($columns -contains 'pass_cpu_ns')) {
    $passSummary = @(
        $rawPassRows |
            Where-Object { -not [string]::IsNullOrWhiteSpace([string]$_.pass_name) -and -not [string]::IsNullOrWhiteSpace([string]$_.pass_cpu_ns) } |
            Group-Object -Property pass_name |
            ForEach-Object {
                $values = @(
                    foreach ($passRow in $_.Group) {
                        $ns = Get-OptionalNumber -Row $passRow -Column 'pass_cpu_ns'
                        if ($null -ne $ns) { $ns / 1000000.0 }
                    }
                )
                if ($values.Count -gt 0) {
                    $sorted = @($values | Sort-Object)
                    $p95Index = [Math]::Max(0, [Math]::Ceiling(0.95 * $sorted.Count) - 1)
                    [pscustomobject]@{
                        Name = $_.Name
                        Samples = $values.Count
                        MeanMs = [double](($values | Measure-Object -Average).Average)
                        P95Ms = [double]$sorted[$p95Index]
                        PeakMs = [double]$sorted[-1]
                    }
                }
            } |
            Sort-Object -Property PeakMs -Descending
    )
}

$latest = $samples[-1]
function Get-MaxSampleValue {
    param([object[]] $Items, [string] $Property)
    $values = @(
        foreach ($item in $Items) {
            $v = $item.PSObject.Properties[$Property].Value
            if ($null -ne $v) { [double]$v }
        }
    )
    if ($values.Count -eq 0) { return $null }
    return [double](($values | Measure-Object -Maximum).Maximum)
}

$frameStart = $samples[0].Frame
$frameEnd = $samples[-1].Frame
$timestamp = (Get-Date).ToString('yyyy-MM-dd HH:mm:ss zzz', $culture)
if ([string]::IsNullOrWhiteSpace($OutputPath)) {
    $outputDirectory = Split-Path -Parent $resolvedCsv
    $outputName = ([System.IO.Path]::GetFileNameWithoutExtension($resolvedCsv)) + '-summary.md'
    $resolvedOutput = Join-Path $outputDirectory $outputName
}
else {
    $resolvedOutput = [System.IO.Path]::GetFullPath($OutputPath)
    $outputDirectory = Split-Path -Parent $resolvedOutput
    if (-not [string]::IsNullOrWhiteSpace($outputDirectory) -and -not (Test-Path -LiteralPath $outputDirectory)) {
        New-Item -ItemType Directory -Path $outputDirectory -Force | Out-Null
    }
}

$lines = New-Object 'System.Collections.Generic.List[string]'
[void]$lines.Add('# Ribbon Engine - Focused Render Profile Summary')
[void]$lines.Add('')
[void]$lines.Add("- Generated: $timestamp")
[void]$lines.Add("- Source CSV: ``$resolvedCsv``")
[void]$lines.Add("- Frame samples: $($samples.Count) (engine frames $frameStart to $frameEnd)")
[void]$lines.Add('- Present mode: ' + [string]$latest.SurfacePresentMode)
if ($null -ne $latest.SurfaceFrameLatency) { [void]$lines.Add('- Desired maximum frame latency: ' + [string][long]$latest.SurfaceFrameLatency) }
[void]$lines.Add('- Note: frame and phase times are elapsed/CPU timings as defined by the source profiler; GPU timings are asynchronous samples.')
[void]$lines.Add('')

$peakFrameMs = Get-MaxSampleValue -Items $samples -Property 'FrameWallMs'
$peakGraphMs = Get-MaxSampleValue -Items $samples -Property 'GraphCpuMs'
$peakCpuOverBudget = Get-MaxSampleValue -Items $samples -Property 'CpuOverBudgetBytes'
$budgetCheck = if ($null -eq $peakCpuOverBudget -or $peakCpuOverBudget -le 0) { 'PASS (no observed unpinned-budget overshoot)' } else { 'REVIEW (overshoot observed)' }
$frameBudgetCheck = if ($null -ne $peakFrameMs -and $peakFrameMs -le 16.667) { 'PASS (peak at or below 16.667 ms)' } else { 'REVIEW (peak exceeds 16.667 ms)' }
[void]$lines.Add('## Key checks')
[void]$lines.Add('')
[void]$lines.Add('| Check | Result | Detail |')
[void]$lines.Add('|---|---|---|')
[void]$lines.Add("| Model GPU upload pacing | **$uploadCheck** | Max cumulative upload increase between adjacent captured samples: $maxUploadDelta; configured limit: $MaxUploadsPerFrame |")
[void]$lines.Add("| Frame budget (60 FPS reference) | **$frameBudgetCheck** | Peak frame wall: $(Format-Number $peakFrameMs) ms; peak graph CPU: $(Format-Number $peakGraphMs) ms |")
[void]$lines.Add("| CPU asset-cache budget | **$budgetCheck** | Peak reported bytes over unpinned budget: $(Format-Bytes $peakCpuOverBudget) |")
[void]$lines.Add('')
[void]$lines.Add('## Timing summary')
[void]$lines.Add('')
[void]$lines.Add('| Metric | Samples | Mean (ms) | P50 (ms) | P95 (ms) | P99 (ms) | Peak (ms) |')
[void]$lines.Add('|---|---:|---:|---:|---:|---:|---:|')
foreach ($metric in $frameMetrics) {
    $stats = Get-Stats -Items $metric.Items -Property $metric.Property
    [void]$lines.Add("| $($metric.Label) | $($stats.Count) | $(Format-Number $stats.Mean) | $(Format-Number $stats.P50) | $(Format-Number $stats.P95) | $(Format-Number $stats.P99) | $(Format-Number $stats.Peak) |")
}
[void]$lines.Add('')
[void]$lines.Add("## $SlowFrameCount slowest captured frames")
[void]$lines.Add('')
[void]$lines.Add('| Frame | Frame wall (ms) | Graph CPU (ms) | Surface acquire (ms) | GPU shadow + mesh (ms) | Upload delta | Uploads cumulative | GPU model cache | Placements |')
[void]$lines.Add('|---:|---:|---:|---:|---:|---:|---:|---:|---:|')
foreach ($s in $slowSamples) {
    $delta = ($uploadDeltaList | Where-Object { $_.Frame -eq $s.Frame } | Select-Object -First 1).Delta
    if ($null -eq $delta) { $delta = 'n/a' }
    $uploadsLabel = if ($null -eq $s.UploadsCumulative) { 'n/a' } else { [string]$s.UploadsCumulative }
    $cacheLabel = if ($null -eq $s.ModelCacheEntries) { 'n/a' } else { [string][long]$s.ModelCacheEntries }
    $placementsLabel = if ($null -eq $s.Placements) { 'n/a' } else { [string][long]$s.Placements }
    [void]$lines.Add("| $($s.Frame) | $(Format-Number $s.FrameWallMs) | $(Format-Number $s.GraphCpuMs) | $(Format-Number $s.SurfaceAcquireMs) | $(Format-Number $s.GpuShadowMeshMs) | $delta | $uploadsLabel | $cacheLabel | $placementsLabel |")
}
[void]$lines.Add('')
[void]$lines.Add('## Model GPU upload pacing')
[void]$lines.Add('')
[void]$lines.Add("- Configured review limit: **$MaxUploadsPerFrame model uploads per captured-frame interval**.")
[void]$lines.Add("- Maximum observed cumulative-counter increase between adjacent frame samples: **$maxUploadDelta**.")
[void]$lines.Add("- Result: **$uploadCheck**.")
[void]$lines.Add('- The first captured cumulative value is treated as the baseline; uploads before the first captured row (such as warmup uploads) cannot be attributed to individual captured frames.')
[void]$lines.Add('- This is a check of counter deltas between adjacent captured samples, not proof about activity outside the capture window.')
[void]$lines.Add('')
if ($uploadEvents.Count -gt 0) {
    [void]$lines.Add('| Frame | Upload delta | Uploads cumulative | Frame wall (ms) | Graph CPU (ms) | GPU model cache | Placements | Note |')
    [void]$lines.Add('|---:|---:|---:|---:|---:|---:|---:|---|')
    foreach ($event in $uploadEvents) {
        $note = [string]$event.Note
        [void]$lines.Add("| $($event.Frame) | $($event.Delta) | $($event.Cumulative) | $(Format-Number $event.FrameWallMs) | $(Format-Number $event.GraphCpuMs) | $($event.ModelCacheEntries) | $($event.Placements) | $note |")
    }
    [void]$lines.Add('')
}
else {
    [void]$lines.Add('No upload-counter increases occurred between the captured frame samples.')
    [void]$lines.Add('')
}

if ($passSummary.Count -gt 0) {
    [void]$lines.Add('## Render pass CPU summary')
    [void]$lines.Add('')
    [void]$lines.Add('| Pass | Samples | Mean CPU (ms) | P95 CPU (ms) | Peak CPU (ms) |')
    [void]$lines.Add('|---|---:|---:|---:|---:|')
    foreach ($pass in $passSummary) {
        [void]$lines.Add("| $($pass.Name) | $($pass.Samples) | $(Format-Number $pass.MeanMs) | $(Format-Number $pass.P95Ms) | $(Format-Number $pass.PeakMs) |")
    }
    [void]$lines.Add('')
}

[void]$lines.Add('## Resource and CPU asset cache snapshot')
[void]$lines.Add('')
[void]$lines.Add('| Metric | Latest | Peak in capture |')
[void]$lines.Add('|---|---:|---:|')
$resourceRows = @(
    [pscustomobject]@{ Label = 'Tracked GPU/renderer bytes estimate'; Property = 'TotalBytes'; Bytes = $true },
    [pscustomobject]@{ Label = 'CPU asset resident bytes estimate'; Property = 'ResidentCpuBytes'; Bytes = $true },
    [pscustomobject]@{ Label = 'CPU asset pinned bytes estimate'; Property = 'PinnedCpuBytes'; Bytes = $true },
    [pscustomobject]@{ Label = 'CPU asset unpinned cache bytes estimate'; Property = 'UnpinnedCpuBytes'; Bytes = $true },
    [pscustomobject]@{ Label = 'Configured unpinned cache budget'; Property = 'CpuBudgetBytes'; Bytes = $true },
    [pscustomobject]@{ Label = 'Unpinned bytes over budget'; Property = 'CpuOverBudgetBytes'; Bytes = $true },
    [pscustomobject]@{ Label = 'GPU model cache entries'; Property = 'ModelCacheEntries'; Bytes = $false },
    [pscustomobject]@{ Label = 'Live meshes'; Property = 'LiveMeshes'; Bytes = $false },
    [pscustomobject]@{ Label = 'Live materials'; Property = 'LiveMaterials'; Bytes = $false },
    [pscustomobject]@{ Label = 'CPU assets loaded'; Property = 'CpuLoadedCount'; Bytes = $false },
    [pscustomobject]@{ Label = 'CPU assets pinned'; Property = 'CpuPinnedCount'; Bytes = $false },
    [pscustomobject]@{ Label = 'Renderer instance capacity'; Property = 'InstanceCapacity'; Bytes = $false }
)
foreach ($item in $resourceRows) {
    $latestValue = $latest.PSObject.Properties[$item.Property].Value
    $peakValue = Get-MaxSampleValue -Items $samples -Property $item.Property
    if ($item.Bytes) {
        $latestLabel = Format-Bytes $latestValue
        $peakLabel = Format-Bytes $peakValue
    }
    else {
        $latestLabel = if ($null -eq $latestValue) { 'n/a' } else { [string][long]$latestValue }
        $peakLabel = if ($null -eq $peakValue) { 'n/a' } else { [string][long]$peakValue }
    }
    [void]$lines.Add("| $($item.Label) | $latestLabel | $peakLabel |")
}
[void]$lines.Add('')
$lastUploads = $latest.UploadsCumulative
$lastReleases = $latest.ReleasesCumulative
$lastEvictions = $latest.CpuBudgetEvictions
[void]$lines.Add('### Cumulative counters at capture end')
[void]$lines.Add('')
[void]$lines.Add('| Counter | Latest value |')
[void]$lines.Add('|---|---:|')
[void]$lines.Add("| Model GPU uploads | $(if ($null -eq $lastUploads) { 'n/a' } else { [string][long]$lastUploads }) |")
[void]$lines.Add("| Model GPU releases | $(if ($null -eq $lastReleases) { 'n/a' } else { [string][long]$lastReleases }) |")
[void]$lines.Add("| CPU budget-based evictions | $(if ($null -eq $lastEvictions) { 'n/a' } else { [string][long]$lastEvictions }) |")
[void]$lines.Add('')
[void]$lines.Add('## Interpretation notes')
[void]$lines.Add('')
[void]$lines.Add('- Memory values are engine estimates from the CSV, not process RSS or driver-reported total VRAM.')
[void]$lines.Add('- The CPU asset budget applies to unpinned cache entries; active strong leases may keep total resident CPU bytes above that budget.')
[void]$lines.Add('- Surface acquisition and presentation are wall-clock phases and can include driver/compositor waits.')
[void]$lines.Add('- Upload deltas are inferred from cumulative counters sampled once per frame. A counter change marks the interval between adjacent captured samples.')

$content = [string]::Join([Environment]::NewLine, $lines.ToArray()) + [Environment]::NewLine
$encoding = New-Object System.Text.UTF8Encoding($false)
[System.IO.File]::WriteAllText($resolvedOutput, $content, $encoding)
