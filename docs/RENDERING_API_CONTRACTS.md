# Rendering API contracts and migration guide

This document defines the currently supported public rendering contracts. The workspace packages
are still `0.1.x`; these contracts make current behavior explicit but do not promise semver stability
for an eventual `1.0.0` without the release review below.

## Backend ownership

| API | Supported role | Deliberate boundary |
| --- | --- | --- |
| `ribbon_plugin_rendering::cpu::render::lighting::SurfaceShader` | CPU voxel-scene shading callbacks | CPU-only. Its arbitrary Rust callbacks are not evaluated by the GPU mesh renderer. |
| `ribbon_plugin_rendering::cpu::render::raycaster` / `CpuRenderer` | Software/reference rendering, including STUDS CPU mode | CPU-only renderer. Retained intentionally; not the modern 3D mesh path. |
| `ribbon_plugin_rendering::ecs::RenderingPlugin` | ECS bridge for `RenderScene`, `VoxelWorldRes`, and `RenderSettings { width, height }` | CPU-only compatibility bridge; it does not own/select a GPU backend. |
| `ribbon_plugin_render3d::Render3dPlugin` | Native-surface ECS mesh rendering | Primary 3D GPU path; consumes extracted mesh/model/voxel data and engine material contracts. |
| `ribbon_plugin_render3d::MeshRenderer` | GPU mesh/material resource owner and renderer | Uses generation-checked `MeshId`/`MaterialId`, `Draw`, `Scene`, and PBR material descriptions. |
| `ribbon_plugin_render3d::OffscreenVoxelRenderer` | Offscreen dynamic voxel chunk mesh rendering with framebuffer readback | Uses `ChunkedVoxelWorld` and `VoxelBlockColor`; currently uses a shared white PBR material multiplied by per-block tint. |
| `ribbon_plugin_render3d::PbrMaterialDesc` / `TextureData` | Data-driven metallic-roughness GPU mesh materials and supported maps | Does not translate arbitrary CPU callbacks to GPU shader code. |

## CPU `SurfaceShader` contract

`SurfaceShader::albedo`, `albedo_with_materials`, and `entity_surface` are callbacks owned by the
CPU renderer. `albedo` returns a scalar grayscale value which the CPU shading path combines with
lighting, ambient occlusion, material-table information, and render style. `entity_surface` returns
grayscale brightness and an emissive flag for CPU-rendered box entities. The default implementation
of `albedo_with_materials` delegates to `albedo`.

These functions are not a renderer-neutral material language. A GPU shader cannot execute arbitrary
Rust trait callbacks, and the callback's inputs/results do not define a portable GPU material graph.
The behavior is therefore intentionally **not** promised to match `Render3dPlugin` or
`OffscreenVoxelRenderer`. The tests in `plugins/rendering/tests/surface_shader_contract.rs` verify
that the custom callbacks continue to affect CPU output; they do not assert cross-backend pixel
parity.

## Supported GPU material controls today

- `PbrMaterialDesc` provides a linear RGBA base-color factor, metallic/roughness factors, optional
  sRGB base-color and emissive textures, optional linear metallic-roughness, tangent-space normal,
  and occlusion maps, occlusion strength, and a linear emissive factor.
- `TextureData` is tightly packed row-major RGBA8. Base-color/emissive inputs are interpreted as
  sRGB; metallic-roughness, normal, and occlusion are linear data. Missing maps, empty/malformed byte
  buffers, overflow-sized dimensions, and dimensions beyond the active device limit use neutral
  fallback textures instead of being submitted as invalid GPU texture extents.
- `Draw::tint` is a linear RGB multiplier on top of the selected GPU material.
- `OffscreenVoxelRenderer::render` takes a `VoxelBlockColor` palette. A matching block ID receives
  that linear RGB tint; an ID absent from the palette receives neutral gray (`[0.58, 0.58, 0.58]`).
  The current API does not assign separate PBR map sets/material handles per voxel block and does not
  call `SurfaceShader`. Use the native `MeshRenderer` material APIs for textured PBR meshes. A future
  shared block-material table can be designed separately if per-block PBR textures become required.

Do not infer alpha blend/mask support, IBL, cascaded shadows, or GPU execution of CPU callbacks from
the existence of a PBR descriptor; use `plugins/render3d/README.md` for the active feature/limit list.

## Intentional breaking change: legacy GPU raycaster removal

The following old API was removed and must not be reintroduced as a hidden feature flag:

- `ribbon_plugin_rendering::gpu::voxel::GpuRenderer` and `GpuPresenter`;
- the `ribbon_plugin_rendering` `gpu` feature and its legacy renderer-only dependencies;
- `render_gpu_frame_system`, `GpuBackend`, `RenderBackend::{Cpu, Gpu}` and `RenderSettings.backend`.

Migration choices:

- New 3D GPU scenes: use `GpuWindowPlugin` + `Render3dPlugin`, or `OffscreenVoxelRenderer` if the app still needs a `Framebuffer`/`WindowFrame` bridge.
- Existing CPU/legacy voxel scenes: use `ribbon_plugin_rendering::cpu` and the CPU-only `RenderingPlugin` with `RenderSettings { width, height }`.
- Gameplay ray queries: use the relevant world query API directly; do not add a rendering backend dependency to perform a hit query.

The removed renderer's CPU/GPU parity test was specific to that implementation. The active GPU renderer has independent mesh/material, shader-validation, depth, transform, texture, and optional GPU lifecycle tests. Cross-renderer parity is not claimed.

## Pre-1.0 API policy

Until the workspace versioning process intentionally moves packages to `1.0.0`, callers should treat public APIs as pre-stable. Any deliberate public API break must update this document, the package-level README, relevant code docs, examples, tests, and the changelog. Before `1.0.0`, review public exports and ownership/lifetime behavior together; do not mark every `pub` item stable by accident.
