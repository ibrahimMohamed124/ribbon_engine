# Ribbon Assets (`ribbon_assets`)

Provides typed asset storage, synchronous loading for startup paths, asynchronous worker-backed source IO/CPU decoding, strong leases, generation-safe eviction, lifecycle events, and polling hot reload. `BytesLoader` and `TextLoader` cover common unprocessed data.

- Use `request::<T>(path)` or `request_ref::<T>(path)` for asynchronous regular loads. Results become visible when `AssetPlugin` polls in `Stage::First` or when direct users call `poll_pending_loads`.
- `Handle<T>` stays a weak copyable ID. `AssetRef<T>` pins an asset generation; retain it as a resource/member while the asset must stay resident. Automatic eviction is opt-in.
- Worker threads run source IO and CPU decoding only. GPU object creation and upload belong to the renderer/device-owning thread.
- Request and decoded-completion queues are bounded by item count. If an initial request cannot enter the queue, it returns `LoadErrorKind::QueueFull`; call `retry` after polling progress. Hot-reload queue admission is retried on a later poll while the last-good value remains resident. These counts are not strict byte limits for large/in-flight decodes.
- `set_cpu_memory_budget_bytes` configures a soft cache budget for unleased decoded assets. `cpu_memory_stats` exposes loader estimates and budget pressure; `request_with_priority`/`request_ref_with_priority` affect eviction retention, not worker scheduling. Leased assets are never evicted to meet the budget. See [`../../docs/CPU_ASSET_CACHE_BUDGET.md`](../../docs/CPU_ASSET_CACHE_BUDGET.md).
- Hot reload retains the last good value on failed replacement and records `ReloadFailed`. `AssetPlugin` and servers using `request` run hot-reload reads/decoding on workers; timestamp detection is still polled synchronously. Direct synchronous-only servers keep legacy immediate hot reload unless opted in.

Allowed workspace dependencies: `ribbon_app`, `ribbon_plugins`, and `ribbon_events`. This crate does not silently replace missing assets with defaults. See [`../../docs/ASSET_LIFETIME_STREAMING.md`](../../docs/ASSET_LIFETIME_STREAMING.md) for lifetime and retry semantics.
