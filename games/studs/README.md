# STUDS

STUDS is a first-person voxel horror game built on Voxelworks Engine. Explore a block maze, collect six pages, and keep the creature in sight: it moves when you look away.

## Features

- A plugin-based ECS application with independent player/enemy/page entities and scheduled gameplay systems.
- Selectable CPU or GPU voxel rendering through the ECS runtime (GPU currently uses offscreen rendering and readback).
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

`ecs_preview` starts directly in-game with a deterministic seed and uses the same ECS app and platform runner as the game binary. Use `--renderer cpu` or `--renderer gpu` to choose the game backend; `--renderer auto` currently prefers GPU when compiled with that feature and falls back to CPU if GPU initialization fails. The GPU path is the existing offscreen voxel raycaster and copies pixels back through the neutral `WindowFrame` contract; it is not native-surface presentation and does not yet use `plugin_render3d` mesh rasterization. CPU screenshots and the Realistic benchmark remain available through examples in their respective crates.
