# Renderer plan: GPU-first, mesh-based, realism-oriented

This plan supersedes the "CPU/GPU parity" direction of `REALISM_PLAN.md` for
new engine rendering work. `REALISM_PLAN.md` remains relevant to STUDS' game-specific
visual settings and effects; the STUDS GPU rendering route has been migrated to the mesh renderer.

## Current implementation snapshot

The engine now has a usable native-surface mesh-rendering vertical slice:

- `ribbon_render::RenderGraph` validates named-resource dependencies and runs
  `FramePass` nodes in resource order. `GpuWindow` models clear, mesh passes,
  and final presentation as a graph chain; surface acquisition and
  `SurfaceTexture::present()` remain presenter responsibilities.
- `ribbon_plugin_render3d` extracts ECS camera, transforms, built-in meshes, glTF model
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
- `studs_voxel` has `ChunkedVoxelWorld`: edits carry a revision, invalidate the owner chunk
  and any face-adjacent chunks affected by a boundary change, and remesh only dirty chunks.
  The greedy mesher reads the complete world so faces spanning adjacent chunks are hidden.
- the STUDS-owned `studs_voxel_gpu::OffscreenVoxelRenderer` drains chunk updates, incrementally uploads meshes,
  releases replaced/empty GPU buffers and reuses mesh slots. It renders using the shared
  HDR/PBR/depth/shadow pipeline and returns an engine framebuffer for `WindowFrame` users.
- The native ECS `Render3dPlugin` path also releases stale uploaded voxel meshes when their
  extracted instances disappear. `STUDS --renderer gpu` now uses the offscreen mesh renderer
  bridge rather than the legacy GPU raycaster. The game's palette and entity appearances remain
  game-owned; generic meshing, invalidation, GPU mesh storage, and rendering are engine-owned.
- `examples/graphics_showcase` and `games/studs/showcases/voxel_mesh_showcase` provide visible samples for
  the mesh/material and native voxel-mesh paths.

This is a foundation, not a claim that every advanced rendering feature is done. The Windows
runtime smoke checks for `graphics_showcase`, `voxel_mesh_showcase`, and STUDS in CPU/GPU modes
have been reported passing. The CPU raycaster remains the intentional software/reference fallback.
The legacy GPU voxel raycaster, its `gpu` Cargo feature, the generic ECS GPU route, and the old
CPU/GPU parity test have now been removed. `RenderingPlugin` is CPU-only; GPU applications use
`ribbon_plugin_render3d`. STUDS still presents through offscreen readback/`WindowFrame`; native-surface
presentation for STUDS, full parity for game-specific flashlight effects, LOD, and streaming remain
separate work.

## Rendering decisions

1. New render features use wgpu rasterization and WGSL; the legacy voxel
   raymarcher is not the place for new engine rendering work.
2. Voxel worlds are converted to greedy-meshed chunks and drawn through
   `ribbon_plugin_render3d`.
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
| Voxels | Mutable `ChunkedVoxelWorld`, edit revisions, dirty-owner/neighbor invalidation, incremental greedy remeshing, boundary culling, Y-up conversion, GPU uploads/eviction/reused slots, STUDS GPU route uses `studs_voxel_gpu::OffscreenVoxelRenderer`; Windows runtime smoke checks reported passing | Material atlas/texture IDs, chunk streaming/async remeshing, in-game block-edit mechanic, native-surface STUDS presentation |
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
| 7a: voxel-to-mesh vertical slice | Implemented | Greedy mesher and native mesh pipeline render chunk-local meshes at their world origins |
| 7b: Dynamic voxel integration / STUDS migration | Implemented; target Windows manual smoke checks reported passing | STUDS GPU route uses the engine mesh renderer, dynamic edits refresh only affected chunks (and face neighbors), empty meshes release GPU buffers, overlays/gameplay remain game-owned; CPU reference remains and legacy GPU code is disconnected from STUDS |
| 8: advanced gameplay support | Planned | Skeletal animation, mesh colliders, 3D audio, state machines and profiling |
| 9: advanced renderer | Planned | IBL, TAA, LOD, GPU culling tracked individually |

## Repository hygiene

- Do not claim all legacy raycaster code has been deleted: the CPU raycaster remains a deliberate
  fallback. The legacy GPU renderer is removed from production source and feature wiring; a source-level
  retirement regression test guards against reintroducing it, and an offscreen GPU integration test
  covers dynamic chunk mesh upload, replacement, boundary invalidation, and eviction when an adapter
  is available. The manual STUDS runtime checks have passed per the target-machine report.
- Split large renderer modules further when that improves API boundaries and
  resource ownership; avoid a cosmetic rewrite that breaks the working samples.
- Keep dependency rules in `tools/allowed_deps.toml`; run
  `python tools/check_deps.py` after manifest changes.
