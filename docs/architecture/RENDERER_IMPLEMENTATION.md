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
   Reinhard tone mapper writes the acquired surface image. The post pass includes basic bloom,
   optional GPU-adapted exposure, display color grading, FXAA, vignette, animated film grain,
   chromatic aberration and an optional depth-reconstructed spatial GTAO-style pass.
4. **Directional and spotlight shadows.** Depth-only passes create one camera-frustum-fitted,
   texel-snapped directional shadow map and one selected perspective spotlight shadow map.
   Both are sampled with depth-comparison PCF. Cascades, multiple spotlight casters, point-light
   shadows, runtime quality settings and cascade blending remain incomplete.
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
- The PBR shader supports normal, material AO and emissive textures, plus up to eight
  deterministically selected unshadowed point lights. Vertex colors, image-based lighting,
  multiple spotlight shadow casters and point-light shadow visibility remain unimplemented.
- Directional shadows use one fixed-resolution, frustum-fitted and texel-snapped map with 3x3 PCF;
  cascades and configurable shadow distance/resolution remain unimplemented.
- The render graph is a scheduling/dependency graph, not a transient GPU resource
  allocator or complete graph of every internal render subpass.
- Normal/AO/emissive texture sampling, height/distance fog, alpha material modes, per-material
  double-sided pipelines, and a compact bright-pass bloom contribution are implemented in
  `ribbon_plugin_render3d`. GPU culling, LOD, skeletal
  animation, GPU-driven culling and world streaming remain planned work. The legacy GPU voxel raycaster
  deletion is complete; the CPU raycaster remains supported.

## Anti-aliasing paths

`ribbon_plugin_render3d::AntiAliasingSettings` selects `None`, `Msaa2x`, `Msaa4x`, `Msaa8x`, or `Fxaa`. MSAA capability negotiation intersects the active adapter's `Rgba16Float` color-format sample counts and resolve support with the `Depth32Float` attachment sample counts and required render/texture usages. The mesh pass renders into a multisampled HDR color target with a depth target using the same sample count, then resolves into the single-sample HDR target consumed by bloom and tone mapping. Unsupported requests fall back to the highest lower common sample count, or one sample when no valid HDR resolve path exists. Target resizing recreates the multisampled color target and matching depth attachment at the renderer's latched sample count.

Set `AntiAliasingSettings` before the first renderer-owned GPU upload (including generic GPU warmups). Sample-dependent pipelines and generation-checked GPU resource handles are owned by the renderer; changing the MSAA count later is intentionally not applied and produces a diagnostic rather than invalidating live handles. FXAA can be toggled at runtime: it runs as a fullscreen spatial filter after HDR+bloom tone mapping. If FXAA is selected after an MSAA renderer has already initialized, it is layered over the latched MSAA sample count instead of replacing it. It is not TAA; there are no projection jitters, motion vectors, history buffers, reprojection, or disocclusion handling.

The FXAA shader and sample-count negotiation have CPU/WGSL regression tests. An ignored GPU integration test exercises the MSAA HDR resolve followed by the FXAA tone-mapping pass on an adapter that supports the required formats. Passing headless tests alone does not establish visual edge quality; compare the graphics showcase on the target GPU.

## Screen-space ambient occlusion

`AmbientOcclusionSettings` is an optional ECS resource installed with the renderer. It is disabled
by default and exposes bounded radius, intensity, bias, power and Low/Medium/High sampling presets.
The compute pass reconstructs world positions from the main `Depth32Float` attachment and the
active inverse view-projection matrix, estimates a local normal from depth-continuous neighbors,
and samples a deterministic screen-space horizon kernel. Separate compute pipelines read
single-sample and multisampled depth textures; the latter uses the nearest depth among the color
pass samples. A full-resolution `Rgba8Unorm` visibility target is sampled by the fullscreen display
pass, where it multiplies the scene HDR contribution before bloom is added.

This is a spatial GTAO-style approximation, not a complete production GTAO implementation. It has
no temporal reprojection/history denoising and no G-buffer geometric-normal input. Because the
current forward renderer does not separate ambient illumination from direct lighting in an
intermediate buffer, the visibility factor affects the scene HDR contribution as a whole rather
than ambient light alone. The full-resolution visibility target and compute pipelines are retained
by the tone mapper while AO is disabled; only the compute dispatch is skipped. Current post-process
GPU timestamp queries do not isolate this compute pass; CPU profiling reports command-encoding time,
not GPU execution duration. Use the ignored single-sample and MSAA GPU tests plus visual comparison
on the target adapter before treating the effect as validated on a platform.

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


## glTF alpha material paths

The current material path preserves glTF `alphaMode`, `alphaCutoff`, and `doubleSided`; the default `graphics_showcase` scene includes a generated five-primitive material wall for visual inspection of all three alpha modes and reversed-winding double-sided geometry. `OPAQUE` ignores alpha and writes depth; `MASK` discards by base-color factor alpha multiplied by base-color texture alpha in both color and shadow passes; `BLEND` uses conventional alpha blending, depth testing without depth writes, and object-center camera-space back-to-front sorting. Blend materials do not cast the current shadow maps, and intersecting transparent surfaces are not order independent. Double-sided materials select per-material no-cull variants and flip back-face normals; ordinary materials retain back-face culling.

## HDR post-processing controls

`PostProcessingSettings` is a public ECS world resource installed with neutral defaults by `Render3dPlugin`. Runtime changes are uploaded through fixed-size uniform buffers; the tone-mapping, bloom and exposure pipelines are retained across settings changes. `AutoExposureSettings` controls key luminance, exposure bounds and asymmetric adaptation rates. When enabled, a compute pass samples a 16x16 grid from the linear HDR texture, computes average log luminance, and exponentially adapts a persistent GPU scalar before the bloom and display passes. No HDR pixels are read back to CPU memory.

`ColorGradingSettings` offers exposure compensation in EV, contrast, saturation, temperature and tint. `VignetteSettings`, `FilmGrainSettings` and `ChromaticAberrationSettings` independently configure radial darkening, animated monochrome noise and radial RGB channel offsets. Inputs are sanitized and clamped before uniform upload to avoid NaN/Inf shader values or unbounded effect strength. Defaults are neutral apart from the pre-existing 1.15 manual exposure and tone-map/bloom behavior. `PostProcessingSettings::cinematic()` provides a restrained example preset.

The exposure reduction currently uses deterministic 16x16 spatial sampling rather than a luminance histogram. It is a low-cost adaptive baseline, not a physically calibrated camera model; histogram percentiles, local exposure, HDR output/display metadata, temporal AO reprojection/denoising and temporal AA remain future work.
