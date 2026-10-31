# Renderer vertical-slice implementation notes

This document records the boundaries of the current implementation to avoid
confusing integrated capabilities with features that remain in the roadmap.

## Integrated in this phase

1. **Graph-driven native-window passes.** `GpuWindow` registers clear, each
   `FramePass`, and final presentation as nodes with named color-version
   dependencies. `RenderGraph` validates reads/writes and computes stable order.
   The presenter still owns the wgpu surface and calls `SurfaceTexture::present()`.
2. **PBR material inputs.** `ribbon_plugin_gltf` retains metallic/roughness factors and
   the packed metallic-roughness image index. When a glTF material omits
   `metallicFactor`, the loader preserves glTF 2.0’s default value of `1.0`.
   `ribbon_plugin_render3d` uploads base color
   as sRGB and the G=roughness/B=metallic map as linear data with generated mip
   levels. The shader evaluates a Cook-Torrance GGX forward BRDF. The public
   `PbrMaterialDesc` groups factors and optional texture maps into one descriptor,
   keeping the material upload API explicit without an oversized argument list.
3. **HDR and tone mapping.** Mesh color renders to `Rgba16Float`, then a fullscreen
   Reinhard tone mapper writes the acquired surface image.
4. **Directional shadows.** A depth-only directional-light pass creates a single
   shadow map; the material pass samples it with a depth-comparison sampler and
   depth bias. Cascades, PCF kernels, point/spot shadows and production-grade
   stability work remain incomplete.
5. **Main-pass visibility.** Conservative bounding-sphere/frustum tests reduce
   the color-pass instance list. The shadow list retains valid off-screen draws
   so objects outside the camera view can still cast shadows.
6. **STUDS dynamic voxel integration (consumer example, not engine functionality).** `games/studs/crates/studs_voxel::ChunkedVoxelWorld` wraps STUDS-owned voxel storage, revisions and a retained CPU mesh cache. `set_block` invalidates the owning chunk and face-neighbors when touching chunk boundaries. Dirty chunks are greedily remeshed against the complete world; empty results are explicit eviction updates. `games/studs/crates/studs_voxel_gpu::OffscreenVoxelRenderer` consumes those updates and uses the public mesh renderer API to replace GPU buffers and reuse freed mesh slots through the common HDR/PBR/depth/shadow pipeline. The generic ECS mesh path only renders ordinary mesh components; it contains no chunk tracking or voxel-specific eviction behavior.

## Deliberately not claimed complete

- The STUDS GPU code path uses the engine mesh renderer. The Windows runtime smoke checks for the
  graphics showcase, voxel mesh showcase, and STUDS in CPU/GPU modes were reported passing on the
  target machine. The CPU raycaster remains the intended reference/fallback. The legacy GPU raycaster,
  its package `gpu` feature, its generic ECS selector/path and its CPU/GPU parity integration test have
  been removed. `RenderingPlugin` now exposes only the supported CPU scene renderer; mesh applications
  use `ribbon_plugin_render3d`.
- STUDS still presents the mesh-rendered framebuffer through offscreen readback and `WindowFrame`;
  native-surface presentation and the old renderer's game-specific spot-light/noise effects are
  not part of this migration.
- The STUDS-owned API supports dynamic chunk edits and GPU mesh update/eviction. The
  `games/studs/showcases/voxel_mesh_showcase` removes and restores a floor block at the corner
  shared by chunks to exercise incremental remeshing and GPU-resource eviction; it is a STUDS
  showcase, not an engine example. Tests in the STUDS-owned crates cover owner-chunk updates,
  boundary neighbors, no-op edits, out-of-bounds edits and empty-chunk eviction updates.
- The PBR shader does not yet implement normal/occlusion/emissive textures,
  alpha modes, vertex colors, image-based lighting, or multiple point/spot lights.
- The current shadow map is a single fixed-resolution directional map without
  cascades or a multi-tap PCF kernel.
- The render graph is a scheduling/dependency graph, not a transient GPU resource
  allocator or complete graph of every internal render subpass.
- Normal/AO/emissive texture sampling, height/distance fog, and a compact bright-pass
  bloom contribution are implemented in `ribbon_plugin_render3d`. GPU culling, LOD, skeletal
  animation, GPU-driven culling and world streaming remain planned work. The legacy GPU voxel raycaster
  deletion is complete; the CPU raycaster remains supported.

## PBR renderer test expectations

The low-level GPU tests read the mesh pass before the production HDR tone-mapping and
surface transfer stage. They therefore assert color dominance and texture contrast in
linear output rather than exact display RGB values or display-space brightness. PBR
specular highlights can add small values to channels that are zero in a material's base
color, and metallic surfaces without image-based lighting can be intentionally dim. The
64x64 checker fixture requires more than 20 dominant-color pixels for each checker color;
this ensures both regions actually render without assuming a fixed pixel count across GPU
adapters and drivers. The glTF loader test separately checks the spec-defined defaults
(`metallicFactor = 1.0` and `roughnessFactor = 1.0`) for the checker fixture that omits both
values.

## Local validation

The GitHub Actions workflow is the authoritative cross-platform build, test, dependency-policy,
and Clippy matrix. For local Rust checks use `cargo fmt --check`, `cargo clippy --locked
--workspace --all-targets -- -D warnings`, and `cargo test --locked --workspace`; the optional
no-default-features matrix is also defined in `.github/workflows/ci.yml`. The STUDS GPU/CPU runtime
smoke checks on the target Windows machine have been reported passing. This retirement overlay must
still be revalidated with the full build/test matrix on the target Windows toolchain, because the current
editing environment does not provide `cargo` or `rustc`.
