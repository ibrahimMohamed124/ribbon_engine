# Ribbon Engine — Streaming Gap Completion & Acceptance Report

**Snapshot:** 2026-10-31  
**Baseline:** `Ribbon_Engine(3).zip` plus the complete streaming continuation chain included in this cumulative overlay.

## Executive result

This cumulative continuation now includes: phase-accurate Region lifecycle traces and retained slowest-activation phase data; per-class generic GPU admission budgets and lease-safe eviction; a multi-Region Scene visibility commit barrier; SceneDoc same-handle revision invalidation; a `wgpu::Device` loss notification/recreation path with renderer-pass resource teardown and warmup requeue; categorized CPU asset residency (including glTF geometry/texture/material allocations and RON sprite-animation clips); editor/CSV diagnostics; and stricter stress-harness GPU-pressure acceptance checks.

It still does **not** establish complete production acceptance for every possible renderer resource and hardware configuration. Generic standalone GPU streaming currently covers Mesh, Texture, Sampler, and PBR Material payloads. Renderer-owned shaders and pipelines are rebuilt from their existing embedded/source descriptors when a replacement device is created, but they are not standalone generation-keyed streamed assets with independent leases. The new `.ranim` animation clips are CPU-side sprite animation data; the current renderer does not implement skinned glTF animation or a separate GPU animation buffer lifecycle. GPU memory remains estimated and does not equal a hard physical VRAM cap. The expanded soak/pressure configurations and device-loss recovery have not been exercised on the Windows/GPU host after these source edits.

The Rust toolchain and PowerShell are not installed in the artifact-preparation environment. The latest source edits therefore have **not** been compiled, formatted by rustfmt, run through Rust tests, Clippy, the Windows runner, or actual GPU validation here. Static checks listed below did run successfully.

## 1. Region lifecycle profiling

### Implemented

`plugins/streaming/src/coordinator.rs` now measures the elapsed duration of every state before a transition and accumulates the principal stages:

- Request admission.
- CPU asset loading and dependency readiness.
- GPU warmup.
- Ready wait.
- Physics preparation and collider registration.
- Activation/commit.
- Active, deactivation, Dormant, unloading, and failure residence.

`StreamEvent.elapsed_ns` records time spent in the source state for each transition. In-flight state duration is sampled in `RegionSnapshot`, so a currently stuck Region is visible before it transitions.

Each completed activation tracks last/max request-to-active latency and activation count. The `slowest_activation` snapshot preserves the phase breakdown from the exact attempt that established the lifetime maximum; a subsequent quick activation no longer overwrites the outlier's phase data. The Streaming editor panel shows per-Region stage timings and the latest transition trace. Render Profile CSV appends fields for the slowest Region and its retained phase breakdown. The latest cumulative schema also includes device-recovery, CPU memory-category, and glTF model-cache soft-budget pressure fields (CSV schema now has 152 columns).

### Interpreting the existing >1s spike

The previously supplied real GPU run `20261031-193330` recorded activation latency p50/p95/p99 of **61.819 / 62.679 / 1,078.966 ms**. Its `App Update` peak was **983.464 ms**. This old run did not contain the new per-stage Region trace, so it cannot retrospectively prove whether that outlier came from the first cold asset read/decode, GPU readiness, physics reconciliation, or activation work. The new trace is designed to capture that cause in the next run; it does not claim the old spike has already been eliminated.

## 2. GPU Residency Manager

### Implemented for generic renderer-managed resources

`plugins/render3d/src/gpu_streaming.rs` supports generic renderer-owned Mesh, Texture, Sampler, and PBR Material requests with:

- Stable `(namespace, id, generation)` keys and duplicate request sharing.
- Strong leases for queued, in-flight validation, and ready resources.
- Total estimated-byte admission limit, plus Mesh, Texture, and Material class byte limits and an independent Sampler count limit.
- Budget accounting for queued, Preparing, and Ready entries, so uploads in flight reserve space.
- Priority-aware orphan selection (lowest priority, then oldest sequence) when a new request cannot fit.
- No eviction of live leases or Preparing/error-scope validation work.
- Request rejection with a typed `BudgetExceeded` error when safe capacity cannot be made.
- GPU destruction performed by the render-owning thread; evicted handles are collected and released there.
- Separate pressure-event, eviction, budget-rejection, and last-evicted-class counters exposed to diagnostics, CSV, and the editor.

A pressure event is counted when an admission request exceeds the configured budget, including requests refused because existing resources remain pinned. It is distinct from the eviction and rejection counters.

### Important scope boundary

These class budgets govern the **generic GPU warmup registry**. In addition, `ModelGpuResidencyBudget` provides a configurable soft budget for the renderer's glTF model cache (default 1 GiB; warning/high/critical thresholds default to 75/90/100%). The model cache reports estimated resident bytes, budget, overage, current pressure level, and pressure-level escalation events to `RenderDiagnostics`, Editor, and CSV. This is an observability/pressure budget, not a hard admission cap: it never refuses an active scene's model upload or invalidates a live placement, and the model bytes are a subset estimate of the renderer's mesh/material totals. The generic registry and model cache still do not form one global hard VRAM cap over transient targets, surface allocations, shader/pipeline overhead, driver allocations, or OS graphics allocations. All bytes are estimates, not driver-reported physical VRAM.

## 3. Generic GPU lifecycle and device recreation

### Generic streamed resources

The generic queue has an explicit lifecycle for standalone mesh, RGBA8 texture, sampler, and PBR material payloads: request, validate, priority promotion, queue, render-thread upload, asynchronous validation, Ready/Failed state, lease retention, generation-specific identity, and render-thread release. CPU upload payloads remain retained while a lease is alive so a device-loss event can rebuild the resource. Their retained bytes are reported separately from GPU residency.

### Device-loss recovery path added

- `GpuContext` registers `wgpu::Device::set_device_lost_callback`. The callback records a thread-safe flag and reason only; it does not mutate renderer state from the callback thread.
- `gpu_window_system` observes the signal on the render-owning thread, notifies every render-graph node, drops the old context after pass-owned resources have been released, and attempts to create a replacement adapter/device. Failed recreation is retried at a bounded one-second cadence instead of permanently disabling the window after the first failed replacement attempt.
- `MeshPass::on_device_lost` drops `MeshRenderer`, HDR/post-process targets, tonemapper, transient targets, GPU model/dynamic-mesh caches, and pending validation scopes. `ModelWarmup` entries move from Ready back to WaitingForGpu. Generic GPU resources with live leases are reset to Queued from their retained CPU payloads; unleased entries are discarded.
- `MeshRenderer` and the tonemapper recreate their shader modules, render pipelines, layouts, and device-owned resources through their normal constructors/lazy initialization on the replacement device. `WindowTextOverlayPass` also drops its old pipeline and vertex buffer and recreates them on demand. This covers existing renderer-owned static pipelines, not independently authored shader/pipeline assets.
- Device-loss events, last reported reason, recovery attempts/success/failure counts, and retained generic CPU payload estimates are exposed in diagnostics, editor data, and Render Profile CSV.

The recovery operation currently uses the same `pollster::block_on` construction path as initial native-window device creation, on a rare device-reset path. It is retried rather than busy-looped, but can still stall the render/update call during an attempt. This path has source-level tests for generic warmup requeue, but no actual device-loss injection or hardware recovery test was run here.

### Animation asset lifecycle added

`ribbon_plugin_animation` now provides `SpriteAnimationClip` and `SpriteAnimationClipLoader` for `.ranim` RON files. Clips validate non-empty frame lists, non-empty names, and finite positive FPS; playback is driven by `SpriteClipPlayer` without waiting when an asset is not yet resident. The loader reports its decoded allocation estimate in the Animation CPU-memory category. `AnimationPlugin` installs the loader when `AssetServer` is available. The atlas texture remains an ordinary renderer texture and must be independently leased/warmed. Existing `SpriteAnimator` compatibility remains intact.

### Remaining GPU lifecycle scope

- There is still no general-purpose standalone ShaderModule/RenderPipeline asset registry with generation-keyed descriptor leases. The current shader modules/pipelines are renderer-owned and recreated from embedded WGSL/renderer descriptors after device recreation; this is intentional until a real stable pipeline-layout/descriptor contract can be exposed.
- The current glTF importer/render path does not implement skeletal animation, skinning, or streamed GPU animation buffers. `.ranim` covers CPU-side sprite clip data only.
- Generic GPU per-class budgets are admission-enforced for standalone registry resources. The glTF model cache has a separate soft residency budget and pressure telemetry, but active Scene placements and required warmups are never invalidated to meet it. Neither budget is a global cap over transient targets, surfaces, driver allocations, or OS graphics memory. All reported bytes are estimates, not physical VRAM readings.

## 4. Scene activation, hot reload, ownership, and Physics

### Scene-wide render visibility commit (new continuation)

- Added the public renderer marker `StagedRender`; render extraction omits marked cameras, built-in/dynamic/generic GPU mesh instances, glTF model instances, and point/spot lights. The marker is queried as an optional ECS component to avoid a separate per-entity world lookup.
- Default non-preview streaming placements and SceneDoc prefab entities are staged while Region activation is incomplete.
- Staging is render-only. ECS presence, transform propagation, and physics registration are unchanged.
- The coordinator now closes the activation group over all Scene definitions containing a Region, following shared-Region membership transitively. A standalone Region remains a one-Region group.
- No member Region becomes Active and no group staging markers are removed until every member is Active or has completed visual spawning, SceneDoc instantiation, and required Rapier collider registration. The markers for the full group are removed first, then all Activating members transition to Active in the same Update, before PostUpdate extraction.
- `preview_when_ready` retains the explicit early-preview behavior and its app-owned staging markers are not cleared by the coordinator.
- Added an explicit two-Region barrier test: the first Region remains staged while the second is still PreparingPhysics, then both become visible/Active after the second is ready. Added extraction coverage across renderer component families and SceneDoc activation/unload coverage. These tests are source additions and remain unexecuted here because this environment has no Rust toolchain.

### Implemented

- A Region reaches `Active` only after required visual placements have been spawned and required Rapier colliders have been observed by the physics integration.
- Scene state aggregation prevents a multi-Region Scene from reporting `Active` until all of its Regions have reached their required state.
- Region-owned visual entities, prefab entities, and colliders are tracked and removed on deactivation/unload.
- Despawned collider entities are retained in pending-cleanup bookkeeping until the Physics diagnostic snapshot confirms they have left the simulation; asset leases and activation capacity are not released early.
- SceneDoc generation replacement and now same-handle content-revision changes trigger retirement of the old prefab-owned entity graph before re-instantiation. Model generation changes and typed dependency generation changes similarly retire/reacquire through the coordinator path.
- Reload/cancellation retains the old asset ownership until entity/physics cleanup is complete; queue saturation is retried as backpressure.

### Render visibility staging and atomicity boundary

The renderer-consumed `StagedRender` marker keeps coordinator-spawned model/light placements and SceneDoc prefab entities out of extraction while they are staged. Render extraction skips staged cameras, built-in/dynamic/generic GPU meshes, glTF models, and point/spot lights. The marker does not remove entities from ECS or physics; prefab colliders can register while their visuals remain hidden.

At commit time, `try_commit_render_visibility_group` computes the connected Scene/Region activation group. Shared Regions link their active Scene users, preventing a Region from being revealed early for one Scene while another activated Scene using that same Region is still preparing. The method waits for every Region in the group to finish its visual spawn work, SceneDoc instantiation, and required Rapier collider registration. It removes all `StagedRender` markers in the group before setting any member Region Active, in one Update before PostUpdate render extraction. This provides a **multi-Region visual commit at the Scene/Region ECS visibility boundary**. `preview_when_ready=true` intentionally bypasses staging and preserves early preview.

This is not a general render-graph transaction or rollback mechanism for arbitrary application-side changes, and it cannot make unrelated entities created outside the streaming ownership contract participate automatically. The visibility commit itself is not hardware-dependent; its source tests remain unexecuted in this environment.

## 5. Editor Streaming Debugger and authored partitions

`plugins/editor_tools/src/lib.rs` now displays:

- Region states and per-stage timing/last-max request-to-active values.
- Recent transition events with source/destination states, source-state residence milliseconds, and transition reason.
- CPU/GPU residency and generic GPU queue/pressure/release metrics.
- A World Partition panel that requests a `.worldpart` through `AssetServer`, displays decode/validation errors, calls `StreamingCoordinator::register_partition`, pins the source lease, and shows registered Region counts/states.

The `.worldpart` file remains authored as RON in the project's normal source editor; the current editor shell does not provide a text/RON editor widget. The panel is a load/validate/register workflow, not a full visual partition painting tool or live hot-reload/replace tool.

## 6. ATHAR integration audit

The actual transition path is in `games/athar/src/scripts/gameplay.rs`:

1. ATHAR's setup registers its village Scene and requests it using `StreamingCoordinator::request_scene`.
2. Gameplay reads `snapshot()` and the Region's real readiness/visual entity counts while the player remains in the room.
3. When the player enters the open doorway after preparation, ATHAR calls the public `activate_scene` API.
4. The commit wait checks `scene_state(SceneId)` for `SceneState::Active` or `Failed`.
5. `enter_village` rechecks both the whole Scene and the concrete Region state before moving the camera/player, switching lighting/weather, removing room-owned entities, and releasing the old room's leases.
6. The Streaming system remains the owner of village asset/Region/physics preparation and cleanup. ATHAR does not load the village's assets itself during the handoff.

This is a real consumer integration through the public StreamingCoordinator API, not a second gameplay-side simulation of Region readiness. Remaining caveat: there is no automated full graphical ATHAR playthrough in this environment, and the coordinator's visible-entity staging semantics are described above.

## 7. Long-running stress and soak testing

### Harness support

`examples/streaming_stress` supports up to 512 Regions, 20 forward/reverse cycles, and 512 generated GLB assets. The PowerShell runner collects build/phase logs and process working-set/private-byte samples. Headless mode exercises the actual AssetServer, StreamingCoordinator, ECS, and Rapier paths; it does not simulate GPU success. GPU mode opens a native window, submits actual renderer-owned GLB/texture/sampler uploads, checks per-frame upload budgets, records GPU timestamps where the adapter supports them, and verifies final lease/cache release.

`-ExpectGpuPressure` exercises a real adapter path that:

- Sets a tight generic GPU residency budget.
- Requests multiple textures and a sampler while holding all leases.
- Verifies an additional texture is rejected while existing leases are pinned.
- Drops the original leases after uploads become Ready.
- Requests a replacement texture to force eviction of an expired Ready texture.
- Waits for render-thread release and requires all expected generic resources to drain.
- Requires nonzero pressure, eviction, and budget-rejection counters.

The script reports the GPU pressure test separately and fails the GPU phase if any required condition is missing.

### Last measured run (before this latest gap-closing source delta)

Run `20261031-193330` is an actual Windows GPU run, not a CPU mock:

| Metric | Observed |
|---|---:|
| Regions / cycles | 48 / 2 |
| Generated GLBs | 64 |
| GPU frames presented | 2,473 |
| Model / generic uploads | 191 / 9 |
| Generic resources released | 9 / 9 |
| Upload budget violations | 0 |
| Peak active Regions | 4 |
| Successful activation samples | 95 / 95 |
| Final unloaded Regions | 52 / 52 |
| Activation latency p50 / p95 / p99 | 61.819 / 62.679 / 1,078.966 ms |
| App update wall p95 / peak | 7.616 / 983.464 ms |
| Render CPU frame p95 | 7.506 ms |
| GPU mesh pass timestamp p95 | 0.048 ms |
| Peak decoded CPU residency estimate | 85.52 MiB |
| Peak renderer GPU allocation estimate | 39.84 MiB |
| Peak process working set / private bytes | 425.47 / 533.26 MiB |

This run demonstrates 4-Region concurrency and generic upload/release behavior for that prior code snapshot. It did **not** use the new `-ExpectGpuPressure` test and it was only 2 cycles/48 Regions/64 assets. It is not evidence that the 10–20-cycle / high-cardinality soak or new pressure-rejection+eviction mode passes after the current source edits.

### Recommended Windows soak commands

Headless long soak (192 Regions, 20 cycles, 256 assets):

```powershell
.\examples\streaming_stress\run_streaming_stress.ps1 -Regions 192 -Cycles 20 -Assets 256 -FramesPerRegion 2 -HeadlessOnly
```

GPU residency pressure + actual upload soak (96 Regions, 10 cycles, 128 assets):

```powershell
.\examples\streaming_stress\run_streaming_stress.ps1 -Regions 96 -Cycles 10 -Assets 128 -FramesPerRegion 2 -TextureSize 512 -ExpectGpuPressure
```

For maximum generator/registration scale, the harness accepts up to `-Regions 512 -Cycles 20 -Assets 512`; increase frame counts only after the smaller soak finishes cleanly. A GPU run requires an interactive Windows display and compatible adapter.

## 8. Automated tests added or extended

Relevant source test names include:

- `lifecycle_timing_records_each_stage_and_request_to_active_latency`
- `dormant_reactivation_starts_a_new_activation_timing_attempt` (also checks the maximum attempt's phase breakdown is retained after a faster reactivation)
- `gpu_budget_rejects_overcommit_without_invalidating_a_live_lease`
- `gpu_budget_evicts_only_expired_queued_leases_by_priority`
- `expired_lease_does_not_evict_preparing_upload_before_validation`
- `sampler_count_budget_is_enforced_independently_of_byte_budget`
- `standalone_texture_payload_is_validated_and_deduplicated`
- `standalone_sampler_is_validated_and_deduplicated`
- `csv_keeps_existing_fields_and_appends_warmup_columns_without_reordering`
- `staged_render_entities_are_excluded_from_all_render_extraction_until_committed`
- `streamed_point_lights_are_staged_by_default_but_preview_policy_keeps_them_visible`
- `region_visibility_commit_removes_staging_from_visual_and_prefab_entities_together`
- `preview_region_does_not_clear_application_owned_render_staging`
- `multi_region_scene_visibility_commits_only_after_every_region_is_ready`
- `device_loss_requeues_leased_gpu_resources_from_retained_payloads`
- `model_cpu_memory_breakdown_matches_total_capacity_estimate`
- `aggregate_only_loaders_are_counted_as_other_cpu_memory`
- `asynchronous_completion_preserves_memory_breakdown_from_the_loader`
- `sprite_animation_clip_asset_loads_and_reports_animation_memory`
- `sprite_animation_clip_loader_rejects_empty_or_invalid_timing`
- `scene_doc_region_loads_and_owns_prefab_entities_through_activation_and_unload` now checks that prefab entities remain staged through `Activating` and become visible only after the visibility group commit.
- ATHAR transition/readiness tests in `games/athar/src/scripts/gameplay.rs`.

These tests are source additions; they have not been executed against this exact final overlay in the current environment.

## 9. Original 16-part prompt — acceptance crosswalk

| Original prompt part | Current status | Evidence / remaining limitation |
|---|---|---|
| 1. Repository audit | Largely implemented | `WORLD_STREAMING_DELIVERY_REPORT.md`, architecture docs, dependency boundary checks. |
| 2. Scene streaming | Implemented core; multi-Region visibility commit | Region/Scene state machines, SceneDoc loading, prefab ownership, physics cleanup, staged extraction, and a connected Scene/Region group commit; not an arbitrary render-graph rollback transaction. |
| 3. Asset streaming | Largely implemented | Reuses AssetServer workers, priorities, dedup, leases, cancellation, dependency readiness and cache budget. Re-test current overlay. |
| 4. GPU resource streaming | Partial, with device-recovery path | Generic Mesh/Texture/Sampler/PBR warmup and lease-safe eviction; live requests requeue after device loss. Static renderer shaders/pipelines rebuild through pass constructors but are not standalone streamed assets; skinned GPU animation remains unsupported. |
| 5. World partitioning | Implemented authored AABB-region core | `.worldpart`, validation, dependencies, neighbors, observer distances/hysteresis/prefetch; no procedural builder or visual painting UI. |
| 6. CPU/GPU memory | Partial, with categorized CPU estimates and separate GPU residency policies | CPU category estimates for glTF geometry/textures/materials/metadata, sprite animation clips, SceneDoc/WorldPartition metadata, and fallback Other; pinned-aware CPU cache; enforced generic GPU class budgets/eviction; soft glTF model-cache budget/pressure telemetry. GPU limits are not global/physical VRAM caps. |
| 7. Profiling/debug tools | Largely implemented | Per-state/event timing, retained slowest-attempt breakdown, CSV and editor diagnostics; next run needed to attribute old >1s spike. |
| 8. Threading/scheduling | Largely implemented; device recovery path added | AssetServer worker pool; GPU create/validate/release and device teardown on render owner; replacement device retries at bounded cadence. Recovery still needs actual Windows/GPU validation and can block during rare recreation attempts. |
| 9. Testing | Broad coverage present; latest result pending | Engine-level tests and real GPU harness exist; current additions not run here; high-scale soak pending. |
| 10. ATHAR integration | Implemented public API consumer path | request_scene/activate_scene/scene_state and Region verification; no full graphical playthrough in this environment. |
| 11. API/module quality | Largely implemented | `ribbon_plugin_streaming` public API and plugin integrations; generic GPU APIs intentionally renderer-owned. |
| 12. Performance/resource rules | Mostly implemented; hardware acceptance pending | Lease safety, per-frame GPU work caps, physics gates, shared assets, Scene-wide visibility commit, and source-level device-recovery path; no physical global VRAM cap or arbitrary ECS/render rollback transaction. |
| 13. Incremental strategy | Implemented across continuation chain | Layered source changes and fix reports; this overlay packages cumulative delta from the uploaded base ZIP. |
| 14. Validation | Incomplete for current delta | Prior workspace validation was reported green by the user on the previous overlay chain; current changes need a fresh Windows run. |
| 15. Acceptance | Partial | See above caveats; do not mark full production acceptance yet. |
| 16. Engineering report | Delivered here | This report maps implementations, evidence, limits, commands, and previous runtime measurements. |

## 10. Validation record and required rerun

### Previously reported by the project checkout (preceding overlay chain only)

The prior conversation included a successful `cargo check --workspace --all-targets`, `cargo test --workspace --no-fail-fast`, and `cargo clippy --workspace --all-targets -- -D warnings`, as well as focused suites including Streaming (42), Assets (20), Scene (9), Render profiling (12), glTF (5), Render3D (33), Physics (24), and ATHAR (21). Those results predate the latest phase-breakdown retention, SceneDoc revision handling, GPU budget pressure semantics, editor transition trace, and CSV extension in this gap-closing delta. They must not be reused as proof that this new delta builds.

### Static checks run for this package

- `python3 tools/validate_athar_transition_warmup.py`: **45/45 static checks passed** after updating its assertions to the generic StreamingCoordinator handoff and current 152-column CSV schema.
- Python `py_compile` passed for `tools/assets/generate_asset_stress_corpus.py` and `tools/validate_athar_transition_warmup.py`; a 2-asset/8px-texture corpus smoke generation also completed successfully.
- Python `tomllib` parsed all 44 TOML files in the work tree with no errors.
- Workspace structure and the overlay source tree were compared against the supplied base ZIP; no original base files were missing.
- CSV header/index consistency was checked: exactly 152 unique fields, including slowest-region phase fields at indices 126–134, device-recovery fields at 135–140, CPU asset category estimates at 141–146, and model-cache pressure fields at 147–151.
- `tools/check_deps.py` was attempted but could not run because it invokes `cargo metadata` and Cargo is absent in this environment; TOML syntax parsing is not a substitute for dependency-policy execution.
- ZIP integrity and SHA-256 are checked for the packaged overlay.

### Commands required on Windows before merge

```powershell
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo test -p ribbon_assets --all-targets
cargo test -p ribbon_scene --all-targets
cargo test -p ribbon_plugin_streaming --all-targets
cargo test -p ribbon_plugin_render3d --all-targets
cargo test -p ribbon_plugin_physics --all-targets
cargo test -p athar --all-targets
cargo test --workspace --no-fail-fast
cargo clippy --workspace --all-targets -- -D warnings
py -3 tools/check_deps.py
.\tools\validate_streaming.ps1
```

Then run both soak commands in Section 7. Do not accept the GPU Residency Manager until the `-ExpectGpuPressure` run reports both pinned-overcommit rejection and at least one safe Ready-resource eviction/release.

## 11. Final acceptance statement

The Streaming foundation is materially stronger: the >1s activation outlier can now be traced by retained per-phase metrics; the generic GPU manager has explicit admission/lease/eviction semantics; reload retirement respects entity and physics ownership; editor and ATHAR are connected to public coordinator diagnostics/APIs; and the harness can run meaningful 20-cycle/high-cardinality and real-GPU pressure workloads.

This is a substantially more complete implementation of the supplied prompt, but it is **not honestly sign-off ready for production** until the new source has compiled and its test suites and Windows/GPU stress runs pass. Multi-Region Scene visibility commit, renderer device-loss/recreation handling, and CPU asset category accounting are now implemented in source. A `.ranim` CPU animation clip asset and playback path is also present. Remaining scope boundaries are: no independent streaming registry for arbitrary shaders/pipelines (existing embedded renderer pipelines are recreated on device recovery), no skeletal glTF/GPU animation pipeline, GPU memory estimates are not a physical VRAM cap, no editor painting UI/procedural partition authoring, and no executed post-change Windows Rust/GPU validation. These are genuine remaining limitations, not successful acceptance claims.
