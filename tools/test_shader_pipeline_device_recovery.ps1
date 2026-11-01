param(
    [string]$Package = "ribbon_plugin_render3d"
)

$ErrorActionPreference = "Stop"

Write-Host "Running the ignored real-adapter shader/pipeline recreation test..." -ForegroundColor Cyan
Write-Host "This test requires a working wgpu adapter and is not a CPU/mock test." -ForegroundColor Yellow

cargo test -p $Package real_adapter_recreation_rebuilds_retained_shader_and_pipeline -- --ignored --nocapture
if ($LASTEXITCODE -ne 0) {
    throw "Real-adapter device recreation test failed with exit code $LASTEXITCODE"
}

Write-Host "Real-adapter shader/pipeline recreation test passed." -ForegroundColor Green
