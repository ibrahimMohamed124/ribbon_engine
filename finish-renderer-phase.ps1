# Renderer phase preflight, focused commits, and push.
# Run from any directory: powershell -ExecutionPolicy Bypass -File .\finish-renderer-phase.ps1
# This script formats source first, runs the workspace checks, and does not commit/push
# unless every check succeeds and the user types COMMIT_AND_PUSH.

$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$RepoRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $RepoRoot

$ExpectedBranch = 'refactor/engine-cleanup'
$Remote = 'origin'
$Branch = ''

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
        throw "$Label failed with exit code $ExitCode. No commits or push have been attempted."
    }
}

function Invoke-Git {
    param([Parameter(Mandatory = $true)][string[]]$Arguments)
    & git @Arguments
    $ExitCode = $LASTEXITCODE
    if ($ExitCode -ne 0) {
        throw "git $($Arguments -join ' ') failed with exit code $ExitCode."
    }
}

function Test-StagedEmpty {
    & git diff --cached --quiet
    return ($LASTEXITCODE -eq 0)
}

function Commit-Group {
    param(
        [Parameter(Mandatory = $true)][string]$Message,
        [Parameter(Mandatory = $true)][string[]]$Paths
    )
    Invoke-Git -Arguments (@('add', '--') + $Paths)
    if (Test-StagedEmpty) {
        Write-Host "No changes in commit group: $Message" -ForegroundColor DarkYellow
        return
    }
    Invoke-Git -Arguments @('diff', '--cached', '--check')
    Invoke-Git -Arguments @('diff', '--cached', '--stat')
    Invoke-Git -Arguments @('commit', '-m', $Message)
}

try {
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        throw 'cargo was not found. Install/select the Rust toolchain before running this script.'
    }
    if (-not (Get-Command git -ErrorAction SilentlyContinue)) {
        throw 'git was not found.'
    }

    $Branch = (& git branch --show-current).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'Could not read the current Git branch.' }
    if ($Branch -ne $ExpectedBranch) {
        throw "Expected branch '$ExpectedBranch', current branch is '$Branch'. Checkout the intended branch before proceeding."
    }
    & git remote get-url $Remote *> $null
    if ($LASTEXITCODE -ne 0) { throw "Git remote '$Remote' is not configured; no checks or commits were started." }

    if (-not (Test-StagedEmpty)) {
        throw 'The Git index already contains staged changes. Unstage or commit them first so this script does not include unrelated changes.'
    }

    # Fail early if the PBR API fix was not applied to the working tree.
    $RendererSourcePath = Join-Path $RepoRoot 'plugins/render3d/src/renderer.rs'
    if (-not (Test-Path $RendererSourcePath)) {
        throw "Expected renderer source file was not found: $RendererSourcePath"
    }
    $RendererSource = Get-Content -LiteralPath $RendererSourcePath -Raw
    $PbrApiPattern = '(?s)pub fn upload_pbr_material_with_maps\(\s*&mut self,\s*device: &wgpu::Device,\s*queue: &wgpu::Queue,\s*material: PbrMaterialDesc'
    if ($RendererSource -notmatch $PbrApiPattern) {
        throw 'The working tree still has the old upload_pbr_material_with_maps API (separate base_color/metallic/roughness/texture parameters). Extract raycast_blocks_clippy_fix_overlay.zip into the project root and overwrite the files, then rerun this script. No commits or push have been attempted.'
    }

    # Fail early if the glTF fixture test still contradicts the glTF 2.0 default.
    $GltfSourcePath = Join-Path $RepoRoot 'plugins/gltf/src/lib.rs'
    if (-not (Test-Path $GltfSourcePath)) {
        throw "Expected glTF loader source file was not found: $GltfSourcePath"
    }
    $GltfSource = Get-Content -LiteralPath $GltfSourcePath -Raw
    if ($GltfSource -notmatch 'assert_eq!\(model\.materials\[0\]\.metallic_factor,\s*1\.0\);') {
        throw 'The glTF test still expects the wrong metallicFactor default. checker_cube.glb omits metallicFactor, so glTF 2.0 default is 1.0. Apply the latest renderer-phase fix archive before rerunning. No commits or push have been attempted.'
    }

    # PBR pixel checks must tolerate specular contribution and sample linear output.
    if ($RendererSource -notmatch 'r > 0 && r > 4 \* g && r > 4 \* b') {
        throw 'The renderer color assertion still expects exact pure red, which is invalid with PBR specular highlights. Apply raycast_blocks_render3d_testfix_overlay.zip and rerun. No commits or push have been attempted.'
    }
    $ModelSourcePath = Join-Path $RepoRoot 'plugins/render3d/src/model.rs'
    if (-not (Test-Path $ModelSourcePath)) {
        throw "Expected model-render test source file was not found: $ModelSourcePath"
    }
    $ModelSource = Get-Content -LiteralPath $ModelSourcePath -Raw
    if ($ModelSource -notmatch 'r > 2 \* b && r > 5' -or
        $ModelSource -notmatch 'b > 2 \* r && b > 5' -or
        $ModelSource -notmatch 'orange > 20 && blue > 20') {
        throw 'The checker texture test does not yet use the adapter-tolerant linear-color thresholds. Apply raycast_blocks_render3d_threshold_fix_overlay.zip and rerun. No commits or push have been attempted.'
    }

    # Normalize formatting first so the subsequent format check is meaningful.
    Invoke-CheckedCommand -File 'cargo' -Arguments @('fmt', '--all') -Label 'Format Rust workspace'
    Invoke-CheckedCommand -File 'cargo' -Arguments @('fmt', '--all', '--', '--check') -Label 'Check Rust formatting'

    if (Get-Command python -ErrorAction SilentlyContinue) {
        Invoke-CheckedCommand -File 'python' -Arguments @('tools/check_deps.py') -Label 'Check dependency policy'
    }
    elseif (Get-Command py -ErrorAction SilentlyContinue) {
        Invoke-CheckedCommand -File 'py' -Arguments @('-3', 'tools/check_deps.py') -Label 'Check dependency policy'
    }
    else {
        throw 'Python 3 was not found; tools/check_deps.py is part of the preflight.'
    }

    Invoke-CheckedCommand -File 'cargo' -Arguments @('check', '--locked', '--workspace', '--all-targets') -Label 'cargo check (workspace/all targets)'
    Invoke-CheckedCommand -File 'cargo' -Arguments @('clippy', '--locked', '--workspace', '--all-targets', '--', '-D', 'warnings') -Label 'cargo clippy (warnings denied)'
    Invoke-CheckedCommand -File 'cargo' -Arguments @('build', '--locked', '--workspace', '--examples') -Label 'Build workspace examples'
    Invoke-CheckedCommand -File 'cargo' -Arguments @('test', '--locked', '--workspace') -Label 'Test workspace'
    Invoke-CheckedCommand -File 'cargo' -Arguments @('build', '--locked', '--workspace', '--all-targets', '--no-default-features') -Label 'Build without default features'
    Invoke-CheckedCommand -File 'cargo' -Arguments @('test', '--locked', '--workspace', '--no-default-features') -Label 'Test without default features'

    Invoke-Git -Arguments @('diff', '--check')
    Write-Host "`nAll preflight checks passed." -ForegroundColor Green
    Write-Host 'The following commits contain only the renderer-phase paths listed in this script:'
    Write-Host '  1. feat(rendering): render graph, HDR/PBR/shadows, voxel mesher and showcase'
    Write-Host '  2. docs(rendering): implementation status, usage and roadmap'
    Write-Host '  3. chore: add renderer-phase preflight script'

    $Answer = Read-Host "Type COMMIT_AND_PUSH to create these commits and push to $Remote/$ExpectedBranch"
    if ($Answer -cne 'COMMIT_AND_PUSH') {
        Write-Host 'Cancelled. Checks passed, but no commits or push were performed.' -ForegroundColor Yellow
        exit 0
    }

    Commit-Group -Message 'feat(rendering): add graph-driven PBR HDR shadows and voxel meshes' -Paths @(
        'Cargo.toml',
        'Cargo.lock',
        'crates/engine_render/src/graph.rs',
        'crates/engine_render/src/gpu_window.rs',
        'plugins/gltf/src/lib.rs',
        'plugins/render3d/Cargo.toml',
        'plugins/render3d/src',
        'plugins/render3d/shaders',
        'plugins/voxel/src',
        'examples/voxel_mesh_showcase',
        'examples/graphics_showcase/src/main.rs',
        'tools/allowed_deps.toml'
    )

    Commit-Group -Message 'docs(rendering): document renderer capabilities and remaining gaps' -Paths @(
        'README.md',
        'ARCHITECTURE.md',
        'docs/RENDERER_PLAN.md',
        'docs/RENDERER_IMPLEMENTATION.md',
        'plugins/render3d/README.md',
        'plugins/voxel/README.md',
        'examples/graphics_showcase/README.md'
    )

    Commit-Group -Message 'chore: add renderer phase preflight script' -Paths @(
        'finish-renderer-phase.ps1'
    )

    Write-Host "`nPushing commits to $Remote/$ExpectedBranch..." -ForegroundColor Cyan
    & git push $Remote "HEAD:refs/heads/$ExpectedBranch"
    if ($LASTEXITCODE -ne 0) {
        throw "Push failed. Commits remain local on '$Branch'; resolve authentication/remote conflicts and push them manually."
    }

    Write-Host "`nSuccessfully pushed renderer phase commits:" -ForegroundColor Green
    Invoke-Git -Arguments @('log', '-3', '--oneline', '--decorate')
    Write-Host "`nUnrelated and untracked project files were not staged by this script."
}
catch {
    Write-Host $_ -ForegroundColor Red
    exit 1
}
