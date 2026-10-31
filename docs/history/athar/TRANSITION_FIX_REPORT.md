# Ribbon Engine / ATHAR — room-to-village transition fix report

Date: 2026-10-29

## Summary

The original transition was not staged: `games/athar/src/scripts/gameplay.rs::enter_village` removed room entities and spawned the whole village in one `Update`, while `plugins/render3d` resolved ModelInstance handles lazily and uploaded no more than two distinct model generations per render frame. The renderer skips placements whose CPU asset isn't resident yet or whose GPU cache entry is still being created. That combination can expose an empty/partial scene if a gameplay transition happens before all assets have made it through that pipeline.

The source review confirms this mechanism can explain progressive appearance. It does **not**, by itself, prove why any literally black pixels were displayed on a particular machine. `crates/render/src/gpu_window.rs::WindowClearNode` clears the swap-chain view every frame, independent of scene draws. A persistent literal black frame could still require runtime surface/render-graph/GPU diagnostics. No visual runtime capture was available during this work.

The patch starts village preparation while the room is active, requires CPU residency plus validated GPU cache residency before commit, and keeps the room/doorway state intact during preparation or failure. It preserves the existing upload budget of `MAX_MODEL_GPU_UPLOADS_PER_FRAME = 2`.

## Current runtime flow after the patch

1. `games/athar/src/main.rs` collects the unique room and village model paths, queues them through `AssetServer::request_ref`, retains strong `AssetRef<ModelAsset>` leases in `GameModelLeases`, and submits the seven unique village handles to `ModelWarmup` during startup.
2. CPU loading and decoding stay within the existing AssetServer worker/request lifecycle. A handle/lease alone is not readiness. `gameplay.rs::update_village_preparation` checks `AssetServer::state`, resident `server.get(handle)` data, and the renderer's per-generation `ModelWarmupStatus`.
3. `plugins/render3d/src/preload.rs::ModelWarmup` records `WaitingForCpu`, `WaitingForGpu`, `Ready`, and terminal `Failed(reason)` state. It works without visible ECS placements; duplicate requests for the same ID/generation share one warmup entry.
4. The renderer-owned `MeshPass` in `plugins/render3d/src/lib.rs` creates GPU meshes, materials and textures on the renderer's device/queue, through the normal model cache. Visible placements have priority; background warmups share the same two-model upload budget. It keeps cache entries live for both active placements and warmup requests, and polls nested WGPU validation/OOM scopes without blocking a game thread.
5. `Ready` is published only after CPU data is resident, the cache upload was created, both error-scope futures resolve without an error, and the matching cache generation remains resident. Invalid model structure, CPU load errors, and GPU validation/OOM errors become path-specific failures rather than false success.
6. While warming, `player.rs::submit_character_movement` prevents walking out through the doorway. HUD lines expose ready-model progress or the failed asset/reason. An explicit retry is available near the unlocked, open door. No sleep or synchronous asset read is added to the gameplay thread.
7. `gameplay.rs::enter_village` only proceeds from `Room + VillageReady`. It preflights and spawns the destination first; only afterward does it move the player/camera, set outdoor clear color, sun, fog and weather, then tear down the room entities and lights. `CommittingTransition`/`VillageActive` cannot satisfy the commit gate, and the actual placement entities retain the GPU cache immediately after warmup-only retention is released.
8. Rendering still executes the window clear every frame. If no draw list exists, the mesh pass returns after cache reconciliation and releases its transient HDR target; it doesn't leave the prior surface contents as the intended clear behavior.

## Source changes

- `games/athar/src/main.rs`: deduplicated asset requests and startup village warmup; one strong CPU lease is retained per asset path.
- `games/athar/src/scripts/gameplay.rs`: CPU/GPU readiness state handling, visible progress/failure/retry behavior, commit gate, destination-before-teardown ordering, outdoor setup, spawn timing and one-shot transform/physics/rain profiling logs. Retries replace the strong lease for an asset path instead of accumulating duplicate references. Regression coverage rejects repeated commits from `CommittingTransition` and `VillageActive`.
- `games/athar/src/scripts/player.rs`: clamps outward doorway motion while background preparation is incomplete or failed.
- `games/athar/src/scripts/mod.rs`: adds transition state/progress and profiling flags to game state.
- `games/athar/src/scripts/village.rs`: preflight all required model handles before entity creation, preserving all-or-nothing behavior for missing handles; village geometry continues using Rapier collider proxies.
- `plugins/render3d/src/preload.rs`, `plugin.rs`, `lib.rs`: explicit renderer-owned warmup API and status, generation-deduplicated tracking, cache liveness retention, nonblocking GPU scope polling, diagnostics, unchanged two-upload budget and cache-handoff behavior. GPU-backed integration tests exercise no-visible-placement warmup across three unique cache keys, the two-upload-per-frame budget, Ready after error-scope validation, handoff to a real placement, later release, empty-scene clear preservation, and upload-counter reset on early-return frames. They skip if WGPU cannot create an offscreen context, matching existing renderer tests' convention.
- `plugins/render3d/src/model.rs`: validate CPU model structure before allocating GPU resources, preserve the original `upload_model` API, expose a fallible `try_upload_model`, and avoid uploading primitives that are not referenced by placed nodes. Tests cover malformed/packaged models and orphan mesh resource allocation.
- `plugins/render3d/src/extract.rs`: records extraction CPU time only while profile capture is enabled. `plugins/render3d/src/lib.rs` adds a nested opt-in CPU phase for model GPU object creation/submission (not GPU execution), exports the one-shot scene timing into the existing CPU phase diagnostics, and includes a regression check.
- `crates/render/src/profiling/types.rs` and `csv.rs`: append warmup counts, uploads/frame, and one-shot warmup counts and uploads/frame while preserving the 81-field CSV schema; a serialization regression test protects its appended columns.
- `crates/scene/src/hierarchy.rs` and `crates/scene/src/lib.rs`: add a one-shot 3D transform-propagation timer installed by `ScenePlugin`; no clock read is taken during steady-state transform propagation. Its regression test checks auto-disarm after one sample.
- `plugins/physics/src/runtime.rs`: gates reconcile/fixed-step wall timing behind `PhysicsDiagnostics::timing_enabled`; ATHAR arms that timing only for the first post-transition sample and then disables it.
- `plugins/weather/src/lib.rs`: `RainDiagnostics` samples only the synchronous particle-creation batch. It avoids adding a timing read to steady-state particle updates, and an allocation regression test checks batch count/reuse.
- `tools/validate_athar_transition_warmup.py`: source-only integration invariant and packaged-GLB header checker, including CSV schema and profiler wiring. `tools/perf/render_profile_report.py` summarizes the nonzero one-shot transform-propagation timing without averaging it with unrelated zero-valued frames.
- `docs/architecture/ATHAR_TRANSITION_WARMUP.md`, `GPU_ASSET_UPLOAD_PACING.md`, `ASSET_LIFETIME_STREAMING.md`, and `GPU_RESOURCE_LIFETIME_PROFILING.md`: explicit warmup semantics, cache-handoff ownership, upload budget and interpretation of the new profiling fields.

## Performance and rendering scope

No numeric render/frame-time measurements were collected in this environment. Therefore no claim is made that synchronous `spawn_village` (128 model placements + 111 collider entities + 8 point lights in the inspected layout) or the 220-particle initial rain batch is cheap enough for every target. Both are now timed at the transition; physics reconciliation/fixed step is captured for the first post-activation pass. The existing 81-column render profile includes warmup counts and uploads/frame; phase rows separately record one-shot transform-propagation duration and `model GPU resource creation/submission`, scene extraction, model cache reconciliation, model/draw-list work, asset CPU memory estimates and existing asynchronous GPU timing samples.

A staged village/collider activation system and early mesh-frustum culling were intentionally not added without runtime measurements. Such changes could complicate collision readiness, shadow correctness, or resource ownership; use before/after captures first. CPU strong leases remain alive for the session, which is reasonable for this fixed prototype asset set but decoded textures/geometry can use much more RAM than the 24.59 MiB of source GLBs.

Known content limits retained: village and room use primitive proxy collision rather than mesh-exact collision; the monster is a placeholder; key/door interaction targeting remains horizontal proximity-based rather than camera-ray-based. These are not represented as causes of the rendering delay.

## Validation performed here

### Passed

- `python tools/validate_athar_transition_warmup.py` — **36/36 static checks passed**. This checks the source readiness gate, cache retention, upload budget constant, nonblocking polling pattern, profiler CSV shape and packaged model file headers. It does not compile or run Rust code.
- `python -m py_compile tools/validate_athar_transition_warmup.py tools/perf/render_profile_report.py` — passed.
- `python tools/perf/render_profile_report.py --help` — passed; the reporting tool exposes the expected capture/compare/output options.
- Packaged asset check: **12 GLB containers**, total source file size **24.59 MiB**, with valid `glTF` magic bytes and container lengths.
- Overlay archive integrity test (`zipfile.testzip`) — passed. Overlay contains only explicitly listed source/docs/tool files plus the install README; no `Cargo.toml` or `Cargo.lock` changes.

### Blocked in this environment

Every requested Rust command was attempted, but this environment has no `cargo` executable (`bash: cargo: command not found`, exit code 127):

- `cargo fmt --all -- --check`
- `cargo test -p ribbon_plugin_render3d`
- `cargo test -p ribbon_plugin_gltf`
- `cargo test -p ribbon_plugin_physics`
- `cargo test -p athar`
- `cargo test --workspace --no-fail-fast`
- `cargo clippy -p ribbon_plugin_render3d --all-targets -- -D warnings`
- `cargo clippy -p ribbon_plugin_physics --all-targets -- -D warnings`
- `cargo clippy -p athar --all-targets -- -D warnings`

`py -3 tools/check_deps.py` was attempted but the Windows `py` launcher is absent here (exit 127). `python tools/check_deps.py` was attempted and exited 2 because that script invokes `cargo metadata`, which cannot run without Cargo. It has **not** passed in this environment.

The source snapshot had no `.git` directory, so a truthful `git status`/diff could not be produced. The overlay was built from an explicit list of affected files to avoid including unrelated workspace content.

## Required target-side verification

After backing up the matching files, extract `Ribbon_Engine_ATHAR_Transition_Fix_Overlay.zip` at the repository root. On the Windows checkout run:

```powershell
cargo fmt --all -- --check
cargo test -p ribbon_plugin_render3d
cargo test -p ribbon_plugin_gltf
cargo test -p ribbon_plugin_physics
cargo test -p athar
cargo test --workspace --no-fail-fast
cargo clippy -p ribbon_plugin_render3d --all-targets -- -D warnings
cargo clippy -p ribbon_plugin_physics --all-targets -- -D warnings
cargo clippy -p athar --all-targets -- -D warnings
py -3 tools/check_deps.py
python tools/validate_athar_transition_warmup.py
```

Then run ATHAR on Windows with an actual GPU and repeat the key → unlock → wait inside → cross doorway → test ground/building/tree collision and rain sequence. Capture a profile using:

```powershell
cargo run -p athar --release -- --profile-csv target/profiles/athar-transition.csv --profile-warmup 120
py -3 tools/perf/render_profile_report.py target/profiles/athar-transition.csv --output target/profiles/athar-transition-report.md
```

Acceptance signals: all seven village models reach Ready before transition; uploads/frame never exceed two; transition commit occurs once; surface continues clearing while draws are empty; profile captures disclose `spawn_village`, first village physics reconciliation/fixed-step, rain creation, and frame-wall peak/P95/P99. Compare before and after on the same hardware. A visual improvement and its measured latency must only be claimed after that run, which was **not** performed here.
