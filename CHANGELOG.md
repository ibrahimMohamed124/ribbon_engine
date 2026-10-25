# Changelog

Notable user-facing changes are recorded here. Unreleased entries must be reviewed before creating a
version tag. The workspace is currently pre-1.0; this file does not imply a released version.

## Unreleased

### Rendering API / migration

- The legacy `ribbon_plugin_rendering::gpu::voxel::GpuRenderer` / `GpuPresenter`, the `gpu` feature,
  and its ECS backend selector were removed. New GPU 3D applications use `ribbon_plugin_render3d`;
  existing CPU legacy-scene applications use `ribbon_plugin_rendering::cpu` and its CPU-only ECS bridge.
- Documented `SurfaceShader::{albedo, albedo_with_materials, entity_surface}` as CPU-only extension
  callbacks. They are not translated into mesh GPU shader behavior. GPU materials use the data-driven
  `PbrMaterialDesc`/`TextureData` API; offscreen voxel chunks currently support per-block linear RGB
  tint through `VoxelBlockColor`, not per-block PBR texture assignment.
- Added behavioral tests asserting custom CPU surface callbacks affect CPU output and deterministic
  palette tests for the offscreen voxel tint/fallback contract.
- Added `docs/RENDERING_API_CONTRACTS.md` and `docs/RELEASE_CANDIDATE_CHECKLIST.md` to make current
  public API boundaries, intentional breaking changes, and outstanding release gates explicit.
