# Renderer plan: GPU-first, mesh-based, realism-oriented

This plan supersedes the "CPU/GPU parity" direction of `REALISM_PLAN.md` for
new engine rendering work. `REALISM_PLAN.md` remains relevant to STUDS while its
gameplay and rendering transition away from the legacy voxel raycaster is pending.

## Current implementation snapshot

The engine now has a usable native-surface mesh-rendering vertical slice:

- `engine_render::RenderGraph` validates named-resource dependencies and runs
  `FramePass` nodes in resource order. `GpuWindow` models clear, mesh passes,
  and final presentation as a graph chain; surface acquisition and
  `SurfaceTexture::present()` remain presenter responsibilities.
- `plugin_render3d` extracts ECS camera, transforms, built-in meshes, glTF model
  placements, voxel-mesh placements, and sunlight into `ExtractedScene3D`.
- The forward mesh shader uses a Cook-Torrance / GGX metallic-roughness BRDF,
  glTF base-color texture and factor, packed metallic-roughness map (G=roughness,
  B=metallic), a directional light, ambient contribution, and a single
  directional depth shadow map.
- Scene color renders to `Rgba16Float`, followed by a fullscreen Reinhard
  tone-mapping pass before the native window image is presented.
- Camera-frustum sphere culling filters color-pass instances. Off-screen models
  are still retained in the shadow draw list so they can cast shadows into the
  visible scene.
- `plugin_voxel` has greedy surface chunk meshing. It removes faces between
  solid voxels, merges adjacent equal-block faces, culls chunk boundary faces
  against neighboring world cells, and converts legacy Z-up coordinates to
  the mesh renderer's Y-up coordinates. `VoxelMeshInstance3D` sends these meshes
  through the same GPU mesh path as built-in geometry and glTF assets.
- `examples/graphics_showcase` and `examples/voxel_mesh_showcase` provide
  visible samples for the mesh/material and native voxel-mesh paths.

This is a foundation, not a claim that every advanced rendering feature is done.
STUDS still uses the legacy voxel renderer; the voxel showcase is the first
integrated vertical slice, not the game migration.

## Rendering decisions

1. New render features use wgpu rasterization and WGSL; the legacy voxel
   raymarcher is not the place for new engine rendering work.
2. Voxel worlds are converted to greedy-meshed chunks and drawn through
   `plugin_render3d`.
3. The CPU renderer is a frozen reference for legacy goldens. Avoid adding new
   engine capabilities there unless explicitly needed for compatibility.
4. `RenderGraph` orders passes by named resource dependencies. It is not yet a
   transient resource allocator, async-compute scheduler, or GPU profiler.
5. Forward PBR remains the initial path. Reconsider clustered or deferred
   rendering only if measured scene complexity justifies it.
6. GPU tests use offscreen rendering/readback and skip cleanly when an adapter is
   unavailable. Tests and GPU validation still need to be run on a machine with
   the Rust toolchain and graphics adapter available.
7. New engine code belongs under `crates/` and `plugins/`, not `engine/crates/*`.

## Target frame

The following is the long-term target, not a statement that every line is already
implemented:

```text
 1  Extract       ECS -> render-facing snapshot
 2  Cull          CPU frustum culling; instance batching (GPU culling later)
 3  Shadows       single directional map now; cascades / point / spot later
 4  Depth         opaque depth buffer now; dedicated pre-pass later if measured
 5  Opaque PBR    Cook-Torrance metallic-roughness into HDR RGBA16F
 6  Sky and fog   future sky model and volumetric fog
 7  Transparent   future alpha modes and sorting
 8  Post          Reinhard tonemap now; bloom, exposure controls, grade, AA later
 9  UI/editor     DrawList quads over the image
10  Present       wgpu surface
```

## Gap analysis

| Area | Present | Remaining work |
| --- | --- | --- |
| Frame graph | Validated resource dependency graph; native `GpuWindow` passes chained by color versions | Transient resource allocation, explicit graph nodes for every internal mesh subpass, detailed tracing/profiling |
| Mesh backend | Native wgpu surface, indexed meshes, depth buffer, GPU instance buffers | GPU-driven rendering, robust asset eviction, async upload, resource lifetime policy |
| Materials | Cook-Torrance/GGX, base-color factor/map, metallic/roughness factors/map, generated mips | Normal maps, occlusion maps, emissive, alpha modes, vertex colors, compression, material caching/sharing |
| Lighting | Directional sun, ambient term, single 1024 directional depth map | Cascaded shadows, multi-tap PCF/soft shadows, point/spot lights, physically based image-based lighting |
| HDR/post | `Rgba16Float` scene color and fullscreen Reinhard tonemapping | Exposure control, bloom, color grading, anti-aliasing, fog, AO/GTAO |
| Visibility | Conservative CPU frustum sphere culling for main color pass; off-screen objects remain in shadow draws | Better bounds for animated/deformed meshes, occlusion culling, LOD, GPU culling, metrics |
| Voxels | Greedy chunk surface meshing, block grouping, cross-chunk face culling, Y-up conversion, native GPU showcase | Dirty-chunk updates/remeshing, material atlas/texture IDs, chunk streaming, STUDS migration and raycaster deletion |
| Front end | Scene hierarchy, camera component, single active extracted camera | Multi-camera/render layers, picking, editor text UI |
| Animation | Transform animation systems and placeholder plugin | glTF skins/clips, morph targets, animation evaluation |
| Physics | Voxel collision | Mesh/AABB colliders, broadphase, robust 3D character controller |
| Audio | Audio output and sound bank | 3D spatialization, effects, voice prioritization |
| Assets | Embedded GLB / self-contained glTF, decoded images, synchronous weak handles | External glTF resources, async loading, robust hot-reload and cache eviction |
| Core | Logger, RNG, schedule stats | CPU/GPU profiling, engine configuration, localization |

## Phases and exit criteria

| Phase | State | Exit criteria |
| --- | --- | --- |
| 1–3: runtime, first mesh, glTF, shared 3D transforms | Implemented foundation | Windowed scene and imported GLB render; parented transforms propagate |
| 4a: first PBR/HDR path | Implemented foundation | Base color + metallic/roughness factors and packed map feed Cook-Torrance shader; HDR tone mapping runs |
| 4b: full material system | In progress | Normal, occlusion, emissive maps; alpha handling; conformance scenes and GPU verification |
| 5a: directional shadows | Implemented baseline | Single directional depth map is rendered and sampled |
| 5b: shadow quality | In progress | Multi-tap PCF, stable shadow bounds, acne/peter-panning regression coverage, cascades and additional light types |
| 6: advanced post | Planned | Bloom, AO, fog, exposure and quality controls |
| 7a: voxel-to-mesh vertical slice | Implemented | Greedy mesher and `voxel_mesh_showcase` draw chunks with native mesh pipeline |
| 7b: STUDS migration / legacy deletion | Not complete | STUDS gameplay renders meshes on native surface; legacy raycaster and unneeded legacy crates are removed safely |
| 8: advanced gameplay support | Planned | Skeletal animation, mesh colliders, 3D audio, state machines and profiling |
| 9: advanced renderer | Planned | IBL, TAA, LOD, GPU culling tracked individually |

## Repository hygiene

- Do not claim STUDS migration complete until `docs/PHASE9.md` and compatibility
  constraints have been resolved and the old voxel path has actually been removed.
- Split large renderer modules further when that improves API boundaries and
  resource ownership; avoid a cosmetic rewrite that breaks the working samples.
- Keep dependency rules in `tools/allowed_deps.toml`; run
  `python tools/check_deps.py` after manifest changes.
