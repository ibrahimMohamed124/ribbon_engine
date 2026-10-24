# Phase 9: retire the legacy `engine/` tree

> **Historical migration record.** This phase is complete. The old `engine/` directory,
> all `voxelworks-*` packages, and active shim dependency edges are removed from the current workspace.
> The step-by-step notes below preserve the state at the time each migration step ran; descriptions
> of a temporary shim or legacy package are historical, not current package/dependency guidance.

Status: steps 1-8 are complete. Steps 1-6 moved the renderer, fixtures, STUDS runtime, and
gameplay ownership onto the current ECS/plugin architecture. Step 7 removed the retired compatibility
workspace after migrating the final STUDS tests/benchmark and dropping the old minimal runner example.
Step 8 recorded a naming decision: keep `ribbon_render`, `ribbon_plugin_rendering`, and `ribbon_plugin_render3d` as
distinct packages with explicit role documentation; do not rename/split packages without an API-focused
change proposal and dependency migration plan. STUDS can select CPU or GPU rendering; the GPU path
uses the offscreen voxel renderer and `WindowFrame` readback bridge.

## Why it is not a rename

The ECS engine is built around `ribbon_app::App`, plugins, and a staged schedule.
The STUDS binary now uses that app and no longer calls
`voxelworks_platform::run_with_mode` or implements the immediate-mode `App`/`Ctx`
traits. The compatibility-tree migration is complete, while gameplay decomposition remains incremental: `Game`
is retained as a domain facade and new-run generator so existing API consumers and rendering fixtures need not change.
The live runtime does not store `Game`; it installs the generated maze and run state as
resources and player/enemy/pages as entities, advanced by dedicated systems. Settings,
menu state, buffered input, and renderer state remain application/plugin resources. The GPU
route is offscreen voxel raycasting plus readback, not native-surface or mesh rendering.

The following records the **pre-cleanup migration snapshot** and the destination of each former package. None of these `voxelworks-*` packages remains an active workspace member:

| Former package | Former responsibility | Current destination |
| --- | --- | --- |
| `voxelworks-core` | Shared math/voxel/input/time facade; audio and physics exports had been removed earlier | `ribbon_math`, `ribbon_plugin_voxel`, `ribbon_platform`, `ribbon_plugin_audio`, `ribbon_plugin_physics` |
| `voxelworks-scene` | Renderer-neutral scene/material/style data | `ribbon_plugin_rendering` scene/material/style types |
| `voxelworks-render-cpu` | CPU voxel raycasting, rasterization, and reference renderer | `ribbon_plugin_rendering::cpu` (`cpu` feature) |
| `voxelworks-api` | Immediate-mode app/frame/renderer contracts | `ribbon_app` and `ribbon_render` contracts |
| `voxelworks-platform` | Legacy window runner and audio decode helper | `ribbon_platform` window runner and `ribbon_plugin_audio::rodio_backend` |

## The former blocker: the GPU backend

The GPU voxel renderer (about 1,650 lines) used to exist only in
`voxelworks-render-gpu`, so deleting `engine/` would have removed GPU rendering
from the game. It has been moved to `ribbon_plugin_rendering::gpu::voxel` (step 1).
The ECS rendering bridge now supports this moved renderer when `RenderBackend::Gpu`
is selected. It renders offscreen, reads the result into `Framebuffer`, and submits
the pixels through `WindowFrame` for the platform runner. This completes the current
ECS bridge without changing the dependency direction; native wgpu-surface presentation
and replacement by the mesh rasterizer remain separate renderer roadmap work.

## Steps

The migration plan is complete. The checklist below records the final state of each step.

- [x] **1. Port the GPU voxel backend** into `ribbon_plugin_rendering` (feature `gpu`).
  Done: the renderer, presenter, WGSL shaders and inline tests now live in
  `ribbon_plugin_rendering::gpu::voxel` / `plugins/rendering/shaders/voxel/`. It
  implements the shared `ribbon_render::Renderer<MaterialTable>` contract, so
  the legacy `Renderer` trait still applies through its blanket impl.
  At this migration step `voxelworks-render-gpu` was temporarily a re-export shim; it was retired with the other legacy packages in step 7. Two deliberate choices: the
  code is moved, not rewritten (it keeps its own wgpu device instead of
  `ribbon_render::gpu::GpuContext`), because `RENDERER_PLAN.md` freezes this
  raymarcher and replaces it in phase 7; and the parity test stayed in the shim
  crate until step 2 moved the fixture it needs. Verified identical to
  before the move: parity mean error `0.0000 / 0.4051 / 0.7043` (max `0 / 1 / 2`,
  Mesa lavapipe) and headless `voxelworks-minimal` checksums (`gpu`
  `8f1af7f2e5f0202b`, `cpu` `d3a66c50b652d12f`).
- [x] **2. Move `voxelworks-testkit`** (neutral fixture room) next to the plugin
  that owns the renderer, as a dev-only module. Done: the fixture now lives in
  `plugins/rendering/tests/common/mod.rs`, written against `ribbon_math`,
  `ribbon_render`, `ribbon_plugin_voxel` and `ribbon_plugin_rendering` types, so it is part of
  that crate's test tree and cannot be a production dependency. The CPU fixture
  golden (`tests/fixture_golden.rs`, feature `cpu`) and the CPU/GPU parity test
  (`tests/parity.rs`, features `cpu` + `gpu`) moved with it and share one
  `FixtureShader`/`render_cpu` helper instead of two copies. The expected hashes
  and parity thresholds are unchanged. `engine/crates/testkit`, its workspace
  member, lockfile entry, allowlist entry and CI references are gone, and the
  `render-cpu`/`render-gpu` shims no longer have dev-dependencies or tests. The
  Linux CI step now runs `cargo test -p ribbon_plugin_rendering --test parity`.
- [x] **3. Switch scene/api imports** in `games/studs` to `ribbon_plugin_rendering` /
  `ribbon_render` paths. Done: every `voxelworks_scene::` path (`RenderStyle`,
  `MaterialTable`, `Material`, `Camera`, `LightingParams`, `Scene`, `entities`,
  `texture`, ...) now points at `ribbon_plugin_rendering`, and `Effect`, `DrawList` and
  `Framebuffer` come from `ribbon_render`, in `src/`, `examples/` and `tests/`.
  The re-exports are the same types, so only imports changed: no test
  expectation or golden value was touched. `games/studs` drops
  `voxelworks-scene` and depends on `ribbon_plugin_rendering` with the `cpu` feature
  and on `ribbon_render`; the old game-level renderer mode was retired with the
  runner replacement and has since been replaced by `--renderer cpu|gpu|auto` on
  the ECS executable. The allowlist entry follows. At the time of step 3,
  `voxelworks-api` remained for the immediate-mode contract; step 5 removed that
  contract from the executable path, while compatibility users still require the
  shim crates until step 7.
- [x] **4. Switch core imports** (`math`, `voxel`, `physics`, `input`, `audio`) to
  `ribbon_math`, `ribbon_plugin_voxel`, `ribbon_plugin_physics`, `ribbon_platform::input`,
  `ribbon_plugin_audio`. Done for `games/studs`: it no longer depends on
  `voxelworks-core` and names the new crates directly (`ribbon_plugin_audio` with default
  features off, as before). Only imports changed, so no golden value moved.
  The `audio` and `physics` re-exports are deleted from `voxelworks-core` along
  with their dependencies. `math`, `voxel`, `input` and `time` stayed temporarily because the
  legacy `api`, `platform` and `minimal` crates still used them at this stage; those
  crates were removed in step 7.
- [x] **5. Replace the runner.** `games/studs/src/main.rs` now builds an
  `ribbon_app::App` and registers `PlatformPlugin`, `ScenePlugin`,
  `RenderingPlugin`, `AudioPlugin`, and `StudsPlugin`. `voxelworks_platform::run_with_mode`,
  `RendererMode`, and the immediate-mode `App`/`Ctx` implementation are removed
  from the executable path. `ribbon_platform` presents a neutral `WindowFrame`
  through its softbuffer-backed winit runner; the game submits the selected CPU or
  offscreen GPU voxel renderer output from `Stage::Render`. At that migration stage, the old platform and
  render-CPU shims were retained as dev-dependencies; step 7 subsequently removed these final edges.
- [x] **6. Convert gameplay to ECS-owned state and systems.** `MazeResource` and
  `RunState` own world/run-level data; `Player`, `Enemy`, and each `PagePickup` are
  independent entities. `update_player`, `collect_pages`, `update_enemy`, and
  `resolve_run` run in `FixedUpdate`. UI/input, terminal-state persistence, audio,
  title updates, and render submission stay in purpose-specific schedule systems.
  `Game` remains as a compatibility/domain façade and new-run generator, not a
  runtime resource. The same plugin accepts CPU or GPU backend selection.
- [x] **6b. Connect the GPU backend to the ECS runtime.** Both the generic renderer
  bridge and STUDS renderer can use `ribbon_plugin_rendering::gpu::voxel::GpuRenderer`.
  GPU frames are still rendered offscreen and copied through `WindowFrame`; this
  is not native-surface GPU presentation and does not make the old voxel raymarcher
  the newer mesh renderer. `ribbon_plugin_render3d` integration remains in the renderer plan.
- [x] **7. Delete `engine/`.** The old `engine/examples/minimal` immediate-mode sample was retired
  rather than rewritten, avoiding a second long-lived application/window/rendering architecture.
  Workspace members, active CI references and dependency allowlist entries were removed. Final STUDS
  test/benchmark imports now reference `ribbon_plugin_rendering::cpu`; audio decode coverage uses
  `ribbon_plugin_audio::rodio_backend` under the existing optional `audio` feature.
- [x] **8. Naming pass.** Keep `ribbon_render` for shared frame/draw/render-graph contracts,
  `ribbon_plugin_rendering` for voxel scene/material data plus CPU/GPU voxel backends and the ECS bridge,
  and `ribbon_plugin_render3d` for native-surface mesh rendering. A split/rename is intentionally deferred
  until there is a concrete public API boundary proposal; a mechanical rename now would add churn
  without separating material ownership and backend responsibilities.

## Done when

Expected closeout checks (run in CI or a Rust-enabled developer environment):

```sh
test ! -d engine
! cargo tree --workspace | grep -E 'voxelworks-(core|scene|api|render-cpu|render-gpu|platform|minimal)'
python tools/check_deps.py
cargo test --workspace
cargo test --workspace --no-default-features
```

The migration did not alter or re-bless any STUDS golden constants. These commands are documented
as required verification; this source-overlay environment does not contain Cargo, so their post-change
results are not claimed here.
