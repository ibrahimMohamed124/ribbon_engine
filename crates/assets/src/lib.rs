//! Asynchronous-capable asset sources, typed handles, loaders, and hot reload.
//!
//! [`Handle`] is a copyable weak identifier. [`AssetRef`] is an explicit strong lease that pins an
//! asset against [`AssetServer::evict_unused`]. Asset values remain owned by the server; leases
//! protect them from server-side eviction but do not outlive the server itself.
//!
//! File/source IO and CPU decoding for [`AssetServer::request`] run on worker threads. Results are
//! committed by [`AssetServer::poll_pending_loads`] on the thread that owns the server. Renderer
//! plugins must keep GPU resource creation/upload on their device-owning render thread.

mod dependencies;
mod handles;
pub use dependencies::{
    AssetDependency, AssetDependencyError, AssetDependencyGraph, AssetDependencyKey,
    AssetDependencyKind, AssetDependencyLoadState, AssetDependencyStatus,
};
pub use handles::{AssetId, Assets, Handle};
use std::any::{Any, TypeId};
use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex, RwLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Typed error kind for asset lookup, source access, routing, decoding, and worker failures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadErrorKind {
    /// No source contained the requested path.
    MissingSource,
    /// A source failed while reading the path.
    Io,
    /// No registered loader handles the path extension.
    NoLoader,
    /// The selected loader rejected the asset bytes.
    Decode,
    /// The extension's loader produces a different asset type.
    TypeMismatch,
    /// The worker pool could not be started or did not accept the request.
    WorkerUnavailable,
    /// The bounded asynchronous request queue is full; retry after completions are polled.
    QueueFull,
    /// A synchronous load was attempted while the path is already loading asynchronously.
    AlreadyLoading,
    /// A queued or running asynchronous load was cancelled before its result was committed.
    Cancelled,
}

/// Typed asset loading error with the path and a useful cause.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadError {
    /// Failure category.
    pub kind: LoadErrorKind,
    /// Requested asset path.
    pub path: String,
    /// Human-readable details.
    pub message: String,
}

impl LoadError {
    /// Create a loader error associated with the path being decoded.
    pub fn new(kind: LoadErrorKind, path: &str, message: impl Into<String>) -> Self {
        Self {
            kind,
            path: path.to_owned(),
            message: message.into(),
        }
    }
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "asset `{}`: {}", self.path, self.message)
    }
}

impl std::error::Error for LoadError {}

/// Cooperative cancellation signal shared by an asset request, its source read, and its loader.
/// Cancellation removes queued work immediately. Running work observes the signal between source
/// reads and after decoding; custom sources/loaders may override their cancellable methods to
/// check it more frequently. Arbitrary user code cannot be forcibly preempted safely.
#[derive(Clone, Debug, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    /// Mark the operation cancelled. Repeated calls are harmless.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    /// Whether cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    fn check(&self, path: &str) -> Result<(), LoadError> {
        if self.is_cancelled() {
            Err(LoadError::new(
                LoadErrorKind::Cancelled,
                path,
                "asset request was cancelled",
            ))
        } else {
            Ok(())
        }
    }
}

/// Marker bound for values managed by the asset server.
pub trait Asset: Any + Send + Sync + 'static {}

impl<T: Any + Send + Sync + 'static> Asset for T {}

struct AssetLease {
    key: (AssetId, u32),
    counts: Arc<Mutex<HashMap<(AssetId, u32), usize>>>,
}

impl Drop for AssetLease {
    fn drop(&mut self) {
        let Ok(mut counts) = self.counts.lock() else {
            // A poisoned count table is handled conservatively: eviction is disabled below.
            return;
        };
        let remove = if let Some(count) = counts.get_mut(&self.key) {
            *count = count.saturating_sub(1);
            *count == 0
        } else {
            false
        };
        if remove {
            counts.remove(&self.key);
        }
    }
}

/// Strong asset lease. Cloning shares one pin, released after the last clone drops.
///
/// The underlying value is still stored in [`AssetServer`]. Keep the server alive for as long as a
/// lease is used, and call [`AssetServer::evict_unused`] or enable its opt-in automatic policy to
/// release cached values whose strong lease count has reached zero.
pub struct AssetRef<T> {
    handle: Handle<T>,
    _lease: Arc<AssetLease>,
}

impl<T> AssetRef<T> {
    /// Return the weak handle used to query the asset server.
    pub fn handle(&self) -> Handle<T> {
        self.handle
    }

    /// Return this lease's stable asset identifier.
    pub fn asset_id(&self) -> AssetId {
        self.handle.asset_id()
    }

    /// Return this lease's generation.
    pub fn generation(&self) -> u32 {
        self.handle.generation()
    }
}

impl<T> Clone for AssetRef<T> {
    fn clone(&self) -> Self {
        Self {
            handle: self.handle,
            _lease: Arc::clone(&self._lease),
        }
    }
}

impl<T> fmt::Debug for AssetRef<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AssetRef")
            .field("handle", &self.handle)
            .finish_non_exhaustive()
    }
}

impl<T> PartialEq for AssetRef<T> {
    fn eq(&self, other: &Self) -> bool {
        self.handle == other.handle
    }
}

impl<T> Eq for AssetRef<T> {}

/// State of an asset path in the server cache.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadState {
    /// The request has started but has not finished.
    Loading,
    /// Asset bytes decoded successfully and a value is present in storage.
    Loaded,
    /// Source lookup, worker scheduling, or decoding failed.
    Failed(LoadError),
    /// The value was evicted or failed initially and can be requested with a new generation.
    Unloaded,
}

/// Source of asset bytes and optional modification timestamps.
pub trait AssetSource: Send + Sync + 'static {
    /// Read a path from this source.
    fn read(&self, path: &str) -> io::Result<Vec<u8>>;
    /// Read a path while cooperatively observing cancellation. Existing sources remain source
    /// compatible through this default implementation; long-running sources should override it.
    fn read_cancellable(
        &self,
        path: &str,
        cancellation: &CancellationToken,
    ) -> io::Result<Vec<u8>> {
        if cancellation.is_cancelled() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "asset read cancelled",
            ));
        }
        let bytes = self.read(path)?;
        if cancellation.is_cancelled() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "asset read cancelled",
            ));
        }
        Ok(bytes)
    }
    /// Return the last modification time when known.
    fn modified(&self, path: &str) -> Option<SystemTime>;
}

/// Filesystem-backed source rooted at one directory.
pub struct FsSource {
    root: PathBuf,
}

impl FsSource {
    /// Create a source rooted at `root`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn full_path(&self, path: &str) -> PathBuf {
        self.root.join(Path::new(path))
    }
}

impl AssetSource for FsSource {
    fn read(&self, path: &str) -> io::Result<Vec<u8>> {
        std::fs::read(self.full_path(path))
    }

    fn read_cancellable(
        &self,
        path: &str,
        cancellation: &CancellationToken,
    ) -> io::Result<Vec<u8>> {
        let mut file = std::fs::File::open(self.full_path(path))?;
        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 64 * 1024];
        loop {
            if cancellation.is_cancelled() {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "asset read cancelled",
                ));
            }
            let read = file.read(&mut chunk)?;
            if read == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..read]);
        }
        Ok(bytes)
    }

    fn modified(&self, path: &str) -> Option<SystemTime> {
        std::fs::metadata(self.full_path(path))
            .ok()?
            .modified()
            .ok()
    }
}

#[derive(Clone)]
struct MemoryEntry {
    bytes: Vec<u8>,
    modified: SystemTime,
}

/// In-memory source useful for embedded data and deterministic loading.
#[derive(Clone, Default)]
pub struct MemorySource {
    values: Arc<RwLock<HashMap<String, MemoryEntry>>>,
    clock: Arc<std::sync::atomic::AtomicU64>,
}

impl MemorySource {
    /// Insert or replace bytes, advancing this path's modification time.
    pub fn insert(&self, path: impl Into<String>, bytes: impl Into<Vec<u8>>) {
        let tick = self
            .clock
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            .saturating_add(1);
        let modified = UNIX_EPOCH + Duration::from_millis(tick);
        if let Ok(mut values) = self.values.write() {
            values.insert(
                path.into(),
                MemoryEntry {
                    bytes: bytes.into(),
                    modified,
                },
            );
        }
    }
}

impl AssetSource for MemorySource {
    fn read(&self, path: &str) -> io::Result<Vec<u8>> {
        self.values
            .read()
            .map_err(|_| io::Error::other("memory asset source lock was poisoned"))?
            .get(path)
            .map(|entry| entry.bytes.clone())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "asset path not found"))
    }

    fn modified(&self, path: &str) -> Option<SystemTime> {
        self.values
            .read()
            .ok()?
            .get(path)
            .map(|entry| entry.modified)
    }
}

/// Static source compiled into the application binary.
pub struct EmbeddedSource(pub &'static [(&'static str, &'static [u8])]);

impl AssetSource for EmbeddedSource {
    fn read(&self, path: &str) -> io::Result<Vec<u8>> {
        self.0
            .iter()
            .find(|(candidate, _)| *candidate == path)
            .map(|(_, bytes)| bytes.to_vec())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "asset path not embedded"))
    }

    fn modified(&self, _path: &str) -> Option<SystemTime> {
        None
    }
}

/// Per-load information supplied to an asset loader.
pub struct LoadContext {
    /// The source path currently being decoded.
    pub path: String,
}

/// Estimated decoded CPU residency split into content categories understood by the engine.
/// These values are estimates supplied by loaders, not allocator or operating-system telemetry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AssetMemoryBreakdown {
    /// CPU bytes retained by vertex/index/geometry payloads.
    pub geometry_bytes: u64,
    /// CPU bytes retained by decoded image pixels and mip payloads.
    pub texture_bytes: u64,
    /// CPU bytes retained by material records and material-specific data.
    pub material_bytes: u64,
    /// CPU bytes retained by animation clips, tracks, and frame data.
    pub animation_bytes: u64,
    /// CPU bytes retained by scene descriptions, node hierarchy, and metadata.
    pub scene_metadata_bytes: u64,
    /// CPU bytes not classified into a more specific category.
    pub other_bytes: u64,
}

impl AssetMemoryBreakdown {
    /// Total estimate across all categories.
    pub fn total_bytes(self) -> u64 {
        self.geometry_bytes
            .saturating_add(self.texture_bytes)
            .saturating_add(self.material_bytes)
            .saturating_add(self.animation_bytes)
            .saturating_add(self.scene_metadata_bytes)
            .saturating_add(self.other_bytes)
    }

    /// Put an aggregate-only loader estimate into the miscellaneous category.
    pub fn other(bytes: u64) -> Self {
        Self {
            other_bytes: bytes,
            ..Self::default()
        }
    }
}

/// Extension-based asset decoder. Implementations should perform CPU decoding only; GPU device
/// resources must be created later on the owning renderer thread.
pub trait AssetLoader: Send + Sync + 'static {
    /// Asset type produced by this loader.
    type Asset: Asset;
    /// File extensions supported by this loader, without a leading dot.
    fn extensions(&self) -> &[&str];
    /// Decode asset bytes.
    fn load(&self, bytes: &[u8], context: &mut LoadContext) -> Result<Self::Asset, LoadError>;
    /// Cooperative decode entry point. Existing loaders use the default pre/post checks; CPU-heavy
    /// loaders should override this method and poll `cancellation` inside their decode loops.
    fn load_cancellable(
        &self,
        bytes: &[u8],
        context: &mut LoadContext,
        cancellation: &CancellationToken,
    ) -> Result<Self::Asset, LoadError> {
        cancellation.check(&context.path)?;
        let asset = self.load(bytes, context)?;
        cancellation.check(&context.path)?;
        Ok(asset)
    }
    /// Estimate resident CPU bytes allocated by a decoded asset.
    ///
    /// The default accounts only for the fixed-size value. Loaders that own heap allocations
    /// should override this method and include their collection capacities, not just lengths.
    /// The estimate is used for cache-budget decisions and is not allocator telemetry.
    fn estimated_size_bytes(&self, _asset: &Self::Asset) -> u64 {
        std::mem::size_of::<Self::Asset>() as u64
    }
    /// Categorize the decoded bytes used by this asset. Aggregate-only loaders remain compatible
    /// and are counted as `other`; specialized loaders can report geometry, texture, material,
    /// animation, and scene metadata separately. The categories must sum to the same or a more
    /// complete estimate than `estimated_size_bytes`.
    fn memory_breakdown(&self, asset: &Self::Asset) -> AssetMemoryBreakdown {
        AssetMemoryBreakdown::other(self.estimated_size_bytes(asset))
    }
}

type ErasedAssetMap = HashMap<TypeId, Box<dyn Any + Send + Sync>>;
type AssetRemover = fn(&mut ErasedAssetMap, AssetId, u32) -> bool;

trait ErasedLoader: Send + Sync {
    fn asset_type(&self) -> TypeId;
    fn load(
        &self,
        bytes: &[u8],
        context: &mut LoadContext,
    ) -> Result<Box<dyn Any + Send + Sync>, LoadError>;
    fn load_cancellable(
        &self,
        bytes: &[u8],
        context: &mut LoadContext,
        cancellation: &CancellationToken,
    ) -> Result<Box<dyn Any + Send + Sync>, LoadError>;
    fn estimated_size_bytes(&self, value: &(dyn Any + Send + Sync)) -> u64;
    fn memory_breakdown(&self, value: &(dyn Any + Send + Sync)) -> AssetMemoryBreakdown;
    fn store(
        &self,
        assets: &mut ErasedAssetMap,
        id: AssetId,
        generation: u32,
        value: Box<dyn Any + Send + Sync>,
    ) -> Result<(), LoadError>;
}

struct LoaderAdapter<L>(L);

impl<L: AssetLoader> ErasedLoader for LoaderAdapter<L> {
    fn asset_type(&self) -> TypeId {
        TypeId::of::<L::Asset>()
    }

    fn load(
        &self,
        bytes: &[u8],
        context: &mut LoadContext,
    ) -> Result<Box<dyn Any + Send + Sync>, LoadError> {
        self.0
            .load(bytes, context)
            .map(|asset| Box::new(asset) as Box<dyn Any + Send + Sync>)
    }

    fn load_cancellable(
        &self,
        bytes: &[u8],
        context: &mut LoadContext,
        cancellation: &CancellationToken,
    ) -> Result<Box<dyn Any + Send + Sync>, LoadError> {
        self.0
            .load_cancellable(bytes, context, cancellation)
            .map(|asset| Box::new(asset) as Box<dyn Any + Send + Sync>)
    }

    fn estimated_size_bytes(&self, value: &(dyn Any + Send + Sync)) -> u64 {
        value
            .downcast_ref::<L::Asset>()
            .map(|asset| self.0.estimated_size_bytes(asset))
            .unwrap_or(std::mem::size_of::<L::Asset>() as u64)
    }

    fn memory_breakdown(&self, value: &(dyn Any + Send + Sync)) -> AssetMemoryBreakdown {
        let Some(asset) = value.downcast_ref::<L::Asset>() else {
            return AssetMemoryBreakdown::other(std::mem::size_of::<L::Asset>() as u64);
        };
        let mut breakdown = self.0.memory_breakdown(asset);
        let estimated = self.0.estimated_size_bytes(asset);
        let classified = breakdown.total_bytes();
        if classified < estimated {
            breakdown.other_bytes = breakdown
                .other_bytes
                .saturating_add(estimated.saturating_sub(classified));
        }
        breakdown
    }

    fn store(
        &self,
        assets: &mut ErasedAssetMap,
        id: AssetId,
        generation: u32,
        value: Box<dyn Any + Send + Sync>,
    ) -> Result<(), LoadError> {
        let asset = value.downcast::<L::Asset>().map_err(|_| {
            LoadError::new(
                LoadErrorKind::TypeMismatch,
                "<asset storage>",
                "loader returned a value with the wrong concrete type",
            )
        })?;
        let storage = assets
            .entry(TypeId::of::<L::Asset>())
            .or_insert_with(|| Box::new(Assets::<L::Asset>::default()));
        let typed = storage.downcast_mut::<Assets<L::Asset>>().ok_or_else(|| {
            LoadError::new(
                LoadErrorKind::TypeMismatch,
                "<asset storage>",
                "asset storage has an unexpected concrete type",
            )
        })?;
        typed.insert(Handle::new(id, generation), *asset);
        Ok(())
    }
}

struct SourceEntry {
    id: u64,
    priority: i32,
    source: Arc<dyn AssetSource>,
}

#[derive(Clone)]
struct SourceSnapshot {
    id: u64,
    source: Arc<dyn AssetSource>,
}

struct PathEntry {
    id: AssetId,
    generation: u32,
    source_id: Option<u64>,
    modified: Option<SystemTime>,
    extension: String,
    state: LoadState,
    reload_pending: bool,
    reload_token: u64,
    estimated_size_bytes: u64,
    memory_breakdown: AssetMemoryBreakdown,
    load_sequence: u64,
    priority: i32,
    cancellation: CancellationToken,
}

/// Snapshot of the asset server's decoded CPU-side cache residency.
///
/// All byte values are loader-provided estimates. The budget applies to unleased cache entries;
/// assets held by at least one [`AssetRef`] remain resident even when that makes total residency
/// exceed the budget.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AssetMemoryStats {
    /// Number of successfully decoded values currently resident.
    pub loaded_assets: usize,
    /// Resident CPU bytes across both leased and unleased values.
    pub resident_bytes_estimate: u64,
    /// Categorized resident CPU estimates across the current cache.
    /// Estimated geometry bytes across resident cache entries.
    pub geometry_bytes_estimate: u64,
    /// Estimated decoded texture bytes across resident cache entries.
    pub texture_bytes_estimate: u64,
    /// Estimated material bytes across resident cache entries.
    pub material_bytes_estimate: u64,
    /// Estimated animation bytes across resident cache entries.
    pub animation_bytes_estimate: u64,
    /// Estimated scene metadata bytes across resident cache entries.
    pub scene_metadata_bytes_estimate: u64,
    /// Estimated unclassified bytes across resident cache entries.
    pub other_bytes_estimate: u64,
    /// Resident CPU bytes protected by at least one strong lease.
    pub pinned_bytes_estimate: u64,
    /// Resident CPU bytes that are eligible for cache eviction.
    pub unpinned_bytes_estimate: u64,
    /// Number of resident values protected by one or more strong leases.
    pub pinned_assets: usize,
    /// Configured unpinned-cache budget, or `None` when budget eviction is disabled.
    pub budget_bytes: Option<u64>,
    /// Unpinned resident bytes in excess of the configured budget.
    pub over_budget_bytes: u64,
    /// Cumulative values evicted specifically to satisfy the configured cache budget.
    pub budget_evictions_cumulative: u64,
}

/// Asset lifecycle and reload notifications delivered on the server-owning thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssetEvent {
    /// An asset request was accepted and began a new loading generation.
    Loading {
        /// Stable numeric ID.
        id: u64,
        /// Current generation.
        generation: u32,
        /// Requested source path.
        path: String,
    },
    /// An asset finished loading successfully.
    Loaded {
        /// Stable numeric ID.
        id: u64,
        /// Current generation.
        generation: u32,
        /// Requested source path.
        path: String,
    },
    /// An initial load failed. The exact error is also available from [`AssetServer::state`].
    Failed {
        /// Stable numeric ID.
        id: u64,
        /// Current generation.
        generation: u32,
        /// Requested source path.
        path: String,
        /// Source, worker, or decode error.
        error: LoadError,
    },
    /// A queued or in-flight initial load was cancelled before it could be committed.
    Cancelled {
        /// Stable numeric ID.
        id: u64,
        /// Generation that was cancelled.
        generation: u32,
        /// Requested source path.
        path: String,
    },
    /// The cached value was evicted after its strong lease count reached zero.
    Unloaded {
        /// Stable numeric ID.
        id: u64,
        /// Generation that was evicted.
        generation: u32,
        /// Requested source path.
        path: String,
    },
    /// Contents changed while the weak handle stayed the same.
    Modified { id: u64 },
    /// A hot reload failed; the previous successfully decoded value remains available.
    ReloadFailed {
        /// Stable numeric ID.
        id: u64,
        /// Generation of the retained last-good value.
        generation: u32,
        /// Source path.
        path: String,
        /// Reload error.
        error: LoadError,
    },
}

struct DecodedAsset {
    loader: Arc<dyn ErasedLoader>,
    value: Box<dyn Any + Send + Sync>,
    source_id: u64,
    modified: Option<SystemTime>,
    estimated_size_bytes: u64,
    memory_breakdown: AssetMemoryBreakdown,
}

#[derive(Clone)]
struct ReloadRequest {
    token: u64,
    observed_modified: Option<SystemTime>,
}

struct LoadCompletion {
    type_id: TypeId,
    path: String,
    id: AssetId,
    generation: u32,
    reload: Option<ReloadRequest>,
    source_read_time_ns: u64,
    decode_time_ns: u64,
    result: Result<DecodedAsset, LoadError>,
}

type WorkerTask = Box<dyn FnOnce() + Send + 'static>;

// Bound both queued source/decode work and decoded-but-not-yet-committed assets. The worker
// channel and main-thread staging queue each hold at most MAX_PENDING_ASSET_COMPLETIONS results.
const MAX_QUEUED_ASSET_LOADS: usize = 256;
const MAX_PENDING_ASSET_COMPLETIONS: usize = 64;
const ASSET_PRIORITY_AGE_INTERVAL: Duration = Duration::from_secs(2);

type WorkerJobId = (u64, u32);

struct QueuedWorkerTask {
    id: WorkerJobId,
    priority: i32,
    sequence: u64,
    queued_at: Instant,
    cancellation: CancellationToken,
    task: WorkerTask,
}

#[derive(Default)]
struct WorkerQueueState {
    tasks: Vec<QueuedWorkerTask>,
    active_cancellations: HashMap<WorkerJobId, CancellationToken>,
    next_sequence: u64,
    active_workers: usize,
    closed: bool,
}

struct WorkerQueue {
    state: Mutex<WorkerQueueState>,
    ready: Condvar,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AssetWorkerStats {
    /// Number of source/decode jobs waiting to start.
    pub queued_jobs: usize,
    /// Number of worker threads currently executing a job.
    pub active_workers: usize,
    /// Number of worker threads owned by this server (configured count before startup).
    pub worker_count: usize,
    /// Maximum number of jobs waiting to start.
    pub queue_capacity: usize,
    /// Maximum number of decoded values waiting for main-thread commit.
    pub completion_capacity: usize,
    /// Decoded values currently staged on the AssetServer owner thread awaiting a later commit.
    /// Additional completions may still be waiting in the bounded worker-to-main channel.
    pub pending_completions: usize,
    /// Number of completion records processed (including stale/failed records) by the most
    /// recent poll.
    pub completions_processed_last_poll: usize,
    /// Total worker source-read time represented by results processed in the most recent poll.
    pub source_read_time_ns_last_poll: u64,
    /// Total worker decode time represented by results processed in the most recent poll.
    pub decode_time_ns_last_poll: u64,
    /// Cumulative worker source-read time represented by results processed by this server.
    pub source_read_time_ns_total: u64,
    /// Cumulative worker decode time represented by results processed by this server.
    pub decode_time_ns_total: u64,
    /// Per-poll completion count limit.
    pub max_completions_per_poll: usize,
    /// Per-poll decoded byte limit; `None` means unbounded by bytes.
    pub max_completion_bytes_per_poll: Option<u64>,
}

enum WorkerSubmitError {
    Full,
    Disconnected,
}

struct WorkerPool {
    queue: Arc<WorkerQueue>,
    worker_count: usize,
    // Dropping JoinHandles detaches workers that are already inside user code. Drop cancels both
    // queued and active tokens, closes the queue, and wakes idle workers; non-cooperative user code
    // may still finish later, but its completion sender is disconnected and its result is dropped.
    _workers: Vec<JoinHandle<()>>,
}

fn effective_asset_priority(task: &QueuedWorkerTask, now: Instant) -> i32 {
    let age_steps = now.saturating_duration_since(task.queued_at).as_secs()
        / ASSET_PRIORITY_AGE_INTERVAL.as_secs().max(1);
    task.priority
        .saturating_add(age_steps.min(i32::MAX as u64) as i32)
}

fn next_asset_task_index(tasks: &[QueuedWorkerTask], now: Instant) -> Option<usize> {
    tasks
        .iter()
        .enumerate()
        .max_by(|(_, left), (_, right)| {
            effective_asset_priority(left, now)
                .cmp(&effective_asset_priority(right, now))
                // Lower sequence means older FIFO request and wins ties.
                .then_with(|| right.sequence.cmp(&left.sequence))
        })
        .map(|(index, _)| index)
}

impl WorkerPool {
    fn new(worker_count: usize) -> io::Result<Self> {
        let queue = Arc::new(WorkerQueue {
            state: Mutex::new(WorkerQueueState::default()),
            ready: Condvar::new(),
        });
        let mut workers: Vec<JoinHandle<()>> = Vec::with_capacity(worker_count.max(1));
        for index in 0..worker_count.max(1) {
            let worker_queue = Arc::clone(&queue);
            let worker = match thread::Builder::new()
                .name(format!("ribbon-assets-{index}"))
                .spawn(move || loop {
                    let task = {
                        let mut state = match worker_queue.state.lock() {
                            Ok(state) => state,
                            Err(_) => return,
                        };
                        loop {
                            if let Some(index) = next_asset_task_index(&state.tasks, Instant::now())
                            {
                                let queued = state.tasks.swap_remove(index);
                                state.active_workers = state.active_workers.saturating_add(1);
                                state
                                    .active_cancellations
                                    .insert(queued.id, queued.cancellation.clone());
                                break Some((queued.id, queued.cancellation, queued.task));
                            }
                            if state.closed {
                                break None;
                            }
                            state = match worker_queue.ready.wait(state) {
                                Ok(state) => state,
                                Err(_) => return,
                            };
                        }
                    };
                    let Some((task_id, cancellation, task)) = task else {
                        break;
                    };
                    // A queued cancellation avoids invoking source/loader user code at all.
                    // Running tasks receive the same token and cooperate at safe boundaries.
                    // A faulty user loader must not permanently kill a worker or leave the
                    // active-worker diagnostic counter elevated. The load task itself reports
                    // normal decoder errors through its completion channel.
                    if !cancellation.is_cancelled() {
                        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(task));
                    }
                    if let Ok(mut state) = worker_queue.state.lock() {
                        state.active_workers = state.active_workers.saturating_sub(1);
                        state.active_cancellations.remove(&task_id);
                    } else {
                        return;
                    }
                }) {
                Ok(worker) => worker,
                Err(error) => {
                    // Do not leak already-started idle workers if a later thread cannot be
                    // created. Close and wake the partial pool before joining its workers.
                    if let Ok(mut state) = queue.state.lock() {
                        state.closed = true;
                    }
                    queue.ready.notify_all();
                    for worker in workers.drain(..) {
                        let _ = worker.join();
                    }
                    return Err(error);
                }
            };
            workers.push(worker);
        }
        Ok(Self {
            queue,
            worker_count: worker_count.max(1),
            _workers: workers,
        })
    }

    fn execute(
        &self,
        id: WorkerJobId,
        priority: i32,
        cancellation: CancellationToken,
        task: WorkerTask,
    ) -> Result<(), WorkerSubmitError> {
        let Ok(mut state) = self.queue.state.lock() else {
            return Err(WorkerSubmitError::Disconnected);
        };
        if state.closed {
            return Err(WorkerSubmitError::Disconnected);
        }
        if state.tasks.len() >= MAX_QUEUED_ASSET_LOADS {
            return Err(WorkerSubmitError::Full);
        }
        let sequence = state.next_sequence;
        state.next_sequence = state.next_sequence.saturating_add(1);
        state.tasks.push(QueuedWorkerTask {
            id,
            priority,
            sequence,
            queued_at: Instant::now(),
            cancellation,
            task,
        });
        drop(state);
        self.queue.ready.notify_one();
        Ok(())
    }

    /// Raise the priority of a queued operation. Running operations are not preempted.
    fn set_priority(&self, id: WorkerJobId, priority: i32) -> bool {
        let Ok(mut state) = self.queue.state.lock() else {
            return false;
        };
        let Some(task) = state.tasks.iter_mut().find(|task| task.id == id) else {
            return false;
        };
        if priority <= task.priority {
            return false;
        }
        task.priority = priority;
        drop(state);
        self.queue.ready.notify_all();
        true
    }

    /// Remove a queued operation without running its closure. Running work cooperates via token.
    fn cancel(&self, id: WorkerJobId) -> bool {
        let Ok(mut state) = self.queue.state.lock() else {
            return false;
        };
        let Some(index) = state.tasks.iter().position(|task| task.id == id) else {
            return false;
        };
        let task = state.tasks.swap_remove(index);
        task.cancellation.cancel();
        true
    }

    fn stats(&self) -> (usize, usize) {
        self.queue
            .state
            .lock()
            .map(|state| (state.tasks.len(), state.active_workers))
            .unwrap_or((0, self.worker_count))
    }
}

impl Drop for WorkerPool {
    fn drop(&mut self) {
        if let Ok(mut state) = self.queue.state.lock() {
            state.closed = true;
            for queued in state.tasks.drain(..) {
                queued.cancellation.cancel();
            }
            for cancellation in state.active_cancellations.values() {
                cancellation.cancel();
            }
        }
        self.queue.ready.notify_all();
    }
}

mod server;
pub use server::AssetServer;

mod helpers;
use helpers::{
    extension_for, read_from_sources, read_from_sources_cancellable, remove_asset_from_map,
};

mod loaders;
pub use loaders::{BytesAsset, BytesLoader, TextAsset, TextLoader};
mod plugin;
pub use plugin::AssetPlugin;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod asset_priority_tests {
    use super::{effective_asset_priority, next_asset_task_index, QueuedWorkerTask, WorkerTask};
    use std::time::{Duration, Instant};

    fn job(priority: i32, sequence: u64, queued_at: Instant) -> QueuedWorkerTask {
        let task: WorkerTask = Box::new(|| {});
        QueuedWorkerTask {
            id: (sequence, 1),
            priority,
            sequence,
            queued_at,
            cancellation: super::CancellationToken::default(),
            task,
        }
    }

    #[test]
    fn queued_assets_are_ordered_by_priority_then_fifo() {
        let now = Instant::now();
        let tasks = vec![job(1, 0, now), job(5, 1, now), job(5, 2, now)];
        assert_eq!(next_asset_task_index(&tasks, now), Some(1));
    }

    #[test]
    fn low_priority_jobs_gain_age_priority_and_do_not_starve_forever() {
        let now = Instant::now();
        let old = job(0, 0, now - Duration::from_secs(30));
        let new = job(10, 1, now);
        assert!(effective_asset_priority(&old, now) > effective_asset_priority(&new, now));
        assert_eq!(next_asset_task_index(&[new, old], now), Some(1));
    }
}
