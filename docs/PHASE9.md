# Phase 9: retire the legacy `engine/` tree

Goal: `games/studs` runs on the ECS engine (`crates/` + `plugins/`), and
`engine/` (the `voxelworks-*` crates) and the shims in `docs/SHIMS.md` are deleted.

Status: steps 1-6 are complete. Step 5 replaces the STUDS executable's legacy
immediate-mode runner with the ECS app and plugins. Step 6 moves live gameplay state
into `MazeResource` and `RunState` resources plus `Player`, `Enemy`, and `PagePickup`
entities, with player movement, pickup collection, enemy behavior, and run resolution
in separate schedule systems. STUDS can select CPU or GPU rendering; the GPU path uses
the existing offscreen voxel renderer and `WindowFrame` readback bridge. Steps 7-8
remain open because compatibility examples and other legacy packages still reference
shim crates.

## Why it is not a rename

The ECS engine is built around `engine_app::App`, plugins, and a staged schedule.
The STUDS binary now uses that app and no longer calls
`voxelworks_platform::run_with_mode` or implements the immediate-mode `App`/`Ctx`
traits. The migration remains incremental: `Game` is retained as a compatibility/domain facade
and new-run generator so existing API consumers and rendering fixtures need not change.
The live runtime does not store `Game`; it installs the generated maze and run state as
resources and player/enemy/pages as entities, advanced by dedicated systems. Settings,
menu state, buffered input, and renderer state remain application/plugin resources. The GPU
route is offscreen voxel raycasting plus readback, not native-surface or mesh rendering.

Current direct legacy dependency use in `games/studs`:

| Legacy crate | Current role | Migration target |
| --- | --- | --- |
| `voxelworks-core` | No direct STUDS dependency; retained transitively by legacy shim crates | `engine_math`, `plugin_voxel`, `plugin_physics`, `engine_platform::input`, `plugin_audio` |
| `voxelworks-scene` | No direct STUDS dependency; current source uses `plugin_rendering` types | `plugin_rendering` scene/material/style types |
| `voxelworks-render-cpu` | Dev-dependency for unchanged renderer/golden integration coverage and `realistic_bench` | `plugin_rendering` (`cpu` feature) |
| `voxelworks-api` | No direct STUDS dependency or runtime use; legacy compatibility packages still use it | `engine_render` frame contracts and `engine_app` |
| `voxelworks-platform` | Dev-dependency for existing audio decoding coverage; no runtime runner use | `engine_platform` + `App`/`Plugin` setup |

## The former blocker: the GPU backend

The GPU voxel renderer (about 1,650 lines) used to exist only in
`voxelworks-render-gpu`, so deleting `engine/` would have removed GPU rendering
from the game. It has been moved to `plugin_rendering::gpu::voxel` (step 1).
The ECS rendering bridge now supports this moved renderer when `RenderBackend::Gpu`
is selected. It renders offscreen, reads the result into `Framebuffer`, and submits
the pixels through `WindowFrame` for the platform runner. This completes the current
ECS bridge without changing the dependency direction; native wgpu-surface presentation
and replacement by the mesh rasterizer remain separate renderer roadmap work.

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
  `voxelworks-scene` and depends on `plugin_rendering` with the `cpu` feature
  and on `engine_render`; the old game-level renderer mode was retired with the
  runner replacement and has since been replaced by `--renderer cpu|gpu|auto` on
  the ECS executable. The allowlist entry follows. At the time of step 3,
  `voxelworks-api` remained for the immediate-mode contract; step 5 removed that
  contract from the executable path, while compatibility users still require the
  shim crates until step 7.
- [x] **4. Switch core imports** (`math`, `voxel`, `physics`, `input`, `audio`) to
  `engine_math`, `plugin_voxel`, `plugin_physics`, `engine_platform::input`,
  `plugin_audio`. Done for `games/studs`: it no longer depends on
  `voxelworks-core` and names the new crates directly (`plugin_audio` with default
  features off, as before). Only imports changed, so no golden value moved.
  The `audio` and `physics` re-exports are deleted from `voxelworks-core` along
  with their dependencies. `math`, `voxel`, `input` and `time` stay because the
  legacy `api`, `platform` and `minimal` crates still use them; they go with those
  crates in step 7.
- [x] **5. Replace the runner.** `games/studs/src/main.rs` now builds an
  `engine_app::App` and registers `PlatformPlugin`, `ScenePlugin`,
  `RenderingPlugin`, `AudioPlugin`, and `StudsPlugin`. `voxelworks_platform::run_with_mode`,
  `RendererMode`, and the immediate-mode `App`/`Ctx` implementation are removed
  from the executable path. `engine_platform` presents a neutral `WindowFrame`
  through its softbuffer-backed winit runner; the game submits the selected CPU or
  offscreen GPU voxel renderer output from `Stage::Render`. The old platform and
  render-CPU shims are retained only as dev-dependencies for compatibility tests/examples.
- [x] **6. Convert gameplay to ECS-owned state and systems.** `MazeResource` and
  `RunState` own world/run-level data; `Player`, `Enemy`, and each `PagePickup` are
  independent entities. `update_player`, `collect_pages`, `update_enemy`, and
  `resolve_run` run in `FixedUpdate`. UI/input, terminal-state persistence, audio,
  title updates, and render submission stay in purpose-specific schedule systems.
  `Game` remains as a compatibility/domain façade and new-run generator, not a
  runtime resource. The same plugin accepts CPU or GPU backend selection.
- [x] **6b. Connect the GPU backend to the ECS runtime.** Both the generic renderer
  bridge and STUDS renderer can use `plugin_rendering::gpu::voxel::GpuRenderer`.
  GPU frames are still rendered offscreen and copied through `WindowFrame`; this
  is not native-surface GPU presentation and does not make the old voxel raymarcher
  the newer mesh renderer. `plugin_render3d` integration remains in the renderer plan.
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
