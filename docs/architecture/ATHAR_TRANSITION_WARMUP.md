# ATHAR room-to-village transition warmup

Date: 2026-10-29

## Readiness contract

The room-to-village path uses the existing `AssetServer` and renderer-owned model cache. `main.rs`
queues the seven unique village `ModelAsset` paths once and keeps strong `AssetRef` leases in
`GameModelLeases`. Those handles and leases only establish request identity and CPU-cache lifetime;
they do not imply readiness.

`ModelWarmup` in `ribbon_plugin_render3d` tracks each handle generation as `WaitingForCpu`,
`WaitingForGpu`, `Ready`, or `Failed`. CPU work remains on `AssetServer` workers and results are
committed by `AssetPlugin` in `Stage::First`. The renderer resolves CPU residency from `AssetServer`,
then creates GPU meshes, material resources, and decoded textures from its render-thread-owned
`MeshRenderer`. It wraps each upload in wgpu validation and out-of-memory error scopes and polls the
scopes without blocking on later frames. `Ready` is published only when the upload scopes resolve
without error and the cache entry/revision remains resident.

Warmups do not create dummy scene entities. A requested handle participates in cache liveness even
when no active `ModelInstance` references it. Active visible placements have priority; warmups share
the same `MAX_MODEL_GPU_UPLOADS_PER_FRAME = 2` budget. The upload limit is not raised or removed.
After village placements are created, ATHAR releases warmup-only requests within the same `Update`,
when the real placements will be included in `PostUpdate` extraction before the render pass. CPU leases
remain in `GameModelLeases` for the session. This is reasonable for this prototype's small fixed asset
set, but decoded RGBA images and geometry can use substantially more RAM than the 24.6 MiB of
compressed GLB inputs.

## Transition state and failure behavior

ATHAR starts preparation at startup while the player explores the room. Until all seven unique model
generations are CPU-resident and GPU-warmup-validated, the room remains active, the HUD reports
progress, and first-person movement is clamped at the outside doorway. The door animation and its
existing unlock/clearance condition remain in effect. Once ready, the game waits until the door is
mostly open, then stages the 128 village model placements and eight point lights as render-only
entities while retaining the room. Outdoor clear color is revealed as the door opens; outdoor sun,
fog, and the longer camera far plane are applied when the preview exists. The ground and road top
surfaces sit 2 cm and 1.4 cm below the room's y=0 floor, respectively, so the room floor occludes
them indoors without coincident surfaces. No village colliders are created during preview. At commit
the staged visuals are reused, the 111 Rapier collider descriptors are added, then room entities,
colliders, and lights are removed. If spawn preflight fails, the room is not removed. The phase/state
gate prevents duplicate commits. Missing or failed CPU/GPU resources display the path
and reason and can be explicitly retried near the open doorway; errors are not converted into success.

The existing render graph's window clear still runs every frame, even when the mesh draw list is empty.
The transition avoids deliberately entering a scene before its models are ready; it does not claim that
an in-game black pixel/surface failure was visually observed or ruled out without a runtime capture.
The weather plug-in also exposes a one-time `RainDiagnostics` batch timing: the existing 220-particle
activation is synchronous in `Update`, so the timing is needed to determine whether it contributes a
visible hitch on target hardware rather than assuming model uploads are the only cause.

## Instrumentation

With `RenderProfilePlugin` installed, the 81-column CSV includes the existing appended warmup counts
(`model_warmup_*`) and `model_gpu_uploads_this_frame`. The renderer records a nested CPU phase named
`model GPU resource creation/submission`; it measures host-side wgpu object creation and queue submission,
not asynchronous GPU execution, and overlaps the broader model resolution/draw-assembly timing. The first
3D transform-propagation pass after village activation appears as a `phase` row named
`3D transform propagation (one-shot)`. That timer is disarmed immediately and does not call the clock during
steady-state propagation. Reusing the existing phase-record format avoids changing the public
`RenderResourceStats` shape or CSV schema.
Existing CPU phase/pass timings include asset-cache reconciliation, model resolution/draw assembly, and
scene extraction/draw building. CPU asset resident/pinned bytes are captured while profiling. At transition,
ATHAR logs the preparation duration, synchronous `spawn_village` duration and created counts, the one-shot
3D transform propagation duration, and the first PreUpdate physics reconciliation/fixed-step durations after
activation. Physics timing fields are exposed through `PhysicsDiagnostics`.

Suggested Windows capture (play the game, then close the window so the profile is flushed):

```powershell
cargo run -p athar --release -- --profile-csv target/profiles/athar-transition.csv --profile-warmup 120
py -3 tools/perf/render_profile_report.py target/profiles/athar-transition.csv --output target/profiles/athar-transition-report.md
```

Exercise key pickup, door unlock, and the transition during this capture. Inspect frame-wall and
render-graph CPU P95/P99/peak, mesh pass CPU timings, `model_gpu_uploads_this_frame`, warmup counts,
resource cache entries and CPU memory. Each frame's model upload count must remain at most two; the
warmup-ready counter should reach seven before the transition commit. Compare a before/after capture on
the same GPU/window present mode before claiming a measured latency improvement. GPU timestamps are
asynchronous and their CSV sample may correspond to work submitted in an earlier frame.

## Scope and limitations

- Village placements and collider descriptors are still spawned synchronously in one `Update` after
  readiness. `spawn_village` logs elapsed time and its exact model/collider/entity counts. The first
  following physics pass logs reconcile and fixed-step CPU time; the weather plug-in logs its initial
  synchronous rain allocation batch. These timings are instrumentation, not measurements obtained in
  this environment. No batching/streaming system is added without a measured frame spike; if values are
  significant on target hardware, use the capture to justify bounded collider activation or scene
  batching as a separate change.
- CPU asset decoding remains off-thread; GPU creation remains renderer-thread-owned.
- The renderer validates CPU model structure before upload and surfaces wgpu validation/out-of-memory
  errors asynchronously. This is not a cross-vendor GPU hardware test.
- Room/village collision uses primitive proxies rather than mesh-accurate collision. The monster is
  still a placeholder and the interaction target checks remain proximity-based.
- The full app and manual visual transition have to be validated on Windows with an actual GPU. Source
  tests cannot prove the final image looks correct or establish an observed frame-time improvement.
