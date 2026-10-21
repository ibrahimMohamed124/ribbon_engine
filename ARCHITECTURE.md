# Voxelworks Engine Architecture

The workspace is organized as a small engine core, optional plugins, sample
applications, and the STUDS game. Engine crates do not depend on game code.
`tools/allowed_deps.toml` is the machine-checked source for direct dependency
policy; run `python tools/check_deps.py` after changing a manifest.

## Repository layout

| Path | Contents | Package prefix |
| --- | --- | --- |
| `crates/` | Engine core: foundations, runtime, scene and rendering contracts, editor | `engine_*` |
| `plugins/` | Optional domain behavior installed into an `App` | `plugin_*` (folder name omits the prefix: `plugins/physics` is `plugin_physics`) |
| `examples/` | Small engine-only applications built on the ECS engine | none |
| `games/` | Games composed from the engine and plugins | none |
| `engine/` | **Legacy** `voxelworks-*` crates, kept only for `games/studs`; scheduled for deletion | `voxelworks-*` |
| `docs/`, `tools/`, `assets/` | Plans and decisions, dependency policy tooling, engine sample assets | |

New code never goes into `engine/`. Nothing outside `engine/` and `games/studs`
may depend on a `voxelworks-*` crate, and `tools/check_deps.py` fails the build if it does.

## Dependency graph

Arrows point from a crate to what it may depend on (the full allowlist is in
`tools/allowed_deps.toml`). Plugins and examples sit above this graph and
depend only on `engine_*` crates and, where listed, on other plugins.

```mermaid
graph TD
  math[engine_math]
  ecs[engine_ecs]
  events[engine_events] --> ecs
  schedule[engine_schedule] --> ecs
  reflect[engine_reflect] --> ecs
  app[engine_app] --> ecs & events & schedule & math
  plugins[engine_plugins] --> app
  platform[engine_platform] --> app & plugins & events & math
  assets[engine_assets] --> app & plugins & ecs & events
  scene[engine_scene] --> ecs & reflect & math & app & plugins & glam
  render[engine_render] --> app & plugins & platform & assets & math
  commands[engine_commands] --> ecs & reflect & scene
  editor[engine_editor] --> app & plugins & platform & render & reflect & commands & scene
```

## Which rendering crate is which

The names are close, so for the record:

| Package | Role |
| --- | --- |
| `engine_render` | Backend-neutral frame/draw contracts, resource-validated render graph, CPU presenter and optional wgpu context. Owns no scene knowledge. |
| `plugin_rendering` | Voxel scene/material data, CPU/GPU voxel raycasters, and the ECS bridge. Its `cpu` and `gpu` features are independently selectable; GPU mode uses offscreen rendering/readback. |
| `plugin_render3d` | Native-surface indexed/instanced mesh renderer with ECS extraction, depth buffering, metallic-roughness PBR, HDR tone mapping, directional and one spotlight shadow map (3x3 PCF), up to eight point lights, glTF base/normal/AO/emissive maps, exponential fog, bloom, frustum culling, greedy voxel mesh support, dynamic GPU mesh eviction and `OffscreenVoxelRenderer`. |

## Layers and responsibilities

| Layer | Packages | Does not do |
| --- | --- | --- |
| Foundations | `engine_math`, `engine_ecs`, `engine_events`, `engine_schedule` | Platform I/O, rendering, or gameplay |
| Runtime | `engine_app`, `engine_plugins`, `engine_platform`, `engine_assets` | Game-specific rules or GPU scene extraction |
| Scene and rendering contracts | `engine_reflect`, `engine_scene`, `engine_render`, `engine_commands`, `engine_editor` | Game assets, gameplay, or automatic parallelism |
| Plugins | `plugin_voxel`, `plugin_physics`, `plugin_animation`, `plugin_audio`, `plugin_rendering`, `plugin_editor_tools` | Owning the app loop or game state |
| Applications | `examples/*`, `games/studs` | Being dependencies of engine crates |

The permitted package edges are listed in `tools/allowed_deps.toml`. Math and
ECS are leaves; schedule/events build on ECS; app builds on those foundations;
platform, assets, scene, and rendering build on app contracts; plugins add
reusable domain behavior; games compose plugins.

## Frame order and plugins

`App::update` advances `Time`, then runs `First`, `PreUpdate`, zero or more
`FixedUpdate` steps, `Update`, `PostUpdate`, `Render`, and `Last`, in that
order. The fixed-step loop is capped at eight steps per frame. `Startup` and
`Shutdown` run once. Headless runs use a virtual 60 Hz delta.

Plugins are added in dependency order. Adding a plugin validates its declared
dependencies and unique name, calls `build`, and records it in `Plugins`;
`finish` hooks run when the app is built. Systems run serially in deterministic
stage order. A plugin should declare its dependencies, install resources, and
register systems in the appropriate stages.

## Writing a plugin or game

A plugin implements `engine_plugins::Plugin`, names its required plugins, and
adds reusable engine behavior. A game owns its content and gameplay, installs
the needed plugins, and adds game-specific systems or a compatibility bridge.
Keep dependencies from engine/plugin packages to game packages out of the
workspace.

## Current limits and migration state

- Systems run serially; system parameter injection is not implemented.
- Asset loading is synchronous and handles are weak.
- The render graph validates named-resource dependencies and orders native window frame passes, but is not yet a transient resource allocator, async scheduler, or GPU profiler.
- `plugin_voxel::ChunkedVoxelWorld` now owns edit revisions, dirty chunk invalidation and incremental greedy remeshing. `plugin_render3d::OffscreenVoxelRenderer` consumes those updates and uploads/releases chunk GPU meshes; the native ECS `Render3dPlugin` path also releases voxel meshes removed from the extracted scene. STUDS' GPU backend uses the offscreen mesh renderer bridge, while the CPU raycaster remains a reference/fallback and legacy raycaster source is retained pending manual runtime checks. See `docs/DYNAMIC_VOXEL_WORLD_MIGRATION.md`.
- `WinitRunner` must create the `softbuffer` CPU presenter only after `WindowFrame`
  contains a complete non-empty image. `PlatformPlugin` registers the resource for every
  app, including GPU-only apps; resource presence alone must not create a CPU surface for
  an HWND already used by `wgpu`, because competing native surfaces can crash on shutdown.
- The editor has no text input and is compiled only with its `editor` feature
  in debug builds. The glyph table limits available UI characters.
- Local 2D/voxel transforms and 3D quaternion transforms share the same entity hierarchy; their global transforms are recomputed each `PostUpdate` (dirty tracking is not implemented). `plugin_render3d` consumes `GlobalTransform3D`.
- `examples/scene_hierarchy_3d` demonstrates this `Transform3D` parent-child composition in the GPU mesh renderer and reports runtime diagnostics for global transform propagation.
- STUDS uses `engine_app::App`, `WinitRunner`, and the platform/scene/rendering/audio/game
  plugins. Live gameplay uses `MazeResource` and `RunState` resources plus `Player`, `Enemy`,
  and `PagePickup` entities, advanced by dedicated schedule systems. `Game` remains a
  compatibility/domain facade rather than a runtime resource. Its GPU route now invokes
  `plugin_render3d::OffscreenVoxelRenderer`, with pixels read back through `WindowFrame`;
  `--renderer cpu` remains the legacy software reference/fallback. STUDS owns no engine
  remeshing or GPU-resource policy. The old raycaster isn't removed until manual gameplay
  verification passes; remaining legacy-tree cleanup is tracked in [`docs/PHASE9.md`](docs/PHASE9.md).
- `examples/horror_slice` is a native-surface visual acceptance example. Its `room.ron` describes one concrete room, door/key placements, a lantern save-point prop, monster spawn/path data, and AABB colliders. The player can move with WASD and look with the mouse; `plugin_physics::slide_capsule_xz` provides reusable static-AABB capsule sliding, while the example owns its first-person input/camera system. Door/key interaction, skeletal animation, positional audio, and save logic remain later phases. Bundled GLBs contain the downloaded Poly Haven CC0 maps; separately marked procedural placeholders remain available as fallbacks.
- The neutral renderer fixture, the CPU golden test and the CPU/GPU parity test
  live in `plugins/rendering/tests/` (`docs/PHASE9.md` step 2).
- `examples/voxel_mesh_showcase` demonstrates dynamic block edits, chunk-boundary invalidation and GPU mesh replacement on the native mesh pipeline. Dynamic editing is provided by `ChunkedVoxelWorld::set_block` and `rebuild_dirty_chunks`; STUDS exercises GPU upload/update/eviction through `OffscreenVoxelRenderer`.

See `docs/SHIMS.md` for transitional imports, `docs/DECISIONS.md` for
choices made where the refactor plan left behavior unspecified, and
`docs/LEGACY_ARCHITECTURE.md` for renderer verification results.


## 3D Rendering

- The mesh renderer uses `GpuWindowPlugin` for direct native wgpu-surface presentation. The GPU window builds a `RenderGraph` chain for surface clearing, registered `FramePass` instances, and final presentation ordering. `Render3dPlugin` extracts camera, global mesh/model/voxel transforms, and sunlight into `ExtractedScene3D` during `PostUpdate`; its frame pass consumes that snapshot during GPU submission.
- `plugin_render3d` currently includes a first Cook-Torrance/GGX metallic-roughness shader, a directional shadow-depth pass, a `Rgba16Float` HDR target and Reinhard tone mapping. It samples glTF base-color, packed metallic-roughness, normal, occlusion and emissive textures, and supports fog, bloom and 3x3 PCF shadow filtering. Frustum culling applies to the color pass while off-screen models remain eligible for shadow casting. Cascaded shadows, alpha modes, AO, IBL and GPU culling remain unimplemented.
- `plugin_voxel::greedy_mesh_chunk` creates material-grouped surface meshes with cross-chunk face culling and converts Z-up voxel coordinates to Y-up mesh coordinates. `examples/voxel_mesh_showcase` proves this path on the native renderer; STUDS GPU mode now uses `plugin_render3d::OffscreenVoxelRenderer`; its CPU raycaster remains a reference/fallback.
