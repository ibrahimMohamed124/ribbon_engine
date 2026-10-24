# Ribbon Voxel Rendering Plugin (`ribbon_plugin_rendering`)

This crate owns legacy voxel-scene/material types, CPU voxel rendering, and the generic ECS CPU rendering bridge. It uses shared `ribbon_render` frame contracts and `ribbon_plugin_voxel` world data. The optional `cpu` feature controls the CPU raycaster and ECS renderer; CPU is the default feature.

`cpu::render::raycaster` remains supported for software/reference rendering, and the existing game CPU mode intentionally continues to use this rendering technology. The generic `RenderingPlugin` and its `RenderSettings { width, height }` are CPU-only. They render the legacy `RenderScene` / `VoxelWorldRes` model and do not select or initialize a GPU renderer.

The former `gpu::voxel::GpuRenderer` / `GpuPresenter` implementation, the plugin's `gpu` feature, its shaders and its `RenderBackend` ECS selector have been removed. This is an intentional API break. New 3D GPU applications should use `ribbon_plugin_render3d`: `Render3dPlugin` for native-surface mesh rendering or `OffscreenVoxelRenderer` for a framebuffer/`WindowFrame` bridge. Mutable voxels flow through `ribbon_plugin_voxel::ChunkedVoxelWorld` and its greedy-meshed chunk updates.

The `ribbon_plugin_render3d` GPU dependencies are independent of this CPU-focused crate. Do not reintroduce the old renderer behind an unused feature flag. See `../../docs/LEGACY_RAYCASTER_RETIREMENT.md` and `../../docs/DYNAMIC_VOXEL_WORLD_MIGRATION.md` for the migration rationale, regression tests and validation matrix.
