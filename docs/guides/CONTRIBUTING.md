# Contributing

Keep reusable engine code and game-specific code in their respective layers.
[architecture guide](../architecture/ARCHITECTURE.md) describes the layers; the rules below are the short form.

## Where code goes

- `crates/*`: Ribbon Engine core (package names retain the `ribbon_*` namespace). Foundations (`ribbon_math`, `ribbon_ecs`,
  `ribbon_events`, `ribbon_schedule`) have no platform or rendering knowledge.
- `plugins/*` (package `ribbon_plugin_*`): optional behavior installed into an `App`. A
  plugin declares its dependencies on other plugins, never owns the app loop and
  never contains game rules.
- `examples/*`: engine-only demos. They may depend on `crates/` and reusable `plugins/`, never on `games/` or another example.
- `games/*`: game rules, levels, content, UI and game-owned assets.

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
- Record new architectural decisions as numbered ADRs under `docs/decisions/`; the legacy log is preserved in `docs/decisions/0000-legacy-decision-log.md`.

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

## Architecture and repository hygiene

- Add new engine runtime/features only under `crates/` or `plugins/`; add demos under `examples/` and game-specific logic under `games/`. The retired `engine/` compatibility tree must not be recreated.
- Keep dependencies flowing toward foundations. The enforced layers are `crates/` → `crates/`; `plugins/` → `crates/` + `plugins/`; and `examples/`/`games/` → `crates/` + `plugins/`. No workspace package may depend on a game. These boundaries are enforced by `tools/check_deps.py` in addition to the explicit allowlist; update `tools/allowed_deps.toml` alongside intentional direct dependency changes.
- Do not place self-hosted GitHub Actions runner installations, `_work` checkouts, runner logs, `target/`, or generated ZIP deliveries in source control. The local `.github-actions-runner/` folder is ignored deliberately.
- Keep runtime/platform code separate from OS-specific PE-resource compilation: window icon selection belongs in `ribbon_platform`; Windows executable resource embedding belongs in `build_support/windows/`.
- For scalability changes, measure before widening concurrent execution. `ribbon_schedule` offers `SystemConfig::parallel` with typed component/resource access declarations, conflict-free batches, and a serial fallback. Declarations are a caller contract; structural world changes and conditional systems stay exclusive. Compare parallel results against serial execution and profile thread-startup costs before relying on performance gains. Asset loading/retention and transform dirty tracking likewise need explicit ownership/invalidation contracts. Record design decisions in `docs/decisions/` and update this architecture guide.
