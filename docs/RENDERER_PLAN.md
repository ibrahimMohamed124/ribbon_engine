# Renderer plan: GPU-first, mesh-based, realism-oriented

This plan supersedes the "CPU/GPU parity" direction of `REALISM_PLAN.md` for new
engine work. `REALISM_PLAN.md` stays valid for STUDS on the legacy voxel
raymarcher until phase 7 below moves it over.

## Where the engine is today

- The only GPU renderer (`engine/crates/render-gpu`) draws one full-screen
  triangle and raymarches a voxel grid in a fragment shader. There are no
  vertex buffers, meshes or rasterization.
- Every shading feature exists twice: in the CPU renderer
  (`plugins/rendering`) and in WGSL.
- `engine_render` already provides a validated `RenderGraph` (nodes declare
  `reads`/`writes`) and a `GpuContext` (device, queue, optional surface,
  offscreen target, readback).
- `engine_math` has no Vec3/Mat4/Quat; `Transform` has a uniform `f32` scale
  and yaw/pitch `Angles` rotation.
- `engine_platform::WinitRunner` opens a window but, before phase 1, nothing
  presents pixels into it.

## Decisions

1. Rasterize meshes with wgpu and WGSL. Do not extend the voxel raymarcher.
2. Voxel worlds become meshes (greedy-meshed chunks) drawn by the same pipeline.
3. The CPU renderer is a frozen reference for legacy goldens. No new features.
4. Forward PBR with a depth pre-pass first; clustered lights and deferred
   shading are revisited only if the light count demands it.
5. GPU tests render offscreen and read back (`GpuContext::readback`). Tests
   skip cleanly when no adapter exists. Linux CI installs
   `mesa-vulkan-drivers`.
6. New code lives under `crates/` and `plugins/`. Nothing new is added to
   `engine/crates/*`.
7. `engine_math` must stay dependency-free (CI enforces it). 3D math types come
   from `glam` in a crate that is allowed to depend on it.

## Target frame

Each step is a `RenderNode` in the existing `RenderGraph`.

```text
 1  Extract       ECS -> RenderWorld (mesh instances, lights, camera)
 2  Cull          frustum culling, instance batching
 3  Shadows       spot (flashlight), cascaded (sun), cube (point)
 4  Depth/Normal  pre-pass
 5  AO            GTAO or SSAO
 6  Opaque PBR    Cook-Torrance, albedo/normal/ORM -> HDR RGBA16F
 7  Sky and fog   volumetric fog
 8  Transparent   sorted back to front
 9  Post          bloom, exposure, tonemap, grade, grain, anti-aliasing
10  UI/editor     DrawList quads over the image
11  Present       wgpu surface
```

## Gap analysis

| Area | Present | Missing |
| --- | --- | --- |
| Front end | scene hierarchy, `Camera`, basic editor | frustum culling, multiple cameras, text GUI |
| Back end | wgpu context, render graph | meshes, depth buffer, shadow maps, shader modules, GPU texture cache |
| Materials, textures | CPU PBR mip chains | GPU upload, compressed formats (KTX2/BC7) |
| Audio | rodio output, sound bank | 3D positional audio, effects, voice priority |
| Animation | placeholder crate | glTF skins and clips, timelines |
| Physics | voxel collision | mesh/AABB colliders, broad phase |
| State machines | none | generic FSM component |
| Resources | images, materials, RON scenes | meshes, animations, glTF, async loading |
| Core | logger, RNG, schedule stats | profiling (CPU and GPU timestamps), engine config, localization |
| Math | `Vect`, `Angles` | Vec3/Mat4/Quat in f32 |
| Parsers | RON | glTF |

New external crates (`glam`, `gltf`, `bytemuck`, `pollster`) must be added to
`tools/allowed_deps.toml` or `tools/check_deps.py` fails.

## Phases and exit criteria

| Phase | Work | Done when |
| --- | --- | --- |
| 0 | Push current work, tag `voxel-cpu-baseline`, freeze `engine/crates/*` | tag exists, CI green |
| 1 | Window presents through wgpu (`GpuWindowPlugin`), then `glam` and a 3D transform | window shows an animated clear color; resize works |
| 2 | First mesh: hard-coded cube, vertex/index buffers, camera uniform, depth buffer | spinning cube, resize safe |
| 3 | glTF loader through `engine_assets`; `MeshAsset`, `MaterialAsset`, GPU cache | a textured `.glb` renders |
| 4 | PBR: albedo/normal/ORM, directional/point/spot lights, HDR, tonemapping | PBR sample scene; offscreen test pins an image |
| 5 | Flashlight shadow map, cascaded shadows, GTAO | shadows visible; no acne on the test scene |
| 6 | Bloom, volumetric fog, grade, grain, quality tiers | tiers switch at runtime |
| 7 | Voxel chunk mesher on the same pipeline; migrate STUDS; delete the raymarcher | STUDS renders through meshes |
| 8 | Skeletal animation, mesh colliders, 3D audio, state machines, profiling | monster animates and collides |
| 9 | IBL and reflection probes, TAA, LODs, GPU culling | tracked per feature |

## Repository hygiene tied to this plan

- Finish the legacy-tree removal (see `SHIMS.md`) before phase 7.
- Split oversized `lib.rs` files when touched: `engine_assets`,
  `engine_commands`, `engine_schedule`, `engine_reflect`.
- New renderer code uses small modules (`context`, `pipelines/`, `passes/`) and
  readable `.wgsl` files under `shaders/`.
- Pick one crate naming scheme (`engine_*` versus `voxelworks-*`).
