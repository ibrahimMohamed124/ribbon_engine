# ATHAR / Ribbon Engine compatibility note

## What the reported workspace log shows

The first hard failure is `rustc-LLVM ERROR: out of memory` while several large crates (including `naga`, `wgpu`, and `rapier3d`) are compiling. The subsequent `E0463: can't find crate` errors appear after compiler processes fail and are not sufficient evidence that those crates are missing from the workspace. The first rerun should therefore lower build concurrency and debug-info memory before changing dependencies or package paths.

## Static API audit of ATHAR against this workspace tree

- `games/athar/Cargo.toml` declares the engine crates it imports, including `ribbon_plugin_render3d`, `ribbon_plugin_gltf`, `ribbon_plugin_physics`, `ribbon_plugin_weather`, `ribbon_assets`, and `ribbon_render`.
- ATHAR uses the engine-re-exported `ribbon_plugin_render3d::glam` types alongside `Transform3D`, `Camera3D`, lights, and model instances from the render plugin.
- ATHAR uses the public, compatibility-preserving `ribbon_plugin_physics::{slide_capsule_xz, Aabb3}` static-collision helper; it does not pretend to use Rapier bodies for this custom camera controller.
- `WeatherPlugin` registers `RainSettings` and the rain update system; ATHAR toggles the settings on the room-to-village transition.
- The asset validation script passed for all 12 packaged self-contained GLB models in this environment.

This is a source-level audit, not a substitute for compiling the Windows workspace. `tools/validate_athar_engine.ps1` performs the compile/test checks on the target machine and stops at the first real failure.

## Run

From the workspace root in PowerShell:

```powershell
.\tools\validate_athar_engine.ps1
```

For a quick manual smoke test after the script passes:

```powershell
cargo run -p athar -- --frames 120
```
