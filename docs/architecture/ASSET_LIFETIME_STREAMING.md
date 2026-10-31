# Asset lifetime and streaming

Date: 2026-10-30

## Implementation status

The first streaming API is implemented in `ribbon_assets`; it is source-complete but has not been compiled or exercised on the target Windows checkout in this delivery. Treat target-side verification as required before merging.

## Ownership contract

- `AssetId` is an opaque, stable identity allocated by one `AssetServer`.
- `Handle<T>` remains a copyable weak ID for compatibility. It includes a generation internally and does not retain an asset against eviction.
- `AssetRef<T>` is a strong lease. Cloning one lease shares the same pin; calling `strong_ref`/`request_ref` independently creates another pin. The pin does not outlive the `AssetServer`.
- An eviction only removes a resident asset when the strong lease count for that `(AssetId, generation)` is zero. Its path enters `Unloaded`; reloading advances the generation so stale handles cannot resolve to a replacement value.
- Automatic eviction is opt-in (`set_auto_evict_unused(true)`) because existing clients may retain only weak handles. Keep `AssetRef`s in resources/components for every asset that must stay resident.

## Asynchronous flow

1. Register a source and CPU-only `AssetLoader`.
2. Call `request::<T>(path)` or `request_ref::<T>(path)`. Identical type/path requests share an in-flight request.
3. Worker threads perform source IO and CPU decode only. The dispatch queue is bounded at 256 requests. A full queue returns `LoadErrorKind::QueueFull`; callers can retry after polling pending loads.
4. The decoded-completion queue is bounded at 64 values. Workers backpressure when the application stops polling rather than retaining an unbounded number of decoded assets.
5. `AssetPlugin` polls completions in `Stage::First`; direct server users can call `poll_pending_loads`. Completion events are emitted on the server-owning thread, and the value is visible through `get` only after commit.
6. With `AssetPlugin` installed, hot-reload reads and CPU decoding are also worker-backed. Each path admits only one reload at a time; generation/token checks reject stale completions. A source whose timestamp changes during a read is rejected rather than installed with a newer timestamp. Servers that use only synchronous `load` retain immediate legacy hot reload unless `set_async_hot_reload(true)` is requested; calling `request` opts into asynchronous hot reload.
7. `ribbon_plugin_render3d` performs GPU upload from its render pass, where it owns the renderer/device. CPU loader threads never create GPU resources.

`set_worker_count` can change the worker count before the first request (range 1–32). The server starts up to four workers by default, based on available parallelism. `load` and `load_ref` remain available for startup paths where synchronous loading is intentional.

## Lifecycle and reload policy

- `AssetEvent::Loading`, `Loaded`, `Failed`, and `Unloaded` report request/lifetime transitions. `Modified` indicates successful hot reload while preserving the handle ID/generation. `ReloadFailed` reports an error but keeps the last successfully decoded value available.
- `state::<T>(path)` reports `Loading`, `Loaded`, `Failed(error)`, or `Unloaded`.
- `retry::<T>(path)` starts a new generation after a failed request. A successful hot reload keeps the existing generation and increments `revision(handle)`.
- `get_mut(handle)` conservatively increments the content revision because the server cannot observe when callers subsequently mutate the returned reference. Renderers can use `revision` to invalidate derived GPU caches.
- Hot-reload detection still polls modification timestamps synchronously at the configured interval, but actual source reads and CPU decode run on workers for AssetPlugin/request users. Queue admission that temporarily fails is retried on a later poll.

## Example

```rust
let lease = {
    let server = app.world.resource_mut::<AssetServer>();
    server.request_ref::<ModelAsset>("models/room.glb")?
};
let weak_handle = lease.handle();
app.insert_resource(RoomModelLease(lease));

// Later, after AssetPlugin has polled pending loads:
if let Some(model) = app.world.resource::<AssetServer>().get(weak_handle) {
    // The model's CPU data is ready. The renderer uploads GPU data on its own render thread.
    let _bounds = model.bounds();
}
```

Do not enable automatic eviction for a subsystem that stores only weak handles unless the subsystem deliberately re-acquires/pins its assets. If an *initial request* cannot enter the bounded worker queue, it returns `LoadErrorKind::QueueFull` and the path enters `Failed`; call `retry` after polling progress to start a new generation. A hot-reload job that cannot enter the queue is left pending for the next polling interval and does not discard the last-good asset. `load_or` also preserves an already-`Loading` request instead of replacing it with a fallback value.

## Cache budget and retention priority (added 2026-10-23)

`AssetLoader::estimated_size_bytes` lets loaders estimate decoded resident memory. Its default counts only `size_of::<Asset>()`; loaders that allocate heap storage should override it and include the capacities they own. `GltfLoader` now estimates the `ModelAsset` container, vector capacities for primitives/materials/images/nodes, nested geometry/index vectors, and decoded RGBA image buffers. `BytesLoader` and `TextLoader` include their backing byte/string capacity. These are cache estimates, not allocator or process-RSS telemetry; transient decode scratch and queued decoded completions are not included.

`AssetServer::set_cpu_memory_budget_bytes(Some(bytes))` enables a **soft budget for unleased decoded cache entries**, not a hard process-memory limit. After `poll_pending_loads` commits completed asynchronous loads/reloads, the server evicts eligible values until unpinned residency fits or there are no candidates left. It evicts lower-retention-priority entries first, then older loaded entries (FIFO by load sequence). Active `AssetRef<T>` leases are never evicted; pinned bytes may therefore exceed the budget. Synchronous-only callers can apply a newly configured budget or a synchronous load's changed residency by calling `enforce_cpu_memory_budget()` explicitly. Set `None` to disable budget-based eviction.

- `request_with_priority`, `retry_with_priority`, and `request_ref_with_priority` set both **worker scheduling** and cache-retention priority. Higher priority queued jobs run first, FIFO ordering is retained for equal effective priorities, and queued jobs can be upgraded. Running decode is not preempted. Jobs gain one effective priority point every two seconds while queued so low-priority work is not starved indefinitely.
- `cpu_memory_stats()` reports resident, pinned, unpinned, budget, over-budget, loaded/pinned counts, and cumulative budget evictions. Budget overage is computed against unpinned bytes only.
- Budget eviction emits `AssetEvent::Unloaded` and advances the path generation on the next request. Stale weak handles must be treated as nonresident and reacquired.
- The CPU-cache scan is collected into render profile resource snapshots only while `RenderProfilePlugin` is installed; the ordinary per-frame renderer path does not scan every asset just to populate these new fields.

## GPU model upload pacing (added 2026-10-23)

The user-provided multi-asset capture identified a 43.541 ms CPU `mesh pass` while 32 new model-cache entries appeared in one frame. `ribbon_plugin_render3d` now limits missing model GPU-cache entries to two distinct asset keys per frame. Cache hits remain drawable, while placements that need additional missing models are deferred and retried on later frames. See `docs/architecture/GPU_ASSET_UPLOAD_PACING.md` for the frame-level evidence, validation procedure, and limitations. This is a count-based pacing policy, not a hard time/byte budget; voxel mesh uploads remain on their existing path.

## Boundaries still not implemented

- There is no force-cancellation of shared or already-running source/decode work. Releasing the last `AssetRef<T>` drops ownership safely; a queued/running operation may still complete, but stale generations are rejected and an unpinned result is eligible for budget eviction. This avoids cancelling a job that another consumer may share.
- Queue capacities bound item counts, not total bytes. One very large source file or several decoded-but-not-committed results can temporarily exceed the decoded resident-cache budget; loaders should reject unreasonable inputs and callers should avoid filling all 64 completion slots with huge files.
- The completion queue applies backpressure when the owner stops polling; worker threads may wait on that bounded channel, but the main/render thread never waits for a worker result.
- Modification timestamp checks still run synchronously at the configured hot-reload polling interval. Renderer passes remain responsible for GPU uploads; the model warmup adapter described below is currently the generic streaming coordinator's only GPU preparation adapter.
- Asset loaders return self-contained typed assets. Region-to-region readiness dependencies are managed by `ribbon_plugin_streaming`; a general arbitrary typed asset dependency graph/cycle scheduler is not implemented.
- Asset source / loader panics are caught on workers and surfaced as typed decode failures so a path cannot remain stuck in `Loading`. Panic hooks may still emit the standard Rust panic message in debug/test runs.

## Renderer-owned model warmup and scene-transition handoff (added 2026-10-29)

A CPU asset lease is not a GPU-residency lease. For transitions that must not reveal a scene one
model at a time, `ribbon_plugin_render3d::ModelWarmup` holds a request keyed by the exact model
handle ID/generation while the renderer uploads its model cache entry. Its public status distinguishes
CPU wait, GPU wait, readiness and a failure reason. AssetServer still owns asynchronous file IO and
CPU decoding; the renderer still owns wgpu object creation on the render thread.

An explicit warmup request keeps the normal model cache entry live even when no `ModelInstance`
exists yet. Releasing the warmup request allows normal cache reconciliation to evict the entry only
when no active scene placement also references the handle. In a scene handoff, spawn the actual
placements before releasing warmup-only retention so the same render reconciliation can observe the
new placements and avoid a cache-eviction gap. Do not infer readiness from a `Handle`, an
`AssetRef`, or `LoadState::Loaded` alone.

Warmup entries carry an explicit priority and stable insertion sequence. Higher priority entries are attempted first; equal priority entries remain FIFO. Multiple regions requesting the same model generation share one warmup entry, and a later higher-priority request upgrades it without duplicating renderer work. The render pass continues to service visible placements before background warmups and applies the same bounded model-upload allowance to both.

ATHAR retains its room assets through game-owned `AssetRef<ModelAsset>` leases and registers the
village as a region with `ribbon_plugin_streaming`. The streaming coordinator keeps one strong lease
per distinct model path per region, shares AssetServer's typed-path load across regions, and shares
renderer `ModelWarmup` residency through a region-user count. It releases region leases only after
owned collider entities have disappeared from the physics plugin's reconciled snapshot. The room's
leases remain alive through the village readiness/activation handoff and are cleared only after the
village is active. During a profiled room-to-village handoff, the renderer also emits a dedicated CPU
phase for GPU resource creation/submission, and `ribbon_scene` exposes a one-shot 3D transform-
propagation duration so resource preparation and ECS transform costs can be distinguished from
asynchronously completed GPU timing samples.
