# Voxelworks Engine

Voxelworks is a Rust game engine built around a small ECS core, a staged
scheduler and optional plugins, with a voxel raycasting renderer (CPU and GPU)
and a wgpu mesh renderer. The [`studs`](games/studs/) game demonstrates an
application built on it.

> **Status:** the engine was refactored from the original `voxelworks-*` crates
> in `engine/` to the ECS design in `crates/` and `plugins/`. `studs` has not
> been migrated yet, so both trees are in the workspace. See
> [docs/PHASE9.md](docs/PHASE9.md).

## Layout

| Path | Contents |
| --- | --- |
| `crates/` | Engine core (`engine_*`): math, ECS, events, schedule, app, plugins, platform, assets, scene, render, reflection, commands, editor |
| `plugins/` | Optional features (`plugin_*`): voxel, physics, rendering, render3d, gltf, animation, audio, editor tools |
| `examples/` | Engine-only sample apps: `hello_ecs`, `voxel_room`, `cube_spin`, `model_viewer`, `material_grid`, `gpu_window`, `editor_demo` |
| `games/studs/` | The game. Still on the legacy crates. |
| `engine/` | **Legacy** `voxelworks-*` crates, to be deleted |
| `docs/`, `tools/` | Plans, decisions, dependency policy and checker |

Read [ARCHITECTURE.md](ARCHITECTURE.md) for the dependency graph, frame order and
known limits, and [CONTRIBUTING.md](CONTRIBUTING.md) before adding code.

## Running

```sh
cargo run                                   # the STUDS game (the workspace default member)
cargo run -p hello_ecs                      # ECS and schedule only
cargo run -p voxel_room                     # ECS voxel room (add -- --headless --renderer cpu)
cargo run -p model_viewer                   # glTF model on the wgpu mesh renderer
cargo run -p voxelworks-minimal -- --headless --renderer cpu   # legacy engine-only example
```

`cargo run` runs the game because `default-members` is `games/studs`; use
`--workspace` (or `-p <name>`) for everything else, for example `cargo test --workspace`.

GPU and audio are Cargo features. `--no-default-features` builds the CPU-only,
silent variant, which needs no audio system libraries (on Linux, audio needs
`libasound2-dev`).

## Dependency rules

`tools/allowed_deps.toml` lists every allowed dependency edge per package.
`python tools/check_deps.py` checks it and runs in CI. Engine crates never
depend on games, and nothing outside `engine/` and `games/studs` depends on the
legacy `voxelworks-*` crates.
