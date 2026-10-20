# Dynamic Voxel World Integration & Legacy Raycaster Migration

## Goal and boundary

This phase routes STUDS' GPU mode through the engine-owned `plugin_voxel` chunk API and `plugin_render3d` mesh renderer. STUDS remains a game/sample used to exercise the engine. Gameplay policy stays in `games/studs`; reusable voxel storage, remeshing, GPU uploads, GPU resource eviction and rendering stay in engine crates/plugins.

The previous CPU raycaster remains selectable as a reference/fallback. The old GPU raycaster code in `plugin_rendering` is intentionally retained for rollback and comparative coverage; this phase changes the STUDS GPU route rather than deleting compatibility code prematurely.

## Engine-side implementation

### Mutable/chunked voxel storage (`plugins/voxel`)

- `VoxelWorld::set` edits an in-bounds cell and reports the prior block id.
- `ChunkedVoxelWorld` wraps that storage with chunk dimensions, a monotonically increasing edit revision, a retained CPU mesh cache and a dirty-chunk set.
- `set_block` treats same-value writes as no-ops, rejects out-of-bounds coordinates, dirties the owner chunk, and dirties a face-adjacent neighbor when the edit lies on a chunk face.
- `rebuild_dirty_chunks` calls the existing greedy mesher only for dirty chunks and returns updates for both non-empty and now-empty chunks. Empty updates are meaningful: GPU consumers must evict the previous uploaded geometry.
- Greedy meshing samples the full world across chunk boundaries. Shared faces between non-empty voxels are culled; outer world boundaries remain visible. Grid Z-up positions/normals are converted into the renderer's Y-up convention.

### GPU mesh lifecycle (`plugins/render3d`)

`OffscreenVoxelRenderer` is a reusable engine API. It accepts a `ChunkedVoxelWorld`, a camera, a block-id color palette or texture-backed `VoxelBlockMaterial` descriptors, lighting and optional cube instances. The material descriptor carries a neutral linear tint, metallic/roughness factors and optional sRGB albedo texture data. The renderer caches GPU materials per block ID, uploads textures once, and leaves asset selection/tint policy to the application. It then:

1. drains dirty chunk updates;
2. releases superseded mesh IDs and uploads only new non-empty chunk meshes;
3. removes GPU buffers when a chunk becomes empty and reuses released mesh slots;
4. renders chunks and additional instances with the common depth/PBR/shadow pipeline to an HDR target;
5. tone maps and explicitly converts to sRGB for an `Rgba8Unorm` offscreen target;
6. returns the result using `engine_render::Framebuffer` for the existing window-frame contract.

The normal ECS `Render3dPlugin` path also evicts uploaded voxel mesh buffers when their extracted instances disappear, rather than retaining every historical mesh forever.

### STUDS integration (`games/studs`)

- The generated maze is held in `ChunkedVoxelWorld` with 8x8xfull-height chunks. The engine-only `voxel_mesh_showcase` also exercises dynamic removal/restoration of a floor voxel at a chunk corner on the native ECS surface renderer, with only the owner and face-neighbor chunks rebuilt.
- `--renderer gpu` uses `plugin_render3d::OffscreenVoxelRenderer`; it no longer routes through the legacy `plugin_rendering::gpu::voxel::GpuRenderer` raycaster.
- The game supplies only its block IDs, albedo assets, linear RGB style tints, camera values, flashlight-derived scene lighting, enemy/page visual instances and game UI/overlay draw lists. The materials are loaded in `games/studs`; texture upload/sampling and cache lifetime are engine services. A visible scene uses the real STUDS albedo textures instead of flat palette colors, and the game uses a multipart cube-mesh creature proxy rather than a single large cuboid. The CPU sprite atlas is not yet used by the new GPU mesh path, so exact creature animation parity remains outstanding.
- `plugin_rendering` owns the shared world-overlay/brightness/UI compositing used by the CPU renderer and the GPU-backed game route.
- The game CPU renderer remains the reference/fallback implementation. The legacy renderer source is retained elsewhere in the workspace until the manual GPU runtime checklist below has been completed.

## Acceptance criteria

| Requirement | Implementation/evidence | Remaining verification |
|---|---|---|
| Load a voxel world | `ChunkedVoxelWorld::new(VoxelWorld, chunk_size, empty_block)`; STUDS wraps its generated maze in this type | Run STUDS on the target Windows machine |
| Build chunk meshes | `rebuild_dirty_chunks` uses greedy chunk meshing and retains CPU snapshots | `cargo test -p plugin_voxel` |
| Edit one block and update affected chunks | `set_block` tracks revisions/dirty chunks; interior edits dirty one chunk, face edits dirty the owner and adjacent chunk | Full workspace test matrix |
| Handle chunk boundaries | Mesher reads neighboring world cells; tests cover cross-boundary interface culling and boundary invalidation | Visual check for seams in a live scene |
| Render through new GPU renderer | STUDS GPU backend uses `plugin_render3d::OffscreenVoxelRenderer`, cached per-block albedo textures and the HDR/PBR mesh pipeline | Manual GPU run, because CI unit tests do not guarantee adapter/window presentation or visual legibility |
| Retain game stability during migration | CPU fallback and the legacy implementation remain available; the game's controls/simulation modules were not migrated into the engine | Compare `--renderer gpu` and `--renderer cpu` in a full play session |
| Preserve engine/game separation | `games/studs/README.md` states the boundary and this file maps responsibilities | Review any future feature against that boundary |

## Validation script

Run from the repository root:

```powershell
powershell -ExecutionPolicy Bypass -File .\finish-dynamic-voxel-phase.ps1
```

The script formats Rust code and runs dependency policy, `cargo check`, Clippy with warnings denied, example build, workspace tests and a no-default-features build/test pass. The no-default-features matrix and final STUDS release-GPU build run with one Cargo job, incremental compilation disabled and debug info disabled to lower peak memory on Windows. If rustc reports OS error 1455 / a paging-file allocation failure, close memory-heavy applications and ensure the Windows paging file is system-managed or large enough before retrying. It does not create commits or push changes.

## Manual runtime smoke checks

After the automated matrix passes, run on the target Windows GPU:

```powershell
cargo run --release -p studs -- --renderer gpu
cargo run --release -p studs -- --renderer cpu
```

Check that the maze floor, walls and ceiling have correct orientation and material detail, the flashlight keeps nearby surfaces readable while shadows remain intentional, the camera moves/looks in the expected direction, the creature is not rendered as an oversized placeholder block, uncollected pages appear and collected pages disappear, overlays and menus remain legible, resize/restart work, and a GPU initialization/render error falls back to CPU. Compare GPU and CPU from the same seed and similar camera position so differences are meaningful. Do not remove the legacy raycaster until the GPU path has passed these checks on the intended target hardware.

## Scope deliberately not claimed

This path still presents through an offscreen framebuffer/readback because STUDS uses the backend-neutral `WindowFrame` presentation contract. Native-surface presentation for STUDS, a textured/animated billboard path for the GPU creature, spot-light/flashlight cone rendering, rebuilding based on streaming world loads, asynchronous mesh jobs, LOD and occlusion culling remain separate follow-on improvements. The existing directional light and shadows are consumed by the mesh renderer, but are not a replacement for all of the game's old raycaster-specific flashlight effects.
## STUDS camera handedness compatibility

STUDS retains its legacy Z-up gameplay/camera convention while the native mesh renderer uses a right-handed Y-up camera. The GPU route mirrors only the rendered world framebuffer horizontally before screen-space overlays are composed. This preserves the game's existing screen-left/right semantics for mouse and arrow turning and for WASD movement without inverting raw platform input or moving game controls into engine crates. The CPU reference renderer is unchanged. A runtime smoke check should verify mouse-right, arrow-right, W/S movement, A/D strafing, HUD orientation, and GPU/CPU behavior.
