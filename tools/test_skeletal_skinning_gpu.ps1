$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
Push-Location $repoRoot
try {
    Write-Host "Running the ignored real-adapter skeletal skinning GPU test..."
    Write-Host "This test requires a working wgpu adapter and will fail if no adapter is available."
    cargo test -p ribbon_plugin_render3d --lib renderer::tests::skinned_vertex_palette_moves_geometry_in_the_color_pass_on_gpu -- --ignored --exact --nocapture
    if ($LASTEXITCODE -ne 0) {
        throw "Real-adapter skeletal skinning test failed with exit code $LASTEXITCODE"
    }
    Write-Host "Real-adapter skeletal skinning test passed."
}
finally {
    Pop-Location
}
