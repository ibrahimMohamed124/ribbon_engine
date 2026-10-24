# Spotlight Shadow Pipeline Repair

## Why this change

The renderer previously reused the directional shadow pipeline for spotlight depth rendering.
That pipeline culls back faces and carries a bias intended for directional shadowing. A
spotlight placed inside a closed room needs the inward-facing surfaces of the room to be
able to populate its perspective depth map; it also benefits from an independent depth-bias
configuration.

## Changes

- Keep the existing directional shadow pipeline and its state unchanged.
- Add a dedicated spotlight shadow pipeline with back-face culling disabled and the default
  depth bias, then use it only for the spotlight shadow pass.
- Adjust the existing headless corridor fixture's light placement and intensity so the
  floor region behind the blocker produces a measurable signal in its `Rgba8Unorm` target.
- Preserve the existing assertions and their thresholds. No new tests are added.

## Verification

This archive was checked for source consistency and ZIP integrity only. Rust formatting,
Clippy, WGSL validation, and the GPU regression test have not been run in the artifact-building
environment. Validate on the development machine:

```powershell
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p ribbon_plugin_render3d spotlight_only_corridor_writes_a_headless_png -- --nocapture
```

If the GPU test still reports zero changed pixels, inspect the newly generated
`target/spotlight_corridor.png` and attach it with the complete test output. Do not lower or
remove the regression assertion.
