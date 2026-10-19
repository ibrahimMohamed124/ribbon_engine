# Voxelworks Engine

Voxelworks is a Rust game engine built around a small ECS core, a staged
scheduler and optional plugins, with a voxel raycasting renderer (CPU and GPU)
and a wgpu mesh renderer. The [`studs`](games/studs/) game demonstrates an
application built on it.

> **Status:** `studs` now starts through the ECS `engine_app::App` with
> `engine_platform`, `engine_scene`, `plugin_rendering`, and `plugin_audio`.
> The old immediate-mode runner and `App`/`Ctx` traits are no longer on the
> game executable's runtime path. The live maze, run progression, player, enemy,
> and page pickups are now ECS resources/entities with scheduled simulation systems.
> STUDS supports `--renderer cpu|gpu|auto`; GPU rendering currently uses the existing
> offscreen voxel raycaster and copies its frame to the window. Legacy shim crates
> remain for compatibility examples until the remaining Phase 9 cleanup. See
> [docs/PHASE9.md](docs/PHASE9.md).

## Layout

| Path | Contents |
| --- | --- |
| `crates/` | Engine core (`engine_*`): math, ECS, events, schedule, app, plugins, platform, assets, scene, render, reflection, commands, editor |
| `plugins/` | Optional features (`plugin_*`): voxel, physics, rendering, render3d, gltf, animation, audio, editor tools |
| `examples/` | Engine-only sample apps: `hello_ecs`, `voxel_room`, `cube_spin`, `model_viewer`, `gpu_window`, `editor_demo`, `graphics_showcase`, `scene_hierarchy_3d`, `voxel_mesh_showcase` |
| `games/studs/` | The game, now launched through the ECS app and plugin runner; some gameplay migration and compatibility cleanup remain. |
| `engine/` | **Legacy** `voxelworks-*` crates, to be deleted |
| `docs/`, `tools/` | Plans, decisions, dependency policy and checker |

Read [ARCHITECTURE.md](ARCHITECTURE.md) for the dependency graph, frame order and
known limits, and [CONTRIBUTING.md](CONTRIBUTING.md) before adding code.

## Running

```sh
cargo run                                   # the STUDS game (ECS app + platform runner)
cargo run -p hello_ecs                      # ECS and schedule only
cargo run -p voxel_room                     # ECS voxel room (add -- --headless --renderer cpu)
cargo run -p model_viewer                   # glTF model on the wgpu mesh renderer
cargo run -p graphics_showcase              # PBR / HDR / directional-shadow graphics showcase
cargo run -p scene_hierarchy_3d             # visible 3D parent/child transform example
cargo run -p voxel_mesh_showcase             # greedy voxel chunks on the native 3D renderer
cargo run -p voxelworks-minimal -- --headless --renderer cpu   # legacy engine-only example
```

`cargo run` runs the game because `default-members` is `games/studs`; use
`--workspace` (or `-p <name>`) for everything else, for example `cargo test --workspace`.

Audio and GPU rendering are optional Cargo features for STUDS. The defaults enable
both; `--no-default-features` selects the CPU renderer and disables Rodio output.
Choose a backend with `cargo run -p studs -- --renderer gpu` or `--renderer cpu`;
`auto` currently prefers GPU when that feature is compiled in and falls back to CPU
if GPU initialization fails. The STUDS GPU path is the existing offscreen voxel
raycaster with readback through `WindowFrame`, not the newer `plugin_render3d`
mesh-rasterization pipeline.

## Dependency rules

`tools/allowed_deps.toml` lists every allowed dependency edge per package.
`python tools/check_deps.py` checks it and runs in CI. Engine crates never
depend on games, and nothing outside `engine/` and `games/studs` depends on the
legacy `voxelworks-*` crates.
