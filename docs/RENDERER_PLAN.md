# Renderer plan: GPU-first, mesh-based, realism-oriented

This plan supersedes the "CPU/GPU parity" direction of `REALISM_PLAN.md` for new
engine work. `REALISM_PLAN.md` stays valid for STUDS on the legacy voxel
raymarcher until phase 7 below moves it over.

## Where the engine is today

- The legacy renderer under `engine/crates/render-gpu` draws a full-screen
  triangle and raymarches a voxel grid in a fragment shader. It remains separate
  from the newer ECS-based windowed mesh pipeline.
- The current workspace also contains `plugins/render3d`, a wgpu rasterization
  pipeline with vertex/index buffers, an instanced cube path, depth buffering,
  directional/ambient lighting, and imported glTF meshes.
- `plugin_gltf` loads embedded `.glb` / self-contained `.gltf` assets through
  `AssetServer`; `plugin_render3d` uploads model geometry, base-color textures,
  and generated mip chains and caches uploaded parts by asset handle.
- The older `plugins/rendering` path still owns voxel-oriented CPU/GPU rendering
  and legacy shading features. STUDS now reaches both through the ECS App and
  shared frame contract, but this remains the voxel-raycaster path; it has not yet
  been migrated to the `plugin_render3d` mesh pipeline.
- `Transform3D` and `GlobalTransform3D` are now owned by `engine_scene` and use the shared `Parent`/`Children` hierarchy. `ScenePlugin` propagates 3D world transforms before render passes, and `plugin_render3d` consumes those global transforms. The legacy voxel world remains Z-up, while the mesh renderer is right-handed and Y-up; conversion is still required when meshing voxel chunks.
- The mesh pipeline is still foundational rather than a full modern PBR renderer:
  mesh picking, voxel chunk meshing, shadow maps, normal/ORM maps, HDR, bloom,
  ambient occlusion, and the broader post-processing chain remain future work.

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
| Front end | scene hierarchy, camera components, basic editor | mesh-picking UI, multiple active cameras, text GUI |
| Back end | wgpu window surface; `plugin_render3d` mesh rasterization and depth buffer | render-graph integration for the new mesh path, shadow maps, culling and GPU profiling |
| Materials, textures | GLB base-color upload and mip chains in `plugin_render3d`; CPU PBR maps in the legacy renderer | mesh-pipeline normal/ORM maps, HDR and compressed GPU texture formats |
| Audio | rodio output, sound bank | 3D positional audio, effects, voice priority |
| Animation | per-frame transform systems; placeholder animation crate | glTF skins and clips, timelines |
| Physics | voxel collision | mesh/AABB colliders, broad phase |
| State machines | none | generic FSM component |
| Resources | images, materials, RON scenes, embedded GLB/self-contained glTF assets | animation assets, external glTF buffer/image resolution, async loading |
| Core | logger, RNG, schedule stats | profiling (CPU and GPU timestamps), engine config, localization |
| Math | `Vect`/`Angles` in `engine_math`; 3D `Vec3`/`Mat4`/`Quat` via `glam` in `engine_scene` | one shared vector/quaternion API across 3D scene, mesh and voxel subsystems; voxel-to-Y-up conversion |
| Parsers | RON, embedded GLB/self-contained glTF | external glTF resources and broader asset formats |

New external crates (`glam`, `gltf`, `bytemuck`, `pollster`) must be added to
`tools/allowed_deps.toml` or `tools/check_deps.py` fails.

## Status

- Phase 1 (window presents through wgpu): done. `GpuWindowPlugin` owns the
  context and surface; drawing steps register as `FramePass` objects and run
  after the clear.
- Phase 2 (first mesh): done. `plugin_render3d` draws instanced cubes with a
  depth buffer and a directional light. `examples/cube_spin` shows it.
- 3D conventions: right-handed, Y-up, camera looks down -Z (glTF). Voxel
  worlds are Z-up and need a conversion when phase 7 meshes them.
- Phase 3 (glTF): model loading and shared transform hierarchy are implemented.
  `plugin_gltf` loads `.glb` (and self-contained `.gltf`) through `AssetServer`
  into `ModelAsset` (primitives, materials, decoded images, flattened node
  placements). `plugin_render3d` uploads models once per asset handle, with sRGB
  base color textures, linear-light mip chains and 8x anisotropic sampling.
  `examples/model_viewer` loads any supported `.glb`. Not yet supported: normal/ORM
  maps, alpha modes, vertex colors, external buffer files, or skinned animation.
- The scene/transform portion of phase 3 is implemented: `Transform3D` is defined by `engine_scene`, with reflected/serialized `GlobalTransform3D` propagation through shared parent relations. Parent-relative transforms compose translation, quaternion rotation, and per-axis scale. Transform decomposition cannot represent shear from rotated children under non-uniformly scaled parents; a matrix-based world-transform contract may be needed if such cases become a supported use case.
- `examples/scene_hierarchy_3d` is a runnable visual/runtime diagnostic for shared 3D hierarchy transforms. It animates a parent, joint, link, and tip through `plugin_render3d`, then prints a PASS/FAIL report for the resolved world transforms. Run it with `cargo run -p scene_hierarchy_3d`.
- `examples/graphics_showcase` brings the current mesh features together in a
  single live scene: colored instanced cubes, cached GLB models and their base
  color textures, a computed-normal triangle, depth testing, directional and
  ambient light, animated transforms, and an orbiting camera. Its README lists
  the controls and explicitly documents renderer capabilities that are not yet
  wired in.
- STUDS' `--renderer cpu|gpu|auto` selects the CPU voxel renderer or the existing
  offscreen GPU voxel renderer through the ECS runtime. The GPU pixels still pass
  through readback and `WindowFrame`; this does not mark phase 7 complete because
  STUDS is not yet drawing voxel chunk meshes through `plugin_render3d` on the
  native wgpu surface.
- `assets/models/*.glb` are generated by `tools/make_test_models.py`.

## Phases and exit criteria

| Phase | Work | Done when |
| --- | --- | --- |
| 0 | Push current work, tag `voxel-cpu-baseline`, freeze `engine/crates/*` | tag exists, CI green |
| 1 | Window presents through wgpu (`GpuWindowPlugin`), then `glam` and a 3D transform | window shows an animated clear color; resize works |
| 2 | First mesh: hard-coded cube, vertex/index buffers, camera uniform, depth buffer | spinning cube, resize safe |
| 3 | glTF loader through `engine_assets`; `MeshAsset`, `MaterialAsset`, GPU cache, shared 3D hierarchy | a textured `.glb` renders and parented mesh instances follow global transforms |
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
