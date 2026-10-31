use super::{
    AssetDependencyKind, AssetDependencyLoadState, AssetEvent, AssetLoader, AssetRef, AssetServer,
    BytesAsset, BytesLoader, CancellationToken, LoadContext, LoadError, LoadErrorKind, LoadState,
    MemorySource, TextAsset, TextLoader,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

struct CooperativeLoader {
    started: Arc<AtomicBool>,
    finished: Arc<AtomicBool>,
}

impl AssetLoader for CooperativeLoader {
    type Asset = TextAsset;

    fn extensions(&self) -> &[&str] {
        &["slow"]
    }

    fn load(&self, _bytes: &[u8], context: &mut LoadContext) -> Result<Self::Asset, LoadError> {
        Err(LoadError::new(
            LoadErrorKind::Decode,
            &context.path,
            "cooperative test loader must use load_cancellable",
        ))
    }

    fn load_cancellable(
        &self,
        _bytes: &[u8],
        context: &mut LoadContext,
        cancellation: &CancellationToken,
    ) -> Result<Self::Asset, LoadError> {
        self.started.store(true, Ordering::Release);
        while !cancellation.is_cancelled() {
            std::thread::yield_now();
        }
        self.finished.store(true, Ordering::Release);
        Err(LoadError::new(
            LoadErrorKind::Cancelled,
            &context.path,
            "test decode observed cancellation",
        ))
    }
}

fn cooperative_server() -> (AssetServer, Arc<AtomicBool>, Arc<AtomicBool>) {
    let source = MemorySource::default();
    source.insert("pending.slow", b"bytes".to_vec());
    let started = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(AtomicBool::new(false));
    let mut assets = AssetServer::default();
    assets.add_source(0, source);
    assets.register_loader(CooperativeLoader {
        started: Arc::clone(&started),
        finished: Arc::clone(&finished),
    });
    (assets, started, finished)
}

#[test]
fn memory_source_loads_typed_assets() {
    let source = MemorySource::default();
    source.insert("ui/title.txt", b"Ribbon".to_vec());
    let mut assets = AssetServer::default();
    assets.add_source(0, source);
    assets.register_loader(TextLoader);
    let handle = assets.load::<TextAsset>("ui/title.txt").unwrap();
    assert_eq!(assets.get(handle), Some(&TextAsset("Ribbon".to_owned())));
}

#[test]
fn extensions_route_to_the_registered_loader() {
    let source = MemorySource::default();
    source.insert("data/level.bytes", vec![1, 2, 3]);
    let mut assets = AssetServer::default();
    assets.add_source(0, source);
    assets.register_loader(BytesLoader);
    let handle = assets.load::<BytesAsset>("data/level.bytes").unwrap();
    assert_eq!(assets.get(handle), Some(&BytesAsset(vec![1, 2, 3])));
}

#[test]
fn failed_load_records_typed_state_and_default_is_explicit() {
    let mut assets = AssetServer::default();
    assets.register_loader(TextLoader);
    assert_eq!(
        assets.load::<TextAsset>("missing.txt").unwrap_err().kind,
        LoadErrorKind::MissingSource
    );
    assert!(matches!(
        assets.state::<TextAsset>("missing.txt"),
        Some(LoadState::Failed(_))
    ));
    let fallback = assets.load_or("missing.txt", TextAsset("fallback".to_owned()));
    assert_eq!(
        assets.get(fallback),
        Some(&TextAsset("fallback".to_owned()))
    );
}

#[test]
fn current_handle_tracks_the_queued_generation_before_cpu_load_completes() {
    let source = MemorySource::default();
    source.insert("queued.bytes", vec![1, 2, 3, 4]);
    let mut assets = AssetServer::default();
    assets.add_source(0, source);
    assets.register_loader(BytesLoader);

    let requested = assets.request::<BytesAsset>("queued.bytes").unwrap();
    assert_eq!(
        assets.current_handle::<BytesAsset>("queued.bytes"),
        Some(requested)
    );
    assert!(matches!(
        assets.state::<BytesAsset>("queued.bytes"),
        Some(LoadState::Loading)
    ));
}

#[test]
fn polling_reloads_memory_assets_without_changing_the_handle() {
    let source = MemorySource::default();
    source.insert("config.txt", b"before".to_vec());
    let mut assets = AssetServer::default();
    assets.add_source(0, source.clone());
    assets.register_loader(TextLoader);
    assets.set_poll_interval(Duration::ZERO);
    let handle = assets.load::<TextAsset>("config.txt").unwrap();
    let initial_revision = assets.revision(handle).unwrap();
    source.insert("config.txt", b"after".to_vec());
    assert_eq!(
        assets.poll_hot_reload(),
        [AssetEvent::Modified { id: handle.id() }]
    );
    assert_eq!(assets.get(handle), Some(&TextAsset("after".to_owned())));
    let reloaded_revision = assets.revision(handle).unwrap();
    assert!(reloaded_revision > initial_revision);
}

#[test]
fn evicted_asset_reloads_with_new_generation_and_stales_old_handle() {
    let source = MemorySource::default();
    source.insert("config.txt", b"before".to_vec());
    let mut assets = AssetServer::default();
    assets.add_source(0, source.clone());
    assets.register_loader(TextLoader);
    let first = assets.load::<TextAsset>("config.txt").unwrap();
    assert_eq!(
        assets.current_handle::<TextAsset>("config.txt"),
        Some(first)
    );

    assert_eq!(assets.evict_unused(), 1);
    assert!(assets.get(first).is_none());

    source.insert("config.txt", b"after".to_vec());
    let second = assets.load::<TextAsset>("config.txt").unwrap();
    assert_eq!(first.id(), second.id());
    assert_eq!(second.generation(), first.generation() + 1);
    assert_eq!(
        assets.current_handle::<TextAsset>("config.txt"),
        Some(second)
    );
    assert!(assets.get(first).is_none());
    assert_eq!(assets.get(second), Some(&TextAsset("after".to_owned())));
}

#[test]
fn cpu_memory_budget_evicts_unleased_assets_but_never_invalidates_a_live_lease() {
    let source = MemorySource::default();
    source.insert("world/pinned.txt", b"pinned payload".to_vec());
    source.insert("world/cache.txt", b"cache payload".to_vec());
    let mut assets = AssetServer::default();
    assets.add_source(0, source);
    assets.register_loader(TextLoader);

    let pinned = assets
        .load_ref::<TextAsset>("world/pinned.txt")
        .expect("synchronous asset load and lease succeed");
    let unpinned = assets
        .load::<TextAsset>("world/cache.txt")
        .expect("second asset loads into the cache");
    assets.set_cpu_memory_budget_bytes(Some(0));

    let before = assets.cpu_memory_stats();
    assert_eq!(before.loaded_assets, 2);
    assert_eq!(before.pinned_assets, 1);
    assert!(before.pinned_bytes_estimate > 0);
    assert!(before.unpinned_bytes_estimate > 0);
    assert_eq!(before.over_budget_bytes, before.unpinned_bytes_estimate);

    assert_eq!(assets.enforce_cpu_memory_budget(), 1);
    assert_eq!(
        assets.get(pinned.handle()),
        Some(&TextAsset("pinned payload".to_owned()))
    );
    assert!(
        assets.get(unpinned).is_none(),
        "only the unleased cache entry may be evicted"
    );
    assert_eq!(assets.cpu_memory_stats().loaded_assets, 1);

    drop(pinned);
    assert_eq!(assets.enforce_cpu_memory_budget(), 1);
    assert_eq!(assets.cpu_memory_stats().loaded_assets, 0);
}

struct PanicsForOneAssetLoader;

impl AssetLoader for PanicsForOneAssetLoader {
    type Asset = TextAsset;

    fn extensions(&self) -> &[&str] {
        &["panictext"]
    }

    fn load(&self, bytes: &[u8], context: &mut LoadContext) -> Result<Self::Asset, LoadError> {
        if context.path == "bad.panictext" {
            panic!("intentional loader panic for regression coverage");
        }
        String::from_utf8(bytes.to_vec())
            .map(TextAsset)
            .map_err(|error| {
                LoadError::new(LoadErrorKind::Decode, &context.path, error.to_string())
            })
    }
}

#[test]
fn panicking_async_loader_becomes_a_typed_failure_and_worker_keeps_serving_jobs() {
    let source = MemorySource::default();
    source.insert("bad.panictext", b"bad".to_vec());
    source.insert("good.panictext", b"still alive".to_vec());
    let mut assets = AssetServer::default();
    assets.add_source(0, source);
    assets.register_loader(PanicsForOneAssetLoader);

    assets.request::<TextAsset>("bad.panictext").unwrap();
    let good = assets.request::<TextAsset>("good.panictext").unwrap();

    for _ in 0..100_000 {
        let _ = assets.poll_pending_loads();
        let bad_failed = matches!(
            assets.state::<TextAsset>("bad.panictext"),
            Some(LoadState::Failed(error)) if error.kind == LoadErrorKind::Decode
        );
        if bad_failed && assets.get(good).is_some() {
            break;
        }
        std::thread::yield_now();
    }

    assert!(matches!(
        assets.state::<TextAsset>("bad.panictext"),
        Some(LoadState::Failed(error)) if error.kind == LoadErrorKind::Decode
    ));
    assert_eq!(assets.get(good), Some(&TextAsset("still alive".to_owned())));
    assert_eq!(assets.worker_stats().active_workers, 0);
}

#[test]
fn cancel_if_unleased_preserves_a_shared_asset_lease() {
    let (mut assets, started, _) = cooperative_server();
    let lease: AssetRef<TextAsset> = assets.request_ref("pending.slow").unwrap();
    let handle = lease.handle();
    let second_lease = lease.clone();
    for _ in 0..100_000 {
        if started.load(Ordering::Acquire) {
            break;
        }
        std::thread::yield_now();
    }
    assert!(started.load(Ordering::Acquire));
    assert!(!assets.cancel_if_unleased(handle));
    drop(lease);
    assert!(!assets.cancel_if_unleased(handle));
    drop(second_lease);
    assert!(assets.cancel_if_unleased(handle));
}

#[test]
fn running_loader_observes_cooperative_cancellation() {
    let (mut assets, started, finished) = cooperative_server();
    let handle = assets.request::<TextAsset>("pending.slow").unwrap();
    for _ in 0..100_000 {
        if started.load(Ordering::Acquire) {
            break;
        }
        std::thread::yield_now();
    }
    assert!(started.load(Ordering::Acquire));
    assert!(assets.cancel(handle));
    for _ in 0..100_000 {
        let _ = assets.poll_pending_loads();
        if finished.load(Ordering::Acquire) && assets.worker_stats().active_workers == 0 {
            break;
        }
        std::thread::yield_now();
    }
    assert!(finished.load(Ordering::Acquire));
    assert!(assets.get(handle).is_none());
    assert!(matches!(
        assets.state::<TextAsset>("pending.slow"),
        Some(LoadState::Unloaded)
    ));
}

#[test]
fn cancelled_async_generation_never_commits_and_retry_uses_a_new_generation() {
    let source = MemorySource::default();
    source.insert("cancelled.txt", b"discard me".to_vec());
    let mut assets = AssetServer::default();
    assets.add_source(0, source);
    assets.register_loader(TextLoader);

    let cancelled = assets.request::<TextAsset>("cancelled.txt").unwrap();
    assert!(assets.cancel(cancelled));
    assert!(matches!(
        assets.state::<TextAsset>("cancelled.txt"),
        Some(LoadState::Unloaded)
    ));
    let events = assets.poll_pending_loads();
    assert!(events.iter().any(|event| matches!(
        event,
        AssetEvent::Cancelled { id, generation, .. }
            if *id == cancelled.id() && *generation == cancelled.generation()
    )));

    // A late worker completion belongs to the cancelled generation and must be ignored.
    for _ in 0..10_000 {
        let _ = assets.poll_pending_loads();
        if assets.worker_stats().active_workers == 0
            && assets.worker_stats().pending_completions == 0
        {
            break;
        }
        std::thread::yield_now();
    }
    assert!(assets.get(cancelled).is_none());
    assert!(matches!(
        assets.state::<TextAsset>("cancelled.txt"),
        Some(LoadState::Unloaded)
    ));

    let retry = assets.request::<TextAsset>("cancelled.txt").unwrap();
    assert!(retry.generation() > cancelled.generation());
    for _ in 0..100_000 {
        let _ = assets.poll_pending_loads();
        if assets.get(retry).is_some() {
            break;
        }
        std::thread::yield_now();
    }
    assert_eq!(assets.get(retry), Some(&TextAsset("discard me".to_owned())));
}

#[test]
fn completion_poll_respects_configured_per_frame_count_budget() {
    let source = MemorySource::default();
    source.insert("one.txt", b"one".to_vec());
    source.insert("two.txt", b"two".to_vec());
    source.insert("three.txt", b"three".to_vec());
    let mut assets = AssetServer::default();
    assets.add_source(0, source);
    assets.register_loader(TextLoader);
    assets.set_max_completions_per_poll(1);
    assets.set_max_completion_bytes_per_poll(None);
    let handles = [
        assets.request::<TextAsset>("one.txt").unwrap(),
        assets.request::<TextAsset>("two.txt").unwrap(),
        assets.request::<TextAsset>("three.txt").unwrap(),
    ];

    let mut maximum_processed = 0;
    for _ in 0..100_000 {
        let _ = assets.poll_pending_loads();
        maximum_processed =
            maximum_processed.max(assets.worker_stats().completions_processed_last_poll);
        if handles.iter().all(|handle| assets.get(*handle).is_some()) {
            break;
        }
        std::thread::yield_now();
    }
    assert_eq!(maximum_processed, 1);
    assert!(handles.iter().all(|handle| assets.get(*handle).is_some()));
}

#[test]
fn required_dependency_readiness_is_transitive_and_ignores_optional_edges() {
    let source = MemorySource::default();
    source.insert("required.bytes", vec![1, 2, 3]);
    source.insert("root.txt", b"root".to_vec());
    // The transitive dependency graph declares this leaf as a TextAsset. Make its
    // backing bytes available so the test exercises readiness rather than a missing file.
    source.insert("leaf.txt", b"leaf".to_vec());
    let mut assets = AssetServer::default();
    assets.add_source(0, source);
    assets
        .add_dependency::<TextAsset, BytesAsset>(
            "root.txt",
            "required.bytes",
            AssetDependencyKind::Required,
        )
        .unwrap();
    assets
        .add_dependency::<BytesAsset, TextAsset>(
            "required.bytes",
            "leaf.txt",
            AssetDependencyKind::Required,
        )
        .unwrap();
    assets
        .add_dependency::<TextAsset, BytesAsset>(
            "root.txt",
            "optional.bytes",
            AssetDependencyKind::Optional,
        )
        .unwrap();

    assert!(!assets
        .required_dependencies_ready::<TextAsset>("root.txt")
        .unwrap());
    let before_request = assets
        .required_dependency_statuses::<TextAsset>("root.txt")
        .unwrap();
    assert_eq!(before_request.len(), 2);
    assert!(before_request
        .iter()
        .all(|status| status.state == AssetDependencyLoadState::NotRequested));

    let leaf = assets.request::<TextAsset>("leaf.txt").unwrap();
    for _ in 0..100_000 {
        let _ = assets.poll_pending_loads();
        if assets.get(leaf).is_some() {
            break;
        }
        std::thread::yield_now();
    }
    assert!(assets.get(leaf).is_some());
    assert!(!assets
        .required_dependencies_ready::<TextAsset>("root.txt")
        .unwrap());

    let required = assets.request::<BytesAsset>("required.bytes").unwrap();
    for _ in 0..100_000 {
        let _ = assets.poll_pending_loads();
        if assets.get(required).is_some() {
            break;
        }
        std::thread::yield_now();
    }
    assert!(assets
        .required_dependencies_ready::<TextAsset>("root.txt")
        .unwrap());
}
