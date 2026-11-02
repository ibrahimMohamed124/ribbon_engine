# Changelog

Notable user-facing changes are recorded here. Unreleased entries must be reviewed before creating a
version tag. The workspace is currently pre-1.0; this file does not imply a released version.

## Unreleased

### Audio / ATHAR runtime and packaging

- The modern audio ECS transform path now reads `GlobalTransform3D` and derives listener orientation from the world quaternion using Y-up / local -Z-forward. Legacy `AudioListener` remains a separate compatibility path; `ribbon_math` and STUDS coordinate conventions are unchanged.
- Added regression tests for front/back/left/right/above/below spatial direction, listener quaternion rotation, invalid transforms, transformed acoustic zones, and listener removal. Parent hierarchy and device-backed playback still require the Rust test suite and runtime validation on a supported host.
- ATHAR now registers `AudioPlugin`, enables Kira output through its default `audio` feature (while retaining a no-default-features headless build), attempts device initialization without failing the game when no device is available, and emits procedural ambience, footstep, door, key, and save-point audio events. Save-point persistence, monster behavior, and door-close gameplay remain unimplemented.
- ATHAR resolves assets from command-line/environment overrides or beside the executable, with a debug-only source fallback. Added an ATHAR package builder and a separate CI artifact path while retaining the STUDS artifact. CI's package script smoke-tests relocated asset resolution; CI execution remains the verification authority.

### 3D lighting and directional shadow stability

- Directional shadow projection now derives coverage from the active camera frustum, uses a rotation-invariant light-space extent, snaps its light-space XY center to the shadow texel grid, and fits depth coverage to the frustum. Cascades and configurable shadow distance/resolution remain deferred.
- Point-light uniform selection now ranks valid lights by estimated camera-space contribution (distance, range, intensity, and color luminance) and uses a stable value tie-breaker instead of ECS extraction order. The shader capacity remains eight and point lights remain unshadowed.
- Added CPU regression coverage for frustum coverage, sub-texel camera translation stability, invalid camera matrices, importance ranking, camera movement, deterministic tie-breaking, invalid lights, and shader-capacity truncation. GPU-backed visual validation and Cargo execution remain required on a supported host.

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
- Added `docs/history/RENDERING_API_CONTRACTS.md` and `docs/release/RELEASE_CANDIDATE_CHECKLIST.md` to make current
  public API boundaries, intentional breaking changes, and outstanding release gates explicit.

### Interactive showcases

- Added a reusable alpha-blended `WindowTextOverlayPass` using the shared engine bitmap font for
  compact native-window diagnostics and control guides.
- Made `graphics_showcase` expose its controls/status in-window, support right-mouse camera orbit, mouse-wheel zoom, bounded camera movement, and a complete reset path.
- Made `voxel_mesh_showcase` interactive with bounded source-grid selection, block placement/removal/material cycling, deterministic reset, a floating target marker, and live chunk/mesh diagnostics.
- Expanded showcase input, selection, reset, overlay geometry, and chunk-boundary regression coverage.
- Extended CI definitions for no-default-feature Clippy, targeted showcase regressions,
  release-example builds, and a source audit guarding against reintroduction of the retired GPU
  voxel raycaster. These workflow definitions have not been run as part of this change.
