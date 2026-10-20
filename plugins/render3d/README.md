# plugin_render3d

Native-surface 3D mesh rendering for the ECS engine. It consumes resolved
`GlobalTransform3D` values from `engine_scene` and draws through the `GpuWindow`
frame-pass / `RenderGraph` path.

## Current path

- Instanced indexed mesh rendering with a depth buffer.
- ECS extraction for camera, built-in mesh, glTF model, voxel mesh, and sun data.
- Cook-Torrance/GGX metallic-roughness shading with base-color factors/textures
  and glTF packed metallic-roughness textures.
- Linear HDR scene target (`Rgba16Float`) and fullscreen Reinhard tone mapping.
- A single directional depth shadow map.
- Conservative CPU camera-frustum culling for the color pass; objects not in the
  camera view remain in the shadow pass list.
- Cached immutable voxel-mesh GPU uploads keyed by retained `Arc` ownership, with eviction
  when extracted voxel mesh instances disappear.
- `OffscreenVoxelRenderer`, a backend-neutral framebuffer bridge that incrementally consumes
  `ChunkedVoxelWorld` updates, uploads/releases GPU chunk meshes, renders through the same PBR/
  HDR/depth/shadow pipeline, and returns engine framebuffer pixels for existing presenters.
- `VoxelBlockMaterial`, an application-provided per-block descriptor for a linear tint, roughness,
  metallic factor and optional sRGB albedo data. GPU texture/material upload is cached by block ID;
  games choose assets and tint, while the engine owns sampling and GPU lifetime.

## Dynamic voxel use

Use `OffscreenVoxelRenderer::new(width, height)` with a mutable
`plugin_voxel::ChunkedVoxelWorld`. Each `render` drains that world's dirty chunk updates,
updates GPU mesh storage by chunk coordinate, applies a caller-provided linear-color block
palette and draws optional unit-cube instances. Empty chunk updates release old GPU geometry;
mesh slots are reused after edits rather than accumulating a new buffer for every revision.
The offscreen output is tone-mapped and converted to sRGB before readback into
`engine_render::Framebuffer`. `GpuContext`/adapter initialization can fail on a target without
a usable GPU adapter, so callers should retain a fallback renderer.

This adapter exists for applications such as STUDS that still present through
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

This is an intentionally focused first PBR implementation, not a complete
production renderer. It does not yet support normal/occlusion/emissive maps,
alpha blend/mask modes, cascaded or point/spot shadows, soft multi-tap PCF,
IBL, bloom, AO, fog, GPU-driven culling, skinned meshes, external glTF resource
resolution, or native-surface presentation through the `WindowFrame` adapter. STUDS' GPU route now uses
`OffscreenVoxelRenderer`; its CPU reference backend and the old `plugin_rendering` raycaster
source remain available during rollout. Manual GPU gameplay smoke checks must pass before
that compatibility implementation is removed. See
[`../../docs/RENDERER_PLAN.md`](../../docs/RENDERER_PLAN.md) and
[`../../docs/DYNAMIC_VOXEL_WORLD_MIGRATION.md`](../../docs/DYNAMIC_VOXEL_WORLD_MIGRATION.md).
