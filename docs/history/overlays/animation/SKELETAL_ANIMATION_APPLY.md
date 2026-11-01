# Ribbon Skeletal Animation / GPU Skinning Overlay

This is an incremental source overlay for the Ribbon Engine checkout that already contains the
previously delivered streaming remaining-gaps work, the global GPU budget hotfix, and the shader
AssetRef lease hotfix. It adds glTF skeletal animation and vertex-stage GPU skinning; it does not
repackage or replace those earlier overlays.

## Apply

Run from the repository root on Windows:

```powershell
Expand-Archive `
  -LiteralPath .\Ribbon_Engine_Skeletal_Animation_Overlay.zip `
  -DestinationPath . `
  -Force
```

Run `cargo fmt` first, then the validation commands in the [implementation report](../../../animation/SKELETAL_ANIMATION_IMPLEMENTATION_REPORT.md).

## GPU-specific validation

The adapter-backed test is ignored by normal `cargo test` so a machine without a GPU adapter cannot
report a false pass. On a Windows machine with a working wgpu adapter, run:

```powershell
.\tools\test_skeletal_skinning_gpu.ps1
```

Run the existing real-adapter device-recreation test separately:

```powershell
.\tools\test_shader_pipeline_device_recovery.ps1
```

The skeletal GPU test asserts real vertex-stage palette movement in the color pass. Both color and
shadow WGSL modules are validated in the normal renderer tests, and the same joint palette is bound
for directional and spot shadow draws. There is not yet a separate pixel-level shadow-map assertion.
