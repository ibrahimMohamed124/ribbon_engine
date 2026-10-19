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
- Cached immutable voxel-mesh GPU uploads keyed by retained `Arc` ownership.

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
resolution, or dynamic dirty-chunk remeshing. STUDS has not yet migrated from
its legacy voxel renderer. See [`../../docs/RENDERER_PLAN.md`](../../docs/RENDERER_PLAN.md).
