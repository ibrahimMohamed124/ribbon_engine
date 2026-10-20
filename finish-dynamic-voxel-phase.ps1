# Validation-only preflight for Dynamic Voxel World Integration & Legacy Raycaster Migration.
# Run from any directory: powershell -ExecutionPolicy Bypass -File .\finish-dynamic-voxel-phase.ps1
# This script never creates commits or pushes to a remote.

$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$RepoRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
Push-Location $RepoRoot

function Invoke-CheckedCommand {
    param(
        [Parameter(Mandatory = $true)][string]$File,
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [Parameter(Mandatory = $true)][string]$Label
    )
    Write-Host "`n==> $Label" -ForegroundColor Cyan
    & $File @Arguments
    $ExitCode = $LASTEXITCODE
    if ($ExitCode -ne 0) {
        throw "$Label failed with exit code $ExitCode. No commits or push were attempted."
    }
}

try {
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        throw 'cargo was not found. Install/select the Rust toolchain before running this script.'
    }

    $ChunkedPath = Join-Path $RepoRoot 'plugins/voxel/src/chunked.rs'
    $OffscreenPath = Join-Path $RepoRoot 'plugins/render3d/src/offscreen.rs'
    $StudsPath = Join-Path $RepoRoot 'games/studs/src/app.rs'
    $StudsReadmePath = Join-Path $RepoRoot 'games/studs/README.md'
    $StudsSurfacesPath = Join-Path $RepoRoot 'games/studs/src/surfaces.rs'
    $ShowcasePath = Join-Path $RepoRoot 'examples/voxel_mesh_showcase/src/main.rs'
    $ShowcaseReadmePath = Join-Path $RepoRoot 'examples/voxel_mesh_showcase/README.md'
    foreach ($Path in @($ChunkedPath, $OffscreenPath, $StudsPath, $StudsReadmePath, $StudsSurfacesPath, $ShowcasePath, $ShowcaseReadmePath)) {
        if (-not (Test-Path -LiteralPath $Path)) {
            throw "Required migration file is missing: $Path"
        }
    }

    $ChunkedSource = Get-Content -LiteralPath $ChunkedPath -Raw
    if ($ChunkedSource -notmatch 'pub fn set_block\(' -or
        $ChunkedSource -notmatch 'pub fn rebuild_dirty_chunks\(' -or
        $ChunkedSource -notmatch 'fn invalidate_voxel\(') {
        throw 'ChunkedVoxelWorld is missing edit/dirty-remesh APIs required by this phase.'
    }

    $OffscreenSource = Get-Content -LiteralPath $OffscreenPath -Raw
    if ($OffscreenSource -notmatch 'pub struct OffscreenVoxelRenderer' -or
        $OffscreenSource -notmatch 'fn sync_dirty_chunks\(' -or
        $OffscreenSource -notmatch 'release_mesh\(' -or
        $OffscreenSource -notmatch 'pub fn render_with_materials\(' -or
        $OffscreenSource -notmatch 'sync_block_materials\(') {
        throw 'The engine offscreen mesh renderer is missing dirty-chunk upload/eviction or texture-material wiring.'
    }

    $StudsSurfacesSource = Get-Content -LiteralPath $StudsSurfacesPath -Raw
    if ($StudsSurfacesSource -notmatch 'pub\(crate\) fn studs_gpu_voxel_materials\(' -or
        $StudsSurfacesSource -notmatch 'albedo\.rgba' -or
        $StudsSurfacesSource -notmatch 'VoxelBlockMaterial') {
        throw 'STUDS GPU block materials must select game assets and send them through the engine material API.'
    }

    $StudsSource = Get-Content -LiteralPath $StudsPath -Raw
    if ($StudsSource -notmatch 'OffscreenVoxelRenderer::new' -or
        $StudsSource -notmatch 'chunked_world_mut\(\)' -or
        $StudsSource -notmatch 'render_with_materials' -or
        $StudsSource -notmatch 'studs_gpu_voxel_materials' -or
        $StudsSource -match 'plugin_rendering::gpu::voxel::GpuRenderer::new') {
        throw 'STUDS GPU routing is not migrated to the engine mesh renderer.'
    }
    if ($StudsSource -match 'drop\(view\);') {
        throw 'STUDS render path must let GameReadView borrows end through normal lexical scope; remove drop(view). '
    }
    if ($StudsSource -notmatch 'mirror_framebuffer_horizontal\(&mut renderer\.output\)' -or
        $StudsSource -notmatch 'mod gpu_camera_compat_tests') {
        throw 'STUDS GPU camera handedness compatibility or its framebuffer regression checks are missing.'
    }
    $InputTestsPath = Join-Path $RepoRoot 'games/studs/tests/input.rs'
    if (-not (Test-Path -LiteralPath $InputTestsPath) -or
        (Get-Content -LiteralPath $InputTestsPath -Raw) -notmatch 'wasd_are_the_default_walk_and_strafe_bindings') {
        throw 'STUDS must document and cover the W/A/S/D default movement bindings.'
    }

    $StudsReadme = Get-Content -LiteralPath $StudsReadmePath -Raw
    if ($StudsReadme -notmatch 'Engine boundary' -or
        $StudsReadme -notmatch 'consumer and stress-test' -or
        $StudsReadme -notmatch 'GPU resource management') {
        throw 'games/studs/README.md must clearly document the engine/game responsibility boundary.'
    }

    $ShowcaseSource = Get-Content -LiteralPath $ShowcasePath -Raw
    if ($ShowcaseSource -notmatch 'ChunkedVoxelWorld::new' -or
        $ShowcaseSource -notmatch 'rebuild_dirty_chunks\(\)' -or
        $ShowcaseSource -notmatch 'Transform3D::from_translation\(Vec3::from_array\(mesh.origin_y_up\)\)' -or
        $ShowcaseSource -notmatch 'CLEAR_BLOCK_FRAME' -or
        $ShowcaseSource -notmatch 'RESTORE_BLOCK_FRAME') {
        throw 'voxel_mesh_showcase must demonstrate world chunking, correct chunk origins, dirty remeshing and a reversible live edit.'
    }
    $ShowcaseReadme = Get-Content -LiteralPath $ShowcaseReadmePath -Raw
    if ($ShowcaseReadme -notmatch 'frame 120' -or $ShowcaseReadme -notmatch 'frame 180') {
        throw 'voxel_mesh_showcase README must document the dynamic boundary-edit demonstration.'
    }

    Invoke-CheckedCommand -File 'cargo' -Arguments @('fmt', '--all') -Label 'Format Rust workspace'
    Invoke-CheckedCommand -File 'cargo' -Arguments @('fmt', '--all', '--', '--check') -Label 'Check Rust formatting'

    if (Get-Command python -ErrorAction SilentlyContinue) {
        Invoke-CheckedCommand -File 'python' -Arguments @('tools/check_deps.py') -Label 'Check dependency policy'
    }
    elseif (Get-Command py -ErrorAction SilentlyContinue) {
        Invoke-CheckedCommand -File 'py' -Arguments @('-3', 'tools/check_deps.py') -Label 'Check dependency policy'
    }
    else {
        throw 'Python 3 was not found; tools/check_deps.py is part of the validation matrix.'
    }

    # This workspace includes many example/test targets. On Windows, rustc can exhaust
    # the system commit limit/page file if Cargo compiles several targets concurrently.
    # Apply low-memory settings BEFORE the first build/test command, including the normal
    # workspace tests (the earlier version enabled these only for no-default-features).
    Write-Host "`n==> Enable low-memory Cargo settings for the full validation matrix" -ForegroundColor Cyan
    $CargoEnvironmentNames = @('CARGO_BUILD_JOBS', 'CARGO_INCREMENTAL', 'CARGO_PROFILE_DEV_DEBUG', 'CARGO_PROFILE_TEST_DEBUG', 'CARGO_PROFILE_RELEASE_DEBUG')
    $CargoEnvironmentSnapshot = @{}
    foreach ($EnvironmentName in $CargoEnvironmentNames) {
        $EnvironmentItem = Get-Item -Path "Env:$EnvironmentName" -ErrorAction SilentlyContinue
        if ($null -eq $EnvironmentItem) {
            $CargoEnvironmentSnapshot[$EnvironmentName] = @{ Exists = $false; Value = $null }
        }
        else {
            $CargoEnvironmentSnapshot[$EnvironmentName] = @{ Exists = $true; Value = $EnvironmentItem.Value }
        }
    }

    try {
        $env:CARGO_BUILD_JOBS = '1'
        $env:CARGO_INCREMENTAL = '0'
        $env:CARGO_PROFILE_DEV_DEBUG = '0'
        $env:CARGO_PROFILE_TEST_DEBUG = '0'
        $env:CARGO_PROFILE_RELEASE_DEBUG = '0'

        Invoke-CheckedCommand -File 'cargo' -Arguments @('check', '--locked', '--workspace', '--all-targets', '--jobs', '1') -Label 'cargo check (workspace/all targets, low-memory)'
        Invoke-CheckedCommand -File 'cargo' -Arguments @('clippy', '--locked', '--workspace', '--all-targets', '--jobs', '1', '--', '-D', 'warnings') -Label 'cargo clippy (warnings denied, low-memory)'
        Invoke-CheckedCommand -File 'cargo' -Arguments @('build', '--locked', '--workspace', '--examples', '--jobs', '1') -Label 'Build workspace examples (low-memory)'
        Invoke-CheckedCommand -File 'cargo' -Arguments @('test', '--locked', '-p', 'plugin_voxel', '--jobs', '1') -Label 'Test chunk storage, invalidation and meshing'
        Invoke-CheckedCommand -File 'cargo' -Arguments @('test', '--locked', '--workspace', '--jobs', '1') -Label 'Test workspace (low-memory, serialized)'
        Invoke-CheckedCommand -File 'cargo' -Arguments @('build', '--locked', '--workspace', '--all-targets', '--no-default-features', '--jobs', '1') -Label 'Build without default features (low-memory, serialized)'
        Invoke-CheckedCommand -File 'cargo' -Arguments @('test', '--locked', '--workspace', '--no-default-features', '--jobs', '1') -Label 'Test without default features (low-memory, serialized)'
        Invoke-CheckedCommand -File 'cargo' -Arguments @('build', '--locked', '--release', '-p', 'studs', '--features', 'gpu', '--jobs', '1') -Label 'Build STUDS release GPU path (serialized)'
    }
    catch {
        Write-Host "`nLow-memory validation did not finish." -ForegroundColor Yellow
        Write-Host "If compiler output contains 'The paging file is too small for this operation to complete' or OS error 1455, Windows ran out of committed virtual memory. Close other memory-heavy applications and ensure the Windows paging file is system-managed (or increase it), then rerun this script. That message alone does not indicate a Rust source error." -ForegroundColor Yellow
        throw
    }
    finally {
        foreach ($EnvironmentName in $CargoEnvironmentNames) {
            $SavedEnvironment = $CargoEnvironmentSnapshot[$EnvironmentName]
            if ($SavedEnvironment.Exists) {
                Set-Item -Path "Env:$EnvironmentName" -Value $SavedEnvironment.Value
            }
            else {
                Remove-Item -Path "Env:$EnvironmentName" -ErrorAction SilentlyContinue
            }
        }
    }

    Write-Host "`nAll automated checks passed." -ForegroundColor Green
    Write-Host 'Manual GPU runtime checks are still required on the target Windows machine:' -ForegroundColor Yellow
    Write-Host '  cargo run -p voxel_mesh_showcase -- --frames 240'
    Write-Host '  cargo run --release -p studs -- --renderer gpu'
    Write-Host '  cargo run --release -p studs -- --renderer cpu'
    Write-Host 'In voxel_mesh_showcase, verify the floor voxel disappears at frame 120 and returns at frame 180 with no chunk seams.'
    Write-Host 'In STUDS, move the mouse and ArrowRight to the right and verify the view turns right; verify W/S walk forward/back and A/D strafe relative to the view in GPU and CPU modes. Check world texture, flashlight readability, creature, pages, HUD, resize/restart and CPU fallback before removing legacy raycaster code.'
    Write-Host 'Known visual gap: GPU creature animation is a multipart mesh proxy; CPU retains the baked sprite atlas until a generic GPU billboard/sprite path is integrated.'
    Write-Host 'No commits or push were attempted.'
}
catch {
    Write-Host $_ -ForegroundColor Red
    exit 1
}
finally {
    Pop-Location
}
