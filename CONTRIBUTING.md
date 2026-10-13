# Contributing

Thanks for considering a contribution to STUDS.

## Before you start

- For a substantial change, open an issue first to discuss the approach.
- Keep pull requests focused and explain the player-facing impact.
- Do not include generated build output, local settings, or assets you do not have permission to share.

## Development

1. Install the stable Rust toolchain.
2. Create a focused branch for your change.
3. Make the smallest coherent change and add or update tests where behavior changes.
4. Before opening a pull request, run:

   ```sh
   cargo fmt --all -- --check
   cargo clippy --locked --all-targets -- -D warnings
   cargo test --locked
   ```

5. In the pull request, describe the change, how you verified it, and include screenshots for visual changes when possible.

## Renderer visuals and goldens

- The engine's default `RenderStyle` is Classic; game-facing style selection is persisted as `visual_style = classic|moody`.
- Generate deterministic visual samples with `cargo run --locked -p blocks_engine --example screenshot` (output: `target/shots/`).
- Compare Classic and Moody frame cost with `cargo run --release --locked -p blocks_engine --example render_bench -- 30`.
- If an intentional rendering change updates a golden, inspect the screenshot output first, then run `cargo test -p studs --test golden -- --nocapture`, update only the affected expected hash, and rerun the full locked workspace tests. Keep the legacy Classic golden pinned unless its output change is intentional and documented.

## Code style

- Follow standard Rust formatting and keep module boundaries clear.
- Prefer deterministic tests for gameplay and rendering logic.
- Avoid adding dependencies unless they provide a clear benefit.
