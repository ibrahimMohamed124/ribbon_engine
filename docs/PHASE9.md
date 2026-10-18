# Phase 9: retire the legacy `engine/` tree

Goal: `games/studs` runs on the ECS engine (`crates/` + `plugins/`), and
`engine/` (the `voxelworks-*` crates) and the shims in `docs/SHIMS.md` are deleted.

Status: steps 1 (GPU backend move), 2 (testkit move) and 3 (scene/api imports) are
done. The engine-side preparation (dependency leaks, CI checks, policy
enforcement) is done; steps 4-8 remain.

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

## The former blocker: the GPU backend

The GPU voxel renderer (about 1,650 lines) used to exist only in
`voxelworks-render-gpu`, so deleting `engine/` would have removed GPU rendering
from the game. It has been moved to `plugin_rendering::gpu::voxel` (step 1).
Note that the ECS rendering bridge in `plugin_rendering::ecs` still renders on
the CPU and only warns when the GPU backend is selected; wiring it to the moved
renderer is optional because phase 7 of `RENDERER_PLAN.md` replaces this path.

## Steps

Each step is its own PR and must leave CI green. Do not remove `games/studs`
from the build while migrating.

- [x] **1. Port the GPU voxel backend** into `plugin_rendering` (feature `gpu`).
  Done: the renderer, presenter, WGSL shaders and inline tests now live in
  `plugin_rendering::gpu::voxel` / `plugins/rendering/shaders/voxel/`. It
  implements the shared `engine_render::Renderer<MaterialTable>` contract, so
  the legacy `Renderer` trait still applies through its blanket impl.
  `voxelworks-render-gpu` is now a re-export shim. Two deliberate choices: the
  code is moved, not rewritten (it keeps its own wgpu device instead of
  `engine_render::gpu::GpuContext`), because `RENDERER_PLAN.md` freezes this
  raymarcher and replaces it in phase 7; and the parity test stayed in the shim
  crate until step 2 moved the fixture it needs. Verified identical to
  before the move: parity mean error `0.0000 / 0.4051 / 0.7043` (max `0 / 1 / 2`,
  Mesa lavapipe) and headless `voxelworks-minimal` checksums (`gpu`
  `8f1af7f2e5f0202b`, `cpu` `d3a66c50b652d12f`).
- [x] **2. Move `voxelworks-testkit`** (neutral fixture room) next to the plugin
  that owns the renderer, as a dev-only module. Done: the fixture now lives in
  `plugins/rendering/tests/common/mod.rs`, written against `engine_math`,
  `engine_render`, `plugin_voxel` and `plugin_rendering` types, so it is part of
  that crate's test tree and cannot be a production dependency. The CPU fixture
  golden (`tests/fixture_golden.rs`, feature `cpu`) and the CPU/GPU parity test
  (`tests/parity.rs`, features `cpu` + `gpu`) moved with it and share one
  `FixtureShader`/`render_cpu` helper instead of two copies. The expected hashes
  and parity thresholds are unchanged. `engine/crates/testkit`, its workspace
  member, lockfile entry, allowlist entry and CI references are gone, and the
  `render-cpu`/`render-gpu` shims no longer have dev-dependencies or tests. The
  Linux CI step now runs `cargo test -p plugin_rendering --test parity`.
- [x] **3. Switch scene/api imports** in `games/studs` to `plugin_rendering` /
  `engine_render` paths. Done: every `voxelworks_scene::` path (`RenderStyle`,
  `MaterialTable`, `Material`, `Camera`, `LightingParams`, `Scene`, `entities`,
  `texture`, ...) now points at `plugin_rendering`, and `Effect`, `DrawList` and
  `Framebuffer` come from `engine_render`, in `src/`, `examples/` and `tests/`.
  The re-exports are the same types, so only imports changed: no test
  expectation or golden value was touched. `games/studs` drops
  `voxelworks-scene` and depends on `plugin_rendering` (feature `cpu` only, so
  the game's `gpu` feature still controls wgpu) and `engine_render`; the
  allowlist entry follows. `voxelworks-api` stays for the immediate-mode
  contract (`App`, `Ctx`, `Flow`, `WindowConfig`, `WindowRequest`) and the
  `FramePlan` alias, which go away with the runner in step 5.
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
