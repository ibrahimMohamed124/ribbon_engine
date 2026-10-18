# Phase 9: retire the legacy `engine/` tree

Goal: `games/studs` runs on the ECS engine (`crates/` + `plugins/`), and
`engine/` (the `voxelworks-*` crates) and the shims in `docs/SHIMS.md` are deleted.

This phase is **not started**. The engine-side preparation (dependency leaks,
CI checks, policy enforcement) is done; what remains is the game migration and
the GPU backend move.

## Why it is not a rename

`games/studs` is written against the legacy immediate-mode `App`/`Ctx`/`Renderer`
traits. The new engine is an ECS with plugins and a staged schedule. Moving the
game means rewriting its update loop as systems and plugins, not changing imports.

Current game usage of legacy crates (counted from `use` paths in `games/studs`):

| Legacy crate | References | Replaced by |
| --- | --- | --- |
| `voxelworks-core` | ~32 | `engine_math`, `plugin_voxel`, `plugin_physics`, `engine_platform::input`, `plugin_audio` |
| `voxelworks-scene` | ~18 | `plugin_rendering` scene/material/style types |
| `voxelworks-render-cpu` | ~15 | `plugin_rendering` (`cpu` feature) |
| `voxelworks-api` | ~13 | `engine_render` frame contracts |
| `voxelworks-platform` | ~10 | `engine_platform` + `App`/`Plugin` setup |

## The blocker: the GPU backend

The only complete GPU voxel renderer, and its CPU/GPU parity test, are in
`voxelworks-render-gpu` (about 1,650 lines). `engine_render` has the wgpu
context and presenter, and `plugin_rendering` records a warning when its `gpu`
backend is selected. Deleting `engine/` before this is ported would remove GPU
rendering from the game. Port it first.

## Steps

Each step is its own PR and must leave CI green. Do not remove `games/studs`
from the build while migrating.

- [ ] **1. Port the GPU voxel backend** into `plugin_rendering` (feature `gpu`),
  using `engine_render::gpu`. Move `world.wgsl`, `ui.wgsl`, `present.wgsl` and
  `tests/parity.rs`. Success criterion: the three fixture frame hashes in
  `LEGACY_ARCHITECTURE.md` (`c0aa91941785e603`, `6666e2333fc5196c`,
  `3260fd9f7e087f2e`) are reproduced on CPU, and GPU mean error stays at or below
  the thresholds in the parity test.
- [ ] **2. Move `voxelworks-testkit`** (neutral fixture room) next to the plugin
  that owns the renderer, as a dev-only crate or a `tests/common` module. Keep
  it out of production dependencies (the allowlist already forbids it).
- [ ] **3. Switch scene/api imports** in `games/studs` to `plugin_rendering` /
  `engine_render` paths (`RenderStyle`, `MaterialTable`, `Camera`, `DrawList`,
  `Framebuffer`, `Effect`). Mostly mechanical.
- [ ] **4. Switch core imports** (`math`, `voxel`, `physics`, `input`, `audio`) to
  `engine_math`, `plugin_voxel`, `plugin_physics`, `engine_platform::input`,
  `plugin_audio`. Delete the matching re-exports from `voxelworks-core` as
  each one is no longer used.
- [ ] **5. Replace the runner.** `voxelworks_platform::run_with_mode` and
  `RendererMode` become an `App` assembled from `engine_platform` and the
  rendering plugin. This is where the immediate-mode `App` trait goes away.
- [ ] **6. Convert the game to a plugin.** Port `player`, `enemy`, `level`, `game`
  and the settings/input handling to systems in the correct stages. Keep the
  golden tests (`games/studs/tests/*`) unchanged; they are the safety net.
- [ ] **7. Delete `engine/`.** Move `engine/examples/minimal` to `examples/`
  (rewritten on the ECS engine) or drop it. Remove the `[allowed.legacy]`
  section from `tools/allowed_deps.toml`, `docs/SHIMS.md`, the `voxelworks-*`
  references in `.github/workflows/ci.yml`, and `docs/LEGACY_ARCHITECTURE.md`.
- [ ] **8. Naming pass.** With the legacy crates gone, decide whether
  `plugin_rendering` should be split (data types vs CPU raycaster vs ECS bridge)
  and renamed; its name is currently too close to `engine_render` and
  `plugin_render3d`.

## Done when

```sh
cargo tree -i voxelworks-core        # error: package not found
test ! -d engine
python tools/check_deps.py           # passes with no [allowed.legacy] section
cargo test --workspace && cargo test --workspace --no-default-features
```

and the STUDS golden tests pass with unchanged expected values.
