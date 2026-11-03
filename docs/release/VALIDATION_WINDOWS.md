# Windows validation commands

Run these commands from the workspace root (`D:\exp\ribbon_engine\engine`) after extracting the delivered source archive. Use fresh, separate Cargo target directories so retained captures or prior build outputs under the repository's existing `target/` are not deleted or overwritten.

```powershell
$ErrorActionPreference = 'Stop'
$env:CARGO_BUILD_JOBS = '2'
$env:CARGO_TARGET_DIR = Join-Path $env:TEMP 'ribbon-engine-restructure-default'

cargo fmt --all
if ($LASTEXITCODE -ne 0) { throw 'cargo fmt failed' }
cargo fmt --all -- --check
if ($LASTEXITCODE -ne 0) { throw 'format check failed' }

cargo check --locked --workspace --all-targets
if ($LASTEXITCODE -ne 0) { throw 'workspace check failed' }
cargo clippy --locked --workspace --all-targets -- -D warnings
if ($LASTEXITCODE -ne 0) { throw 'workspace Clippy failed' }
cargo build --locked --workspace --examples
if ($LASTEXITCODE -ne 0) { throw 'workspace example build failed' }
cargo test --locked --workspace
if ($LASTEXITCODE -ne 0) { throw 'workspace tests failed' }

cargo test --locked -p studs
if ($LASTEXITCODE -ne 0) { throw 'STUDS tests failed' }
cargo tree -p ribbon_plugin_render3d
if ($LASTEXITCODE -ne 0) { throw 'render3d dependency tree failed' }
cargo tree -p ribbon_plugin_physics
if ($LASTEXITCODE -ne 0) { throw 'physics dependency tree failed' }
cargo tree -p ribbon_plugin_particles
if ($LASTEXITCODE -ne 0) { throw 'particle dependency tree failed' }

$renderTree = cargo tree --locked -p ribbon_plugin_render3d
if ($renderTree -match 'studs_voxel|studs_voxel_gpu|studs_cpu_renderer') { throw 'render3d depends on STUDS-owned voxel packages' }
$physicsTree = cargo tree --locked -p ribbon_plugin_physics
if ($physicsTree -match 'studs_voxel|studs_voxel_gpu|studs_cpu_renderer') { throw 'physics depends on STUDS-owned voxel packages' }
$particlesTree = cargo tree --locked -p ribbon_plugin_particles
if ($particlesTree -match 'ribbon_plugin_render3d|studs_voxel|studs_cpu_renderer') { throw 'particles must not depend on renderer or game code' }
python games/athar/tools/validate_horror_assets.py
if ($LASTEXITCODE -ne 0) { throw 'Athar asset validation failed' }
python tools/check_deps.py
if ($LASTEXITCODE -ne 0) { throw 'dependency policy check failed' }

$env:CARGO_TARGET_DIR = Join-Path $env:TEMP 'ribbon-engine-restructure-no-default'
cargo build --locked --workspace --all-targets --no-default-features
if ($LASTEXITCODE -ne 0) { throw 'no-default-features build failed' }
cargo test --locked --workspace --no-default-features
if ($LASTEXITCODE -ne 0) { throw 'no-default-features tests failed' }

Remove-Item Env:CARGO_TARGET_DIR
Remove-Item Env:CARGO_BUILD_JOBS
```

The script deliberately formats the workspace before checking formatting. Keep both the default-feature and no-default-feature target directories until validation has completed. If a command fails, preserve its first complete diagnostic rather than cleaning the repository's existing `target/` folder.

## Structural checks

Run from the same workspace root:

```powershell
# The first command must produce no paths. The engine must own no voxel implementation.
Get-ChildItem crates,plugins -Recurse -File -Include *.rs,*.toml,*.wgsl,*.md,*.ps1,*.py,*.yml,*.ron |
    ForEach-Object { Select-String -Path $_.FullName -Pattern 'voxel' -List -CaseSensitive:$false } |
    Select-Object -ExpandProperty Path -Unique

# Confirm both the mesh-based game and the default game target are registered.
cargo metadata --locked --no-deps --format-version 1 | ConvertFrom-Json |
    Select-Object -ExpandProperty packages |
    Where-Object { $_.name -in @('athar','studs','ribbon_input','ribbon_profiling') } |
    Select-Object name, manifest_path

# Exercise the native Athar window separately after the non-interactive matrix passes.
cargo run --locked -p athar
if ($LASTEXITCODE -ne 0) { throw 'Athar launch failed' }

# Verify both STUDS renderer backends and inspect camera/input behavior manually.
cargo run --locked -p studs -- --renderer cpu
if ($LASTEXITCODE -ne 0) { throw 'STUDS CPU launch failed' }
cargo run --locked -p studs -- --renderer gpu
if ($LASTEXITCODE -ne 0) { throw 'STUDS GPU launch failed' }
```

The particle package is required to depend on `ribbon_render_contracts`, not on `ribbon_plugin_render3d`; inspect `cargo tree -p ribbon_plugin_particles` to confirm the implementation boundary. Also inspect `cargo tree -p ribbon_plugin_render3d` and `cargo tree -p ribbon_plugin_physics` to ensure neither pulls in `studs_voxel`, `studs_voxel_gpu`, or `studs_cpu_renderer`. Do not interpret source-level checks as proof of GPU output correctness: manually verify mouse look, WASD movement, cursor capture/release, room readability, lighting/shadows, the rain particle emitter at the village transition, and normal shutdown in the Windows GPU run.
