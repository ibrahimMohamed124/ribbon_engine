# Contributing

Keep reusable engine code and game-specific code in their respective layers.
[ARCHITECTURE.md](ARCHITECTURE.md) describes the layers; the rules below are the short form.

## Where code goes

- `crates/engine_*`: engine core. Foundations (`engine_math`, `engine_ecs`,
  `engine_events`, `engine_schedule`) have no platform or rendering knowledge.
- `plugins/*` (package `plugin_*`): optional behavior installed into an `App`. A
  plugin declares its dependencies on other plugins, never owns the app loop and
  never contains game rules.
- `examples/*`: engine-only demos. They must build without `games/` or `engine/`.
- `games/*`: game rules, levels, content, UI and game-owned assets.
- `engine/`: **legacy**, frozen. Only bug fixes needed to keep `games/studs`
  building. New code goes in `crates/` or `plugins/`.

## Rules

- Do not add game vocabulary or game dependencies to engine crates or plugins;
  CI greps for it.
- Add or change a dependency edge only together with `tools/allowed_deps.toml`,
  and run `python tools/check_deps.py`.
- Optional backends stay optional. `wgpu` and `rodio` are reachable only through
  `gpu`/`audio` features; depend on a workspace crate with
  `default-features = false` unless you need its defaults.
- Put renderer-neutral behavior in shared data when both backends need to interpret
  it. If a capability remains CPU-specific, document it and cover the limitation
  with a named test.
- Record decisions that the plans leave open in `docs/DECISIONS.md`.

## Before submitting

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test --workspace --no-default-features
python tools/check_deps.py
python games/studs/tools/fetch_assets.py --check
```

Also run the CPU and GPU headless examples where an adapter is available. Keep the
existing game golden values unchanged.
