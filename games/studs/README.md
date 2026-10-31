# STUDS

STUDS is a first-person voxel horror game built on Ribbon Engine. Explore a block maze, collect six pages, and keep the creature in sight: it moves when you look away.

## Features

- A plugin-based ECS application with independent player/enemy/page entities and scheduled gameplay systems.
- Selectable CPU reference rendering or the engine's chunk-mesh GPU renderer (HDR/PBR mesh rasterization with offscreen readback into the neutral `WindowFrame` contract).
- Creature navigation, line-of-sight pursuit, and search behavior.
- Sprint stamina, flashlight, configurable controls, saved settings, and spatial audio.
- Classic grayscale, Moody, and photo-textured Realistic styles.
- CC0 baked animated monster sprite with a box fallback.

## Run

```sh
cargo run --release -p studs
cargo run --release -p studs -- --renderer cpu
cargo run --release -p studs -- --renderer gpu
cargo run --locked -p studs --example ecs_preview
```

Run the workspace tests from the repository root with `cargo test --workspace`.

## Controls

| Action | Default keys |
| --- | --- |
| Move | W / A / S / D |
| Look | Arrow keys or mouse |
| Jump | E |
| Sprint | Left or right Shift |
| Flashlight | F |
| Restart | R |
| Toggle mouse look | Tab or M |
| Pause / Back | Esc |
| Quit | Q |

Settings and bindings are stored in `settings.cfg` under the operating system's user configuration directory. The game starts with a main menu. Difficulty changes when the creature wakes and its chase speed; looking at it slows it on every setting.

## Runtime assets

Processed assets and the on-demand source manifest live in `games/studs/assets/`. Run `python games/studs/tools/fetch_assets.py --check` to verify tracked files and hashes. Raw source downloads are cached under the ignored `games/studs/assets/source/` directory.

`ecs_preview` starts directly in-game with a deterministic seed and uses the same ECS app and platform runner as the game binary. Use `--renderer cpu` or `--renderer gpu` to choose the game backend; `--renderer auto` prefers the mesh GPU backend when the feature is enabled and falls back to CPU if GPU initialization or rendering fails. The GPU backend uses `studs_voxel_gpu::OffscreenVoxelRenderer`: the STUDS-owned voxel adapter consumes dirty chunk updates, greedy-meshes changed chunks, uploads/releases GPU mesh buffers and renders the maze through the shared PBR/HDR mesh pipeline. The result is read back into the neutral `WindowFrame` contract, so STUDS still does not use native-surface presentation. The CPU raycaster remains the intentional reference/fallback path. STUDS' `gpu` feature enables `ribbon_plugin_render3d`; GPU/CPU mode selection is defined in STUDS and is no longer coupled to the retired `studs_cpu_renderer::ecs::RenderBackend` API. Because the CPU raycaster and the right-handed mesh camera use opposite horizontal screen-right conventions, STUDS adapts horizontal look and A/D intent only while the mesh GPU backend is active: moving the mouse right and pressing D turn/strafe right on screen, while the CPU fallback retains its established behavior.

## Engine boundary (important)

STUDS is a **consumer and stress-test of Ribbon Engine**, not part of the engine implementation. The only logic in `games/studs` is game/domain logic: maze generation and block meanings (`AIR`, `FLOOR`, `WALL`, `CEIL`), player movement and collision policy, creature behavior, page pickups, difficulty, lighting/palette choices for the game's art direction, controls, menus, audio and progression.

Reusable engine functionality must stay in engine crates/plugins. In particular:

- `games/studs/crates/studs_voxel` owns mutable voxel storage, chunk coordinates, world revisions, dirty-chunk tracking, boundary-neighbor invalidation and greedy chunk meshing.
- `plugins/render3d` owns general mesh upload, GPU mesh-slot reuse and eviction, depth/shadows, PBR/HDR rendering, and tone mapping. `games/studs/crates/studs_voxel_gpu` owns the game-specific offscreen voxel adapter.
- `games/studs/crates/studs_cpu_renderer` owns backend-neutral game-frame overlay composition and continues to provide the CPU reference renderer.

Do not move chunk remeshing, GPU resource management, general-purpose scene rendering, or other engine services into `games/studs`. If STUDS exposes a missing reusable capability, implement that capability in the appropriate engine crate/plugin and let the game provide only its own gameplay data and presentation choices.

## Dynamic voxel migration status

The maze is stored in `ChunkedVoxelWorld`. Call `chunked_world_mut().set_block(x, y, z, block)` when future gameplay mechanics change blocks; `studs_voxel::ChunkedVoxelWorld` invalidates the owning chunk plus face-adjacent chunks when the edit touches a boundary. On the next GPU render, only dirty chunks are remeshed/uploaded, replaced GPU buffers are released, and empty chunk meshes are evicted. Chunk meshing samples the complete voxel grid, so faces shared across chunk boundaries are culled consistently.

The previously reported Windows manual smoke checks predate the latest architecture restructuring and must be rerun on the target machine. The CPU raycaster remains the supported reference/fallback. The legacy GPU voxel raycaster and generic ECS GPU route were removed; mesh-based GPU applications use `ribbon_plugin_render3d`, while the STUDS-only offscreen bridge lives in `games/studs/crates/studs_voxel_gpu`. See `../../docs/history/LEGACY_RAYCASTER_RETIREMENT.md` for the intentional public API break and migration path.


## Engine/game ownership boundary

STUDS-specific voxel storage, chunk meshing, CPU raycasting, and the offscreen GPU bridge live under `games/studs/crates/` and `games/studs/src/`. They are game implementation details, not engine APIs. Engine crates provide general-purpose mesh rendering, scene, assets, input, profiling, and physics contracts only.
