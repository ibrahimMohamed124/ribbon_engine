# plugin_render3d

Native-surface 3D mesh rendering for the ECS engine. It consumes resolved
`GlobalTransform3D` values from `engine_scene` and draws through the `GpuWindow`
frame-pass / `RenderGraph` path.

## Current path

- Instanced indexed mesh rendering with a depth buffer.
- ECS extraction for camera, built-in mesh, glTF model, voxel mesh, directional/spot/point lights, and fog settings.
- Cook-Torrance/GGX metallic-roughness shading with base-color factors/textures
  and glTF packed metallic-roughness textures.
- Linear HDR scene target (`Rgba16Float`) and fullscreen Reinhard tone mapping.
- A directional depth shadow map plus one perspective shadow map for the first shadow-casting spotlight; both use 3x3 PCF and depth bias.
- Up to eight non-shadowed point lights.
- glTF base-color, metallic-roughness, tangent-space normal, occlusion and emissive maps.
- Exponential height/distance fog and a quarter-resolution bright-pass bloom contribution before tone mapping.
- Conservative CPU camera-frustum culling for the color pass; objects not in the
  camera view remain in the shadow pass list.
- Cached immutable voxel-mesh GPU uploads keyed by retained `Arc` ownership, with eviction
  when extracted voxel mesh instances disappear.
- `OffscreenVoxelRenderer`, a backend-neutral framebuffer bridge that incrementally consumes
  `ChunkedVoxelWorld` updates, uploads/releases GPU chunk meshes, renders through the same PBR/
  HDR/depth/shadow pipeline, and returns engine framebuffer pixels for existing presenters.

## Dynamic voxel use

Use `OffscreenVoxelRenderer::new(width, height)` with a mutable
`plugin_voxel::ChunkedVoxelWorld`. Each `render` drains that world's dirty chunk updates,
updates GPU mesh storage by chunk coordinate, applies a caller-provided linear-color block
palette and draws optional unit-cube instances. Empty chunk updates release old GPU geometry;
mesh slots are reused after edits rather than accumulating a new buffer for every revision.
The offscreen output is tone-mapped and converted to sRGB before readback into
`engine_render::Framebuffer`. `GpuContext`/adapter initialization can fail on a target without
a usable GPU adapter, so callers should retain a fallback renderer.

This adapter exists for applications that still present through
`WindowFrame`. Applications using `GpuWindowPlugin` can continue to use the native-surface
`Render3dPlugin` and `VoxelMeshInstance3D` ECS components directly.

## Basic use

Add `PlatformPlugin`, `ScenePlugin`, `GpuWindowPlugin`, and `Render3dPlugin` in
that dependency order. Spawn an entity with `Transform3D` and `Camera3D`, then
spawn `MeshInstance`, `ModelInstance`, or `VoxelMeshInstance3D` entities.
The `examples/graphics_showcase`, `examples/model_viewer`,
`examples/scene_hierarchy_3d`, and `examples/voxel_mesh_showcase` applications
show these paths in context.

## Limitations

This is an intentionally focused PBR implementation, not a complete production
renderer. It does not yet support alpha blend/mask modes, multiple shadow-casting
spotlights, cascaded shadows, image-based lighting, screen-space ambient occlusion,
GPU-driven culling, skinned meshes, or external glTF resource resolution. The
spotlight-shadow and bloom implementations still need validation across a range of
Windows GPU adapters; a headless spot-lit corridor test writes `target/spotlight_corridor.png`
when an adapter is available. Applications can use `OffscreenVoxelRenderer` for offscreen
voxel rendering; `plugin_rendering` also retains its CPU and GPU voxel raycasters. See
[`../../docs/RENDERER_PLAN.md`](../../docs/RENDERER_PLAN.md) and
[`../../docs/DYNAMIC_VOXEL_WORLD_MIGRATION.md`](../../docs/DYNAMIC_VOXEL_WORLD_MIGRATION.md).
