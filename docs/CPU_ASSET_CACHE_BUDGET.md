# CPU asset cache budget and retention policy

Date: 2026-10-23

## Purpose and scope

The asset server can report estimated decoded CPU residency and evict unleased cache entries when they exceed a configured soft budget. This prevents old, unpinned decoded assets from accumulating indefinitely when a caller continues polling the server. It is not a process RSS limiter, a hard total-memory ceiling, or a budget for GPU objects.

## API contract

```rust
let server = app.world.resource_mut::<AssetServer>();
server.set_cpu_memory_budget_bytes(Some(64 * 1024 * 1024));
```

Call `set_cpu_memory_budget_bytes(None)` to disable budget enforcement. Asynchronous loads and hot-reload replacements are accounted after `poll_pending_loads` commits their results. A direct synchronous-only caller should call `enforce_cpu_memory_budget()` after applying a new budget or after loading/replacing values synchronously. The method returns the number of assets evicted in that call.

`cpu_memory_stats()` reports:

- loaded resident asset count and estimated resident bytes;
- bytes and asset count pinned by at least one `AssetRef<T>`;
- unpinned bytes eligible for cache eviction;
- configured unpinned cache budget and unpinned-byte overage;
- cumulative values evicted specifically during budget enforcement.

If strong-lease accounting is poisoned, eviction is conservative: assets are treated as pinned and are not freed based on untrusted counts.

## Size estimates

`AssetLoader::estimated_size_bytes` defaults to `size_of::<Asset>()`, suitable only for assets without meaningful owned heap allocations. Heap-owning loaders should override it and account for the capacities they allocate. Current built-in estimators:

- `BytesLoader`: `BytesAsset` container plus backing `Vec<u8>` capacity.
- `TextLoader`: `TextAsset` container plus `String` capacity.
- `GltfLoader`: `ModelAsset` container, top-level vector capacities, primitive position/normal/UV/index capacities, and decoded RGBA image-buffer capacities.

Estimates do not include allocator metadata, alignment/fragmentation, decode scratch, source/compressed bytes temporarily held by workers, queued decoded completions, or GPU copies. They should be used for cache policy trends, not as exact operating-system memory measurements.

## Eviction order and ownership

A value held by an `AssetRef<T>` is never selected for budget eviction. If active/pinned values alone exceed the budget, total residency may exceed it by design; the unpinned portion is still trimmed to the configured budget where candidates are available.

Among unpinned loaded values, the lowest cache-retention priority is selected first. Equal priorities are ordered by successful load/reload sequence, oldest first (FIFO). This is not access-based LRU. Priority can be assigned via `request_with_priority`, `retry_with_priority`, and `request_ref_with_priority`; it affects eviction retention only, not worker queue scheduling or cancellation. Eviction sets the path to `Unloaded`, clears the resident estimate, removes revision history and emits `AssetEvent::Unloaded`. A later request advances the path generation, so stale weak handles must not be reused.

Budget enforcement runs after asynchronous completion batches are committed. If no cache budget is configured, the existing opt-in `set_auto_evict_unused` behavior remains unchanged. A caller that changes the budget in a synchronous-only server must call enforcement explicitly.

## Multi-asset stress scenario

`tools/generate_asset_stress_corpus.py` creates self-contained textured GLB models below `target/asset-stress-corpus/`. The stress harness can pin a set of distinct active assets, request additional prefetch assets without leases at low retention priority, then capture resource and timing profiles under a configured unpinned-byte budget. See [`ASSET_LIFETIME_LARGE_SCENE_STRESS.md`](ASSET_LIFETIME_LARGE_SCENE_STRESS.md) for the full PowerShell command and report interpretation.

## Remaining limitations

- There is no explicit cancellation of obsolete requests or progress API.
- Worker queues have item-count limits, not a byte-weighted admission policy; in-flight or decoded-but-not-committed values can temporarily exceed the resident-cache budget.
- Retention priority does not reorder worker scheduling.
- GPU resource ownership/eviction remains inside the renderer cache and is not governed by this CPU cache budget.
- No new tests were added. Compile, lint, runtime lifecycle, and memory profiling must be verified on the target Windows checkout before considering the feature accepted.
