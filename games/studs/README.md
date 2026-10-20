# STUDS

STUDS is a first-person voxel horror game built on Voxelworks Engine. Explore a block maze, collect six pages, and keep the creature in sight: it moves when you look away.

## Features

- A plugin-based ECS application with independent player/enemy/page entities and scheduled gameplay systems.
- Selectable CPU reference rendering or the engine's chunk-mesh GPU renderer (HDR/PBR mesh rasterization with offscreen readback into the neutral `WindowFrame` contract).
- Creature navigation, line-of-sight pursuit, and search behavior.
- Sprint stamina, flashlight, configurable controls, saved settings, and spatial audio.
- Classic grayscale, Moody, and photo-textured Realistic styles.
- CC0 baked animated monster sprite on the CPU reference path and a multipart mesh proxy on the current GPU mesh path.

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
| Move forward / backward | W / S (relative to the current view) |
| Strafe left / right | A / D (relative to the current view) |
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

`ecs_preview` starts directly in-game with a deterministic seed and uses the same ECS app and platform runner as the game binary. Use `--renderer cpu` or `--renderer gpu` to choose the game backend; `--renderer auto` prefers the mesh GPU backend when the feature is enabled and falls back to CPU if GPU initialization or rendering fails. The GPU backend now uses `plugin_render3d::OffscreenVoxelRenderer`: the engine consumes dirty chunk updates, greedy-meshes the changed chunks, uploads/releases GPU mesh buffers and renders the maze through the shared PBR/HDR mesh pipeline. STUDS supplies its own block IDs, albedo assets and per-style tints through the generic `VoxelBlockMaterial` API; texture upload, sampling and lifetime remain engine responsibilities. The result is read back into the neutral `WindowFrame` contract, so this migration does not yet change STUDS to native-surface presentation. The CPU raycaster remains a reference/fallback path until visual and gameplay parity have been checked on target hardware. The GPU game route currently uses a game-owned multipart mesh proxy for the creature; the CPU route uses the baked animated sprite atlas, so exact creature-animation parity is not claimed yet. The legacy Z-up camera basis has the opposite horizontal screen handedness from the native right-handed mesh camera, so STUDS mirrors only the successful GPU world framebuffer before compositing its world overlays and HUD. This game-side compatibility adapter keeps mouse/arrow turning and WASD walking/strafe directions consistent without changing the engine's raw mouse deltas or introducing game logic into engine crates.

## Engine boundary (important)

STUDS is a **consumer and stress-test of Voxelworks Engine**, not part of the engine implementation. The only logic in `games/studs` is game/domain logic: maze generation and block meanings (`AIR`, `FLOOR`, `WALL`, `CEIL`), player movement and collision policy, creature behavior, page pickups, difficulty, lighting/palette choices for the game's art direction, controls, menus, audio and progression.

Reusable engine functionality must stay in engine crates/plugins. In particular:

- `plugins/voxel` owns mutable voxel storage, chunk coordinates, world revisions, dirty-chunk tracking, boundary-neighbor invalidation and greedy chunk meshing.
- `plugins/render3d` owns mesh upload, GPU mesh-slot reuse and eviction, cached per-block texture materials, texture sampling, depth/shadows, PBR/HDR rendering, tone mapping and the offscreen voxel renderer API.
- `plugins/rendering` owns backend-neutral game-frame overlay composition and continues to provide the CPU reference renderer.

Do not move chunk remeshing, GPU resource management, general-purpose scene rendering, or other engine services into `games/studs`. If STUDS exposes a missing reusable capability, implement that capability in the appropriate engine crate/plugin and let the game provide only its own gameplay data and presentation choices.

## Dynamic voxel migration status

The maze is stored in `ChunkedVoxelWorld`. Call `chunked_world_mut().set_block(x, y, z, block)` when future gameplay mechanics change blocks; the engine invalidates the owning chunk plus face-adjacent chunks when the edit touches a boundary. On the next GPU render, only dirty chunks are remeshed/uploaded, replaced GPU buffers are released, and empty chunk meshes are evicted. Chunk meshing samples the complete voxel grid, so faces shared across chunk boundaries are culled consistently.

The old raycaster implementations have deliberately **not** been deleted. Keep them until manual GPU gameplay smoke checks have confirmed camera orientation, textured floor/wall/ceiling geometry, light/readability, creature presentation, overlays, page pickup presentation, resizing, level restart and graceful CPU fallback on target Windows hardware.
