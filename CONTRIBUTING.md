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

## Code style

- Follow standard Rust formatting and keep module boundaries clear.
- Prefer deterministic tests for gameplay and rendering logic.
- Avoid adding dependencies unless they provide a clear benefit.
