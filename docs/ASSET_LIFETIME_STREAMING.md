# Asset lifetime and streaming

Date: 2026-10-22

## Implementation status

The first streaming API is implemented in `engine_assets`; it is source-complete but has not been compiled or exercised on the target Windows checkout in this delivery. Treat target-side verification as required before merging.

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
7. `plugin_render3d` performs GPU upload from its render pass, where it owns the renderer/device. CPU loader threads never create GPU resources.

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

## Boundaries not yet implemented

- There is no request priority, explicit cancellation, progress reporting, byte-based memory budget, or LRU/age policy. Queue item counts cap outstanding work, but a single asset can still decode to a large allocation.
- A caller must poll the server for completions. If the main loop stops polling, the completion queue applies backpressure and worker threads may wait.
- Modification timestamp checks still run synchronously at the configured hot-reload polling interval. There is no generic GPU-upload queue; renderer passes own that stage.
- Asset bundles/dependencies are not yet scheduled as a dependency graph; loaders must return a self-contained CPU-side asset value.
- Renderer GPU uploads remain owned by renderer passes; this milestone does not create a generic cross-backend GPU upload queue.
- No new tests were added and no Rust build, formatter, linter, runtime, benchmark, or test command was run as part of this delivery. Target-side validation is mandatory before accepting this milestone.
