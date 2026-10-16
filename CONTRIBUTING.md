# Contributing

Keep reusable engine code and game-specific code in their respective layers.

- `engine/crates/core`: portable math, voxel, physics, input/time primitives, and audio identifiers. Keep it dependency-free.
- `engine/crates/scene`: renderer-neutral scene, material, texture, and style data.
- `engine/crates/api`: shared app, frame, draw, framebuffer, and renderer contracts.
- `engine/crates/render-cpu`: software raycasting, rasterization, font rendering, and post-processing.
- `engine/crates/render-gpu`: GPU buffers, pipelines, shaders, offscreen rendering, and window presentation.
- `engine/crates/platform`: windowing, input mapping, audio playback, and application lifecycle.
- `engine/examples/minimal`: small examples that must build without depending on the game.
- `games/studs`: game rules, levels, content, UI, and game-owned runtime assets/tools.

Do not add game vocabulary or game dependencies to engine crates. Put renderer-neutral behavior in shared data when both backends need to interpret it. If a capability remains CPU-specific, document it and cover the limitation with a named test.

Before submitting, run `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, the same test suite with `--no-default-features`, `python games/studs/tools/fetch_assets.py --check`, and both CPU/GPU headless minimal example runs where a GPU adapter is available. Keep the existing game golden values unchanged.
