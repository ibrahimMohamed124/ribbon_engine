# STUDS

STUDS is a first-person voxel horror game built on Voxelworks Engine. Explore a block maze, collect six pages, and keep the creature in sight: it moves when you look away.

## Features

- A plugin-based ECS application and desktop runner, currently using the CPU scene renderer for the game path.
- Creature navigation, line-of-sight pursuit, and search behavior.
- Sprint stamina, flashlight, configurable controls, saved settings, and spatial audio.
- Classic grayscale, Moody, and photo-textured Realistic styles.
- CC0 baked animated monster sprite with a box fallback.

## Run

```sh
cargo run --release -p studs
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

`ecs_preview` starts directly in-game with a deterministic seed and uses the same ECS app, `engine_platform` runner, and CPU rendering path as the game binary. The former `gpu_preview` example and its `RendererMode` argument were tied to the legacy immediate-mode runner and have been retired. GPU voxel rendering remains available through its existing renderer crate, but it is not selected by the migrated STUDS runtime. CPU screenshots and the Realistic benchmark are available through examples in their respective crates.
