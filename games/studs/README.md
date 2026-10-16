# STUDS

STUDS is a first-person voxel horror game built on Voxelworks Engine. Explore a block maze, collect six pages, and keep the creature in sight: it moves when you look away.

## Features

- GPU-first voxel rendering, with a selectable CPU reference renderer.
- Creature navigation, line-of-sight pursuit, and search behavior.
- Sprint stamina, flashlight, configurable controls, saved settings, and spatial audio.
- Classic grayscale, Moody, and photo-textured Realistic styles.
- CC0 baked animated monster sprite with a box fallback.

## Run

```sh
cargo run --release -p studs
cargo run --locked -p studs --example gpu_preview -- gpu
cargo run --locked -p studs --example gpu_preview -- cpu
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

For a deterministic GPU scene, run `cargo run --locked -p studs --example gpu_preview -- gpu` (pass `cpu` to compare the reference). CPU screenshots and the Realistic benchmark are available through examples in their respective crates.
