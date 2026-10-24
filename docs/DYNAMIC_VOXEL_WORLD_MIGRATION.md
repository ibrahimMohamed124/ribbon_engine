# Dynamic Voxel World Integration & Legacy Raycaster Migration

## Goal and boundary

This phase routes STUDS' GPU mode through the engine-owned `ribbon_plugin_voxel` chunk API and `ribbon_plugin_render3d` mesh renderer. STUDS remains a game/sample used to exercise the engine. Gameplay policy stays in `games/studs`; reusable voxel storage, remeshing, GPU uploads, GPU resource eviction and rendering stay in engine crates/plugins.

The CPU raycaster remains selectable as the reference/fallback. The old GPU raycaster implementation has now been removed from `ribbon_plugin_rendering` together with its package `gpu` feature, shader files, ECS route and CPU/GPU parity test. The generic `RenderingPlugin` remains available as a CPU-only legacy-scene renderer. New 3D GPU applications use the mesh renderer in `ribbon_plugin_render3d`.

## Engine-side implementation

### Mutable/chunked voxel storage (`plugins/voxel`)

- `VoxelWorld::set` edits an in-bounds cell and reports the prior block id.
- `ChunkedVoxelWorld` wraps that storage with chunk dimensions, a monotonically increasing edit revision, a retained CPU mesh cache and a dirty-chunk set.
- `set_block` treats same-value writes as no-ops, rejects out-of-bounds coordinates, dirties the owner chunk, and dirties a face-adjacent neighbor when the edit lies on a chunk face.
- `rebuild_dirty_chunks` calls the existing greedy mesher only for dirty chunks and returns updates for both non-empty and now-empty chunks. Empty updates are meaningful: GPU consumers must evict the previous uploaded geometry.
- Greedy meshing samples the full world across chunk boundaries. Shared faces between non-empty voxels are culled; outer world boundaries remain visible. Grid Z-up positions/normals are converted into the renderer's Y-up convention.

### GPU mesh lifecycle (`plugins/render3d`)

`OffscreenVoxelRenderer` is a reusable engine API. It accepts a `ChunkedVoxelWorld`, a camera, a block-id color palette, lighting and optional cube instances. It then:

1. drains dirty chunk updates;
2. releases superseded mesh IDs and uploads only new non-empty chunk meshes;
3. removes GPU buffers when a chunk becomes empty and reuses released mesh slots;
4. renders chunks and additional instances with the common depth/PBR/shadow pipeline to an HDR target;
5. tone maps and explicitly converts to sRGB for an `Rgba8Unorm` offscreen target;
6. returns the result using `ribbon_render::Framebuffer` for the existing window-frame contract.

The normal ECS `Render3dPlugin` path also evicts uploaded voxel mesh buffers when their extracted instances disappear, rather than retaining every historical mesh forever.

### STUDS integration (`games/studs`)

- The generated maze is held in `ChunkedVoxelWorld` with 8x8xfull-height chunks. The engine-only `voxel_mesh_showcase` also exercises dynamic removal/restoration of a floor voxel at a chunk corner on the native ECS surface renderer, with only the owner and face-neighbor chunks rebuilt.
- `--renderer gpu` uses `ribbon_plugin_render3d::OffscreenVoxelRenderer`; it no longer routes through the legacy `ribbon_plugin_rendering::gpu::voxel::GpuRenderer` raycaster.
- The game supplies only its block IDs, linear RGB palette, camera values, flashlight-derived scene lighting, enemy/page visual instances and game UI/overlay draw lists.
- `ribbon_plugin_rendering` owns the shared world-overlay/brightness/UI compositing used by the CPU renderer and the GPU-backed game route.
- The game CPU renderer remains the reference/fallback implementation. No production path in `ribbon_plugin_rendering` uses the retired GPU raycaster. Target-machine manual smoke checks were reported passing before this API retirement; the new source-guard and offscreen mesh lifecycle tests are included in this change but need to be run on the target Rust toolchain.

## Acceptance criteria

| Requirement | Implementation/evidence | Remaining verification |
|---|---|---|
| Load a voxel world | `ChunkedVoxelWorld::new(VoxelWorld, chunk_size, empty_block)`; STUDS wraps its generated maze in this type | Covered by current code and workspace test report; stress streaming is future work |
| Build chunk meshes | `rebuild_dirty_chunks` uses greedy chunk meshing and retains CPU snapshots | The pre-cleanup workspace suite was reported passing; rerun the matrix after this feature/bootstrap cleanup |
| Edit one block and update affected chunks | `set_block` tracks revisions/dirty chunks; interior edits dirty one chunk, face edits dirty the owner and adjacent chunk | Continue extending engine-level regression tests with new editing semantics |
| Handle chunk boundaries | Mesher reads neighboring world cells; tests cover cross-boundary interface culling and edits on all six positive/negative chunk faces | Runtime smoke checks reported passing; randomized chunk dimensions remain useful future coverage |
| Render through new GPU renderer | STUDS GPU backend uses `ribbon_plugin_render3d::OffscreenVoxelRenderer` and the HDR/PBR mesh pipeline; an offscreen integration test exercises mesh uploads, replacement, eviction, empty frames and repeated slot reuse | Target Windows smoke checks reported passing before this API removal; the new GPU integration test still needs target-side execution |
| Retain game stability during migration | CPU fallback remains; the old GPU raycaster is not enabled by STUDS; game controls/simulation remain game-owned | Full gameplay regression testing remains useful for future game changes |
| Preserve engine/game separation | `games/studs/README.md` maps reusable engine vs game responsibilities | Review any future feature against that boundary |

## Validation script

Run from the repository root:

```powershell
cargo fmt --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo test --locked --workspace --no-default-features
python tools/check_deps.py
```

The GitHub Actions workflow runs the cross-platform build/test and dependency-policy matrix. These local commands do not include the manual GPU gameplay review below.

## Manual runtime smoke checks

The following target-machine checks have now been run and reported passing:

```powershell
cargo run --release -p graphics_showcase
cargo run --release -p voxel_mesh_showcase
cargo run --release -p studs --no-default-features -- --renderer cpu
cargo run --release -p studs -- --renderer gpu
```

The maintainer reports that all four ran correctly, including the STUDS CPU/GPU routes. Those manual checks preceded this final source/API removal and do not replace rerunning the full validation matrix after applying the current overlay. The retirement boundary, removed APIs, replacement tests and remaining limitations are recorded in `docs/LEGACY_RAYCASTER_RETIREMENT.md`.

## Scope deliberately not claimed

This path still presents through an offscreen framebuffer/readback because STUDS uses the backend-neutral `WindowFrame` presentation contract. The native `ribbon_plugin_render3d` path now has a reusable spotlight component with a perspective shadow map, PCF filtering, up to eight point lights, map-based normal/AO/emissive shading, height/distance fog and a compact bloom pass; see `examples/horror_slice` for the separate native-surface acceptance scene. This generic renderer feature is not yet parity with the legacy STUDS raycaster's game-specific flashlight effects. Streaming-world loads, asynchronous mesh jobs, LOD and occlusion culling remain future work.
