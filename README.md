# Ribbon Engine

Ribbon Engine is a Rust game engine built around a small ECS core, a staged scheduler
with access-declared parallel execution and a deterministic serial fallback, and
optional plugins. The legacy voxel-scene renderer retains a CPU raycaster for software/reference
rendering, while the primary 3D GPU path is the engine-owned indexed mesh renderer. The [`studs`](games/studs/)
game demonstrates an application built on it.

> **Status:** `studs` now starts through the ECS `ribbon_app::App` with
> `ribbon_platform`, `ribbon_scene`, `ribbon_plugin_rendering`, and `ribbon_plugin_audio`.
> The old immediate-mode runner and `App`/`Ctx` traits are no longer on the
> game executable's runtime path. The live maze, run progression, player, enemy,
> and page pickups are now ECS resources/entities with scheduled simulation systems.
> STUDS supports `--renderer cpu|gpu|auto`. Its GPU backend uses the engine-owned
> `ribbon_plugin_render3d::OffscreenVoxelRenderer`, which consumes dynamically remeshed voxel
> chunks and draws through the PBR/HDR mesh pipeline. The CPU raycaster remains the reference and
> fallback. The Windows runtime smoke checks for `graphics_showcase`, `voxel_mesh_showcase`, and
> STUDS in CPU/GPU modes have been reported passing before this retirement change. Those manual
> runs do not verify the API-removal diff; rerun the validation matrix after applying it. The legacy
> GPU voxel raycaster and its `ribbon_plugin_rendering` backend-selection route have now been removed;
> `RenderingPlugin` is CPU-only, and new GPU applications use `ribbon_plugin_render3d`. STUDS keeps a
> game-local CPU/GPU selector and its GPU path uses `OffscreenVoxelRenderer`. See
> [docs/DYNAMIC_VOXEL_WORLD_MIGRATION.md](docs/DYNAMIC_VOXEL_WORLD_MIGRATION.md),
> [docs/LEGACY_RAYCASTER_RETIREMENT.md](docs/LEGACY_RAYCASTER_RETIREMENT.md), and
> [docs/PHASE9.md](docs/PHASE9.md).

## Layout

| Path | Contents |
| --- | --- |
| `crates/` | Ribbon Engine core (`ribbon_*`): math, ECS, events, schedule, app, plugins, platform, assets, scene, render, reflection, commands, editor |
| `plugins/` | Optional features (`ribbon_plugin_*`): voxel, physics, rendering, render3d, gltf, animation, audio, editor tools |
| `examples/` | Engine-only sample apps: `hello_ecs`, `voxel_room`, `cube_spin`, `model_viewer`, `gpu_window`, `editor_demo`, `graphics_showcase`, `scene_hierarchy_3d`, `voxel_mesh_showcase`, `horror_slice` |
| `games/studs/` | A game composed from the ECS app, engine runtime, and plugins |
| `docs/`, `tools/` | Architecture decisions, migration records, dependency policy, and tooling |

Rust packages in `crates/` use the `ribbon_*` namespace, while optional plugin packages use
`ribbon_plugin_*`. Their source directories follow the same core names; plugin directories remain
feature-named under `plugins/`. Runnable examples retain concise names such as `hello_ecs` and
`graphics_showcase`. The namespace change is an API/package rename: downstream Cargo dependencies and
Rust imports must use the Ribbon names.

`graphics_showcase` displays its controls in-window and supports mouse orbit/zoom; `voxel_mesh_showcase` supports keyboard block editing and shows chunk/GPU mesh diagnostics in its in-window panel.

Read [ARCHITECTURE.md](ARCHITECTURE.md) for the dependency graph, frame order and
known limits, [docs/RENDERING_API_CONTRACTS.md](docs/RENDERING_API_CONTRACTS.md) for the public
CPU/GPU material contract, [docs/RELEASE_CANDIDATE_CHECKLIST.md](docs/RELEASE_CANDIDATE_CHECKLIST.md)
for release acceptance, [CHANGELOG.md](CHANGELOG.md) for user-facing changes, and
[docs/SCALABILITY_ROADMAP.md](docs/SCALABILITY_ROADMAP.md),
[docs/GPU_RESOURCE_LIFETIME_PROFILING.md](docs/GPU_RESOURCE_LIFETIME_PROFILING.md), and
[docs/PERFORMANCE_BASELINES.md](docs/PERFORMANCE_BASELINES.md) for scalability, GPU profiling, and
repeatable performance capture, and [docs/SURFACE_PRESENT_MODE_COMPARISON.md](docs/SURFACE_PRESENT_MODE_COMPARISON.md)
for a controlled FIFO-vs-Mailbox multi-asset capture; use `tools/run_present_mode_comparison.ps1` to collect three alternating-order repeats per mode. For opt-in large-scene and asset-lifecycle stress workflows, see [docs/ASSET_LIFETIME_LARGE_SCENE_STRESS.md](docs/ASSET_LIFETIME_LARGE_SCENE_STRESS.md) and `tools/run_large_scene_stress.ps1`. Read [CONTRIBUTING.md](CONTRIBUTING.md) before adding code. The cleanup
summary and verification
boundary are in [docs/ARCHITECTURE_REFACTOR_CLOSEOUT.md](docs/ARCHITECTURE_REFACTOR_CLOSEOUT.md).

## Running

```sh
cargo run                                   # the STUDS game (ECS app + platform runner)
cargo run -p hello_ecs                      # ECS and schedule only
cargo run -p voxel_room                     # ECS voxel room (add -- --headless --renderer cpu)
cargo run -p model_viewer                   # glTF model on the wgpu mesh renderer
cargo run -p graphics_showcase              # PBR / HDR / directional-shadow graphics showcase
cargo run -p scene_hierarchy_3d             # visible 3D parent/child transform example
cargo run -p voxel_mesh_showcase             # dynamic greedy voxel chunks on the native 3D renderer
cargo run -p horror_slice                    # native-surface single-room horror visual slice
```

`cargo run` runs the game because `default-members` is `games/studs`; use
`--workspace` (or `-p <name>`) for everything else, for example `cargo test --workspace`.

Audio and GPU rendering are optional Cargo features for STUDS. The defaults enable
both; `--no-default-features` selects the CPU renderer and disables Rodio output.
Choose a backend with `cargo run -p studs -- --renderer gpu` or `--renderer cpu`;
`auto` currently prefers GPU when that feature is compiled in and falls back to CPU
if GPU initialization fails. The STUDS GPU path uses `ribbon_plugin_render3d::OffscreenVoxelRenderer`
for GPU mesh rasterization and copies the tone-mapped framebuffer through `WindowFrame`.
The CPU path remains the deterministic reference/fallback; native-surface voxel rendering
is a separate future milestone.

## Horror Slice visual acceptance example

`cargo run -p horror_slice` opens the native-surface single-room horror scene. The room is RON-authored and uses a flashlight spotlight with a perspective shadow map, concrete/floor/rusted-metal PBR maps, a locked-door prop, a key, a small lantern/save-point prop and a monster silhouette. The bundled GLBs contain the nine downloaded Poly Haven CC0 maps; deterministic procedural maps remain separately marked `PLACEHOLDER` fallbacks. Provenance and regeneration steps are documented in [`examples/horror_slice/README.md`](examples/horror_slice/README.md). The example supports first-person WASD movement, mouse look and static AABB wall/prop collision via `ribbon_plugin_physics`; Shift sprints, Escape releases the cursor and left-click recaptures it. It remains a rendering/movement slice: door/key interaction, skeletal animation, positional audio and save/load are not implemented yet. Interactive visual acceptance still requires review on the target Windows GPU.

## Dependency rules

`tools/allowed_deps.toml` lists every allowed dependency edge per package.
`python tools/check_deps.py` checks normal, build, development, and optional dependency edges in CI.
Engine crates never depend on games or examples. The retired `voxelworks-*` workspace has been
removed; the checker rejects reintroducing legacy workspace package names.


## Window and launcher identity

The Ribbon Engine icon source and setup for titlebar, taskbar, and Windows executable icons are documented in [`docs/WINDOW_ICON.md`](docs/WINDOW_ICON.md).
