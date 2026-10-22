//! Asynchronous-capable asset sources, typed handles, loaders, and hot reload.
//!
//! [`Handle`] is a copyable weak identifier. [`AssetRef`] is an explicit strong lease that pins an
//! asset against [`AssetServer::evict_unused`]. Asset values remain owned by the server; leases
//! protect them from server-side eviction but do not outlive the server itself.
//!
//! File/source IO and CPU decoding for [`AssetServer::request`] run on worker threads. Results are
//! committed by [`AssetServer::poll_pending_loads`] on the thread that owns the server. Renderer
//! plugins must keep GPU resource creation/upload on their device-owning render thread.

use engine_app::{App, Stage, World};
use engine_events::WorldEventsExt;
use engine_plugins::Plugin;
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::fmt;
use std::io;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex, RwLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
#[cfg(any(feature = "hot_reload", debug_assertions))]
use std::time::Instant;

/// Opaque identity shared by typed views of one asset path.
///
/// The numeric identity is never reused by an [`AssetServer`]. A path reloaded after eviction keeps
/// this identity while its generation advances, making handles from earlier generations stale.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AssetId(u64);

impl AssetId {
    /// Return the stable numeric identity, independent of its current generation.
    pub const fn get(self) -> u64 {
        self.0
    }
}

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

/// Marker bound for values managed by the asset server.
pub trait Asset: Any + Send + Sync + 'static {}

impl<T: Any + Send + Sync + 'static> Asset for T {}

/// Copyable weak typed identifier. It does not pin the value against eviction.
pub struct Handle<T> {
    id: AssetId,
    generation: u32,
    marker: PhantomData<fn() -> T>,
}

impl<T> Handle<T> {
    fn new(id: AssetId, generation: u32) -> Self {
        Self {
            id,
            generation,
            marker: PhantomData,
        }
    }

    /// Stable numeric identifier, retained for compatibility with existing renderer caches.
    pub fn id(self) -> u64 {
        self.id.get()
    }

    /// Typed identity, including the stable asset ID but not its generation.
    pub fn asset_id(self) -> AssetId {
        self.id
    }

    /// Generation checked when retrieving the asset.
    pub fn generation(self) -> u32 {
        self.generation
    }
}

impl<T> Copy for Handle<T> {}

impl<T> Clone for Handle<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> fmt::Debug for Handle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Handle")
            .field("id", &self.id)
            .field("generation", &self.generation)
            .finish()
    }
}

impl<T> PartialEq for Handle<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.generation == other.generation
    }
}

impl<T> Eq for Handle<T> {}

impl<T> std::hash::Hash for Handle<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::hash::Hash::hash(&self.id, state);
        std::hash::Hash::hash(&self.generation, state);
    }
}

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

/// Typed storage indexed by weak handles.
pub struct Assets<T> {
    values: HashMap<u64, (u32, T)>,
}

impl<T> Default for Assets<T> {
    fn default() -> Self {
        Self {
            values: HashMap::new(),
        }
    }
}

impl<T> Assets<T> {
    /// Get a value when both the identifier and generation match.
    pub fn get(&self, handle: Handle<T>) -> Option<&T> {
        let (generation, value) = self.values.get(&handle.id.get())?;
        (*generation == handle.generation).then_some(value)
    }

    /// Mutably get a value when the identifier and generation match.
    pub fn get_mut(&mut self, handle: Handle<T>) -> Option<&mut T> {
        let (generation, value) = self.values.get_mut(&handle.id.get())?;
        (*generation == handle.generation).then_some(value)
    }

    /// Number of values currently stored.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Whether the storage is empty.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    fn insert(&mut self, handle: Handle<T>, value: T) {
        self.values
            .insert(handle.id.get(), (handle.generation, value));
    }

    fn remove(&mut self, handle: Handle<T>) -> bool {
        if self
            .values
            .get(&handle.id.get())
            .is_some_and(|(generation, _)| *generation == handle.generation)
        {
            self.values.remove(&handle.id.get());
            true
        } else {
            false
        }
    }
}

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

/// Extension-based asset decoder. Implementations should perform CPU decoding only; GPU device
/// resources must be created later on the owning renderer thread.
pub trait AssetLoader: Send + Sync + 'static {
    /// Asset type produced by this loader.
    type Asset: Asset;
    /// File extensions supported by this loader, without a leading dot.
    fn extensions(&self) -> &[&str];
    /// Decode asset bytes.
    fn load(&self, bytes: &[u8], context: &mut LoadContext) -> Result<Self::Asset, LoadError>;
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
    result: Result<DecodedAsset, LoadError>,
}

type WorkerTask = Box<dyn FnOnce() + Send + 'static>;

// Bound both queued source/decode work and decoded-but-not-yet-committed assets. This prevents a
// producer that outruns the frame loop from retaining an unbounded amount of work or decoded data.
const MAX_QUEUED_ASSET_LOADS: usize = 256;
const MAX_PENDING_ASSET_COMPLETIONS: usize = 64;

struct WorkerPool {
    sender: mpsc::SyncSender<WorkerTask>,
    // Dropping JoinHandles detaches workers; dropping sender wakes their receivers so they exit.
    _workers: Vec<JoinHandle<()>>,
}

impl WorkerPool {
    fn new(worker_count: usize) -> io::Result<Self> {
        let (sender, receiver) = mpsc::sync_channel::<WorkerTask>(MAX_QUEUED_ASSET_LOADS);
        let receiver = Arc::new(Mutex::new(receiver));
        let mut workers = Vec::with_capacity(worker_count);
        for index in 0..worker_count.max(1) {
            let receiver = Arc::clone(&receiver);
            let worker = thread::Builder::new()
                .name(format!("ribbon-assets-{index}"))
                .spawn(move || loop {
                    let task = match receiver.lock() {
                        Ok(receiver) => receiver.recv(),
                        Err(_) => return,
                    };
                    match task {
                        Ok(task) => task(),
                        Err(_) => break,
                    }
                })?;
            workers.push(worker);
        }
        Ok(Self {
            sender,
            _workers: workers,
        })
    }

    fn execute(&self, task: WorkerTask) -> Result<(), mpsc::TrySendError<WorkerTask>> {
        self.sender.try_send(task)
    }
}

/// Priority-ordered asset server with synchronous compatibility and worker-backed streaming.
pub struct AssetServer {
    sources: Vec<SourceEntry>,
    loaders: HashMap<String, Arc<dyn ErasedLoader>>,
    asset_removers: HashMap<TypeId, AssetRemover>,
    assets: ErasedAssetMap,
    revisions: HashMap<(AssetId, u32), u64>,
    paths: HashMap<(TypeId, String), PathEntry>,
    next_id: u64,
    next_source_id: u64,
    #[cfg(any(feature = "hot_reload", debug_assertions))]
    last_poll: Instant,
    poll_interval: Duration,
    workers: Option<WorkerPool>,
    worker_count: usize,
    completion_tx: mpsc::SyncSender<LoadCompletion>,
    completion_rx: Mutex<mpsc::Receiver<LoadCompletion>>,
    pending_events: Vec<AssetEvent>,
    strong_refs: Arc<Mutex<HashMap<(AssetId, u32), usize>>>,
    auto_evict_unused: bool,
    async_hot_reload: bool,
    last_reload_errors: HashMap<(AssetId, u32), LoadError>,
}

impl Default for AssetServer {
    fn default() -> Self {
        let (completion_tx, completion_rx) =
            mpsc::sync_channel::<LoadCompletion>(MAX_PENDING_ASSET_COMPLETIONS);
        let worker_count = thread::available_parallelism()
            .map_or(2, |count| count.get())
            .clamp(1, 4);
        let mut server = Self {
            sources: Vec::new(),
            loaders: HashMap::new(),
            asset_removers: HashMap::new(),
            assets: HashMap::new(),
            revisions: HashMap::new(),
            paths: HashMap::new(),
            next_id: 1,
            next_source_id: 1,
            #[cfg(any(feature = "hot_reload", debug_assertions))]
            last_poll: Instant::now(),
            poll_interval: Duration::from_millis(500),
            workers: None,
            worker_count,
            completion_tx,
            completion_rx: Mutex::new(completion_rx),
            pending_events: Vec::new(),
            strong_refs: Arc::new(Mutex::new(HashMap::new())),
            auto_evict_unused: false,
            async_hot_reload: false,
            last_reload_errors: HashMap::new(),
        };
        server.register_loader(BytesLoader);
        server.register_loader(TextLoader);
        server
    }
}

impl AssetServer {
    /// Add a source. Higher priorities are consulted first; equal priorities keep insertion order.
    pub fn add_source(&mut self, priority: i32, source: impl AssetSource) {
        let id = self.next_source_id;
        self.next_source_id = self
            .next_source_id
            .checked_add(1)
            .expect("asset source ID space exhausted");
        let index = self
            .sources
            .iter()
            .position(|entry| priority > entry.priority)
            .unwrap_or(self.sources.len());
        self.sources.insert(
            index,
            SourceEntry {
                id,
                priority,
                source: Arc::new(source),
            },
        );
    }

    /// Register a loader for each declared extension.
    pub fn register_loader<L: AssetLoader>(&mut self, loader: L) {
        let extensions: Vec<_> = loader
            .extensions()
            .iter()
            .map(|extension| extension.trim_start_matches('.').to_ascii_lowercase())
            .collect();
        let loader = Arc::new(LoaderAdapter(loader)) as Arc<dyn ErasedLoader>;
        self.asset_removers
            .entry(TypeId::of::<L::Asset>())
            .or_insert(remove_asset_from_map::<L::Asset>);
        for extension in extensions {
            self.loaders.insert(extension, Arc::clone(&loader));
        }
    }

    /// Set the interval used by polling hot reload.
    pub fn set_poll_interval(&mut self, interval: Duration) {
        self.poll_interval = interval;
    }

    /// Set the worker count before the first asynchronous request. The count is clamped to 1..=32.
    /// Calls made after the worker pool starts only affect a future server instance.
    pub fn set_worker_count(&mut self, count: usize) {
        if self.workers.is_none() {
            self.worker_count = count.clamp(1, 32);
        }
    }

    /// Enable or disable worker-backed hot-reload IO/decode. `request` enables this automatically;
    /// synchronous-only callers retain immediate reload behavior unless they opt in.
    pub fn set_async_hot_reload(&mut self, enabled: bool) {
        self.async_hot_reload = enabled;
    }

    /// Enable automatic eviction during [`poll_pending_loads`](Self::poll_pending_loads).
    ///
    /// This is opt-in because legacy [`Handle`] values are weak and do not pin their assets. When
    /// enabled, callers should retain an [`AssetRef`] for every asset that must remain resident.
    pub fn set_auto_evict_unused(&mut self, enabled: bool) {
        self.auto_evict_unused = enabled;
    }

    /// Load synchronously and cache by asset type and path.
    ///
    /// Prefer [`request`](Self::request) for assets that should not block the calling thread.
    pub fn load<T: Asset>(&mut self, path: impl AsRef<str>) -> Result<Handle<T>, LoadError> {
        let path = path.as_ref().to_owned();
        let key = (TypeId::of::<T>(), path.clone());
        if let Some(entry) = self.paths.get(&key) {
            match &entry.state {
                LoadState::Loaded => return Ok(Handle::new(entry.id, entry.generation)),
                LoadState::Loading => {
                    return Err(LoadError::new(
                        LoadErrorKind::AlreadyLoading,
                        &path,
                        "asset is already loading asynchronously; poll the server and retry",
                    ));
                }
                LoadState::Failed(error) => return Err(error.clone()),
                LoadState::Unloaded => {}
            }
        }

        let extension = extension_for(&path);
        let (id, generation) = self.begin_loading(&key, &extension);
        let result = self
            .load_bytes(&path)
            .and_then(|(bytes, source_id, modified)| {
                let loader = self.loaders.get(&extension).cloned().ok_or_else(|| {
                    LoadError::new(
                        LoadErrorKind::NoLoader,
                        &path,
                        format!("no loader registered for extension `{extension}`"),
                    )
                })?;
                if loader.asset_type() != TypeId::of::<T>() {
                    return Err(LoadError::new(
                        LoadErrorKind::TypeMismatch,
                        &path,
                        "extension loader produces a different asset type",
                    ));
                }
                let mut context = LoadContext { path: path.clone() };
                let value = loader.load(&bytes, &mut context)?;
                let value = value.downcast::<T>().map_err(|_| {
                    LoadError::new(
                        LoadErrorKind::TypeMismatch,
                        &path,
                        "loader returned a value with the wrong concrete type",
                    )
                })?;
                self.storage_mut::<T>()
                    .insert(Handle::new(id, generation), *value);
                self.revisions.insert((id, generation), 1);
                if let Some(entry) = self.paths.get_mut(&key) {
                    entry.source_id = Some(source_id);
                    entry.modified = modified;
                    entry.state = LoadState::Loaded;
                    entry.reload_pending = false;
                }
                self.last_reload_errors.remove(&(id, generation));
                self.pending_events.push(AssetEvent::Loaded {
                    id: id.get(),
                    generation,
                    path: path.clone(),
                });
                Ok(Handle::new(id, generation))
            });
        if let Err(error) = &result {
            if let Some(entry) = self.paths.get_mut(&key) {
                entry.state = LoadState::Failed(error.clone());
                entry.reload_pending = false;
            }
            self.pending_events.push(AssetEvent::Failed {
                id: id.get(),
                generation,
                path: path.clone(),
                error: error.clone(),
            });
        }
        result
    }

    /// Queue source IO/CPU decode on workers and return a weak handle immediately. This also opts
    /// the server into worker-backed hot reload so subsequent edits do not block the frame loop.
    ///
    /// Repeated requests for the same typed path share the same in-flight operation. The handle can
    /// be polled with [`get`](Self::get) and [`state`](Self::state); it becomes readable once
    /// [`poll_pending_loads`](Self::poll_pending_loads) commits the worker result.
    pub fn request<T: Asset>(&mut self, path: impl AsRef<str>) -> Result<Handle<T>, LoadError> {
        self.async_hot_reload = true;
        let path = path.as_ref().to_owned();
        let key = (TypeId::of::<T>(), path.clone());
        if let Some(entry) = self.paths.get(&key) {
            match &entry.state {
                LoadState::Loaded | LoadState::Loading => {
                    return Ok(Handle::new(entry.id, entry.generation));
                }
                LoadState::Failed(error) => return Err(error.clone()),
                LoadState::Unloaded => {}
            }
        }

        let extension = extension_for(&path);
        let loader = match self.loaders.get(&extension).cloned() {
            Some(loader) if loader.asset_type() == TypeId::of::<T>() => loader,
            Some(_) => {
                let error = LoadError::new(
                    LoadErrorKind::TypeMismatch,
                    &path,
                    "extension loader produces a different asset type",
                );
                return Err(self.record_request_failure(&key, &extension, error));
            }
            None => {
                let error = LoadError::new(
                    LoadErrorKind::NoLoader,
                    &path,
                    format!("no loader registered for extension `{extension}`"),
                );
                return Err(self.record_request_failure(&key, &extension, error));
            }
        };

        let (id, generation) = self.begin_loading(&key, &extension);
        if self.workers.is_none() {
            match WorkerPool::new(self.worker_count) {
                Ok(pool) => self.workers = Some(pool),
                Err(error) => {
                    let load_error = LoadError::new(
                        LoadErrorKind::WorkerUnavailable,
                        &path,
                        format!("could not start asset worker pool: {error}"),
                    );
                    self.fail_path(&key, id, generation, &load_error);
                    return Err(load_error);
                }
            }
        }

        let sources: Vec<_> = self
            .sources
            .iter()
            .map(|source| SourceSnapshot {
                id: source.id,
                source: Arc::clone(&source.source),
            })
            .collect();
        let completion_tx = self.completion_tx.clone();
        let request_path = path.clone();
        let request_loader = Arc::clone(&loader);
        let type_id = TypeId::of::<T>();
        let task: WorkerTask = Box::new(move || {
            let result = read_from_sources(&sources, &request_path).and_then(
                |(bytes, source_id, modified)| {
                    let mut context = LoadContext {
                        path: request_path.clone(),
                    };
                    request_loader
                        .load(&bytes, &mut context)
                        .map(|value| DecodedAsset {
                            loader: Arc::clone(&request_loader),
                            value,
                            source_id,
                            modified,
                        })
                },
            );
            let _ = completion_tx.send(LoadCompletion {
                type_id,
                path: request_path,
                id,
                generation,
                reload: None,
                result,
            });
        });
        let queue_result = self.workers.as_ref().map(|workers| workers.execute(task));
        match queue_result {
            Some(Ok(())) => {}
            Some(Err(mpsc::TrySendError::Full(_))) => {
                let error = LoadError::new(
                    LoadErrorKind::QueueFull,
                    &path,
                    format!(
                        "asset request queue is full (capacity {MAX_QUEUED_ASSET_LOADS}); \
                         retry after pending loads are polled"
                    ),
                );
                self.fail_path(&key, id, generation, &error);
                return Err(error);
            }
            Some(Err(mpsc::TrySendError::Disconnected(_))) => {
                self.workers = None;
                let error = LoadError::new(
                    LoadErrorKind::WorkerUnavailable,
                    &path,
                    "asset worker queue is unavailable",
                );
                self.fail_path(&key, id, generation, &error);
                return Err(error);
            }
            None => {
                let error = LoadError::new(
                    LoadErrorKind::WorkerUnavailable,
                    &path,
                    "asset worker queue is unavailable",
                );
                self.fail_path(&key, id, generation, &error);
                return Err(error);
            }
        }
        Ok(Handle::new(id, generation))
    }

    /// Retry a previously failed asynchronous request with a fresh generation. Other states follow
    /// the normal deduplicating behavior of [`request`](Self::request).
    pub fn retry<T: Asset>(&mut self, path: impl AsRef<str>) -> Result<Handle<T>, LoadError> {
        let path = path.as_ref().to_owned();
        let key = (TypeId::of::<T>(), path.clone());
        if self
            .paths
            .get(&key)
            .is_some_and(|entry| matches!(&entry.state, LoadState::Failed(_)))
        {
            if let Some(entry) = self.paths.get_mut(&key) {
                entry.state = LoadState::Unloaded;
            }
        }
        self.request::<T>(path)
    }

    /// Queue an asynchronous request and return a strong lease that pins the pending/loaded asset.
    pub fn request_ref<T: Asset>(
        &mut self,
        path: impl AsRef<str>,
    ) -> Result<AssetRef<T>, LoadError> {
        let handle = self.request::<T>(path)?;
        self.strong_ref(handle).ok_or_else(|| {
            LoadError::new(
                LoadErrorKind::WorkerUnavailable,
                "<asset lease>",
                "could not create a strong lease for a stale handle",
            )
        })
    }

    /// Load synchronously and return a strong lease that pins the cached asset against eviction.
    pub fn load_ref<T: Asset>(&mut self, path: impl AsRef<str>) -> Result<AssetRef<T>, LoadError> {
        let handle = self.load::<T>(path)?;
        self.strong_ref(handle).ok_or_else(|| {
            LoadError::new(
                LoadErrorKind::TypeMismatch,
                "<asset lease>",
                "loaded handle could not be pinned",
            )
        })
    }

    /// Create a new independent strong lease for a current loaded or in-flight handle.
    pub fn strong_ref<T: Asset>(&self, handle: Handle<T>) -> Option<AssetRef<T>> {
        let is_live = self.paths.iter().any(|((type_id, _), entry)| {
            *type_id == TypeId::of::<T>()
                && entry.id == handle.id
                && entry.generation == handle.generation
                && matches!(&entry.state, LoadState::Loaded | LoadState::Loading)
        });
        if !is_live {
            return None;
        }
        let key = (handle.id, handle.generation);
        let mut counts = self.strong_refs.lock().ok()?;
        let count = counts.entry(key).or_insert(0);
        *count = count.saturating_add(1);
        drop(counts);
        Some(AssetRef {
            handle,
            _lease: Arc::new(AssetLease {
                key,
                counts: Arc::clone(&self.strong_refs),
            }),
        })
    }

    /// Return the number of independent strong leases pinning a handle's current generation.
    pub fn strong_ref_count<T>(&self, handle: Handle<T>) -> usize {
        self.strong_refs
            .lock()
            .map(|counts| {
                counts
                    .get(&(handle.id, handle.generation))
                    .copied()
                    .unwrap_or(0)
            })
            // If accounting is poisoned, protect values rather than evicting them accidentally.
            .unwrap_or(usize::MAX)
    }

    /// Explicitly load a default value when source lookup or decoding fails.
    pub fn load_or<T: Asset>(&mut self, path: impl AsRef<str>, default: T) -> Handle<T> {
        let path = path.as_ref().to_owned();
        match self.load::<T>(&path) {
            Ok(handle) => handle,
            Err(_) => {
                let key = (TypeId::of::<T>(), path.clone());
                let Some((id, generation)) = self
                    .paths
                    .get(&key)
                    .map(|entry| (entry.id, entry.generation))
                else {
                    return Handle::new(AssetId(0), 0);
                };
                let handle = Handle::new(id, generation);
                // `load_or` predates asynchronous requests. If a request for this path is already
                // in flight, do not replace its Loading state with a fallback value: doing so would
                // make the valid worker completion look stale and discard the requested asset.
                if self
                    .paths
                    .get(&key)
                    .is_some_and(|entry| entry.state == LoadState::Loading)
                {
                    return handle;
                }
                self.storage_mut::<T>().insert(handle, default);
                self.revisions.insert((id, generation), 1);
                if let Some(entry) = self.paths.get_mut(&key) {
                    entry.state = LoadState::Loaded;
                    entry.reload_pending = false;
                }
                self.last_reload_errors.remove(&(id, generation));
                self.pending_events.push(AssetEvent::Loaded {
                    id: id.get(),
                    generation,
                    path,
                });
                handle
            }
        }
    }

    /// Get the most recent load state for a typed path.
    pub fn state<T: Asset>(&self, path: impl AsRef<str>) -> Option<&LoadState> {
        self.paths
            .get(&(TypeId::of::<T>(), path.as_ref().to_owned()))
            .map(|entry| &entry.state)
    }

    /// Monotonically advancing content revision for a currently resident handle. The asset ID and
    /// generation remain stable across successful hot reloads; render caches can use this revision
    /// to know when derived GPU data must be rebuilt.
    pub fn revision<T: Asset>(&self, handle: Handle<T>) -> Option<u64> {
        self.get(handle)?;
        self.revisions.get(&(handle.id, handle.generation)).copied()
    }

    /// Get the last hot-reload error for a handle without discarding its last good value.
    pub fn last_reload_error<T>(&self, handle: Handle<T>) -> Option<&LoadError> {
        self.last_reload_errors.get(&(handle.id, handle.generation))
    }

    /// Get a typed asset by handle. Stale generations return `None`.
    pub fn get<T: Asset>(&self, handle: Handle<T>) -> Option<&T> {
        self.assets
            .get(&TypeId::of::<T>())?
            .downcast_ref::<Assets<T>>()?
            .get(handle)
    }

    /// Mutably get a typed asset by handle. Stale generations return `None`.
    pub fn get_mut<T: Asset>(&mut self, handle: Handle<T>) -> Option<&mut T> {
        self.get(handle)?;
        // Conservatively mark mutable access as a content change: the caller may mutate any field
        // after this reference is returned, so derived renderer resources must be considered dirty.
        let revision = self
            .revisions
            .entry((handle.id, handle.generation))
            .or_insert(1);
        *revision = revision.saturating_add(1);
        self.assets
            .get_mut(&TypeId::of::<T>())?
            .downcast_mut::<Assets<T>>()?
            .get_mut(handle)
    }

    /// Access typed storage directly.
    pub fn assets<T: Asset>(&self) -> Option<&Assets<T>> {
        self.assets.get(&TypeId::of::<T>())?.downcast_ref()
    }

    /// Commit completed worker requests and return lifecycle events for the caller's thread.
    ///
    /// The method does not create GPU resources. A rendering plugin should observe loaded CPU-side
    /// assets and upload them from its render pass, where its GPU device/queue is available.
    pub fn poll_pending_loads(&mut self) -> Vec<AssetEvent> {
        let mut completions = Vec::new();
        if let Ok(receiver) = self.completion_rx.lock() {
            while let Ok(completion) = receiver.try_recv() {
                completions.push(completion);
            }
        }

        for completion in completions {
            let key = (completion.type_id, completion.path.clone());
            let reload = completion.reload.clone();
            let current = self.paths.get(&key).is_some_and(|entry| {
                let same_generation =
                    entry.id == completion.id && entry.generation == completion.generation;
                if !same_generation {
                    return false;
                }
                match reload.as_ref() {
                    Some(reload) => {
                        entry.state == LoadState::Loaded
                            && entry.reload_pending
                            && entry.reload_token == reload.token
                    }
                    None => matches!(&entry.state, LoadState::Loading),
                }
            });
            if !current {
                // A retry, eviction, or newer generation superseded this worker. Never overwrite
                // newer server state with an out-of-date completion.
                continue;
            }

            let result = completion.result.and_then(|decoded| {
                if decoded.loader.asset_type() != completion.type_id {
                    return Err(LoadError::new(
                        LoadErrorKind::TypeMismatch,
                        &completion.path,
                        "asynchronous decoder returned a different asset type",
                    ));
                }
                decoded
                    .loader
                    .store(
                        &mut self.assets,
                        completion.id,
                        completion.generation,
                        decoded.value,
                    )
                    .map(|()| (decoded.source_id, decoded.modified))
            });

            match result {
                Ok((source_id, modified)) => {
                    if let Some(entry) = self.paths.get_mut(&key) {
                        entry.source_id = Some(source_id);
                        entry.modified = modified;
                        entry.state = LoadState::Loaded;
                        entry.reload_pending = false;
                    }
                    let revision = self
                        .revisions
                        .entry((completion.id, completion.generation))
                        .or_insert(1);
                    if reload.is_some() {
                        *revision = revision.saturating_add(1);
                        self.pending_events.push(AssetEvent::Modified {
                            id: completion.id.get(),
                        });
                    } else {
                        self.pending_events.push(AssetEvent::Loaded {
                            id: completion.id.get(),
                            generation: completion.generation,
                            path: completion.path,
                        });
                    }
                    self.last_reload_errors
                        .remove(&(completion.id, completion.generation));
                }
                Err(error) => {
                    if let Some(reload) = reload {
                        // Keep the last-good asset and suppress repeated attempts for this exact
                        // failed edit. A new timestamp triggers another worker reload.
                        if let Some(entry) = self.paths.get_mut(&key) {
                            entry.reload_pending = false;
                            entry.modified = reload.observed_modified;
                        }
                        self.last_reload_errors
                            .insert((completion.id, completion.generation), error.clone());
                        self.pending_events.push(AssetEvent::ReloadFailed {
                            id: completion.id.get(),
                            generation: completion.generation,
                            path: completion.path,
                            error,
                        });
                    } else {
                        self.fail_path(&key, completion.id, completion.generation, &error);
                    }
                }
            }
        }

        if self.auto_evict_unused {
            self.evict_unused();
        }
        std::mem::take(&mut self.pending_events)
    }

    /// Evict all loaded cache entries with no strong leases. Weak handles become stale immediately.
    /// Returns the number of values removed. The default is manual eviction to preserve legacy
    /// behavior for callers that use [`Handle`] without an [`AssetRef`].
    pub fn evict_unused(&mut self) -> usize {
        let counts = match self.strong_refs.lock() {
            Ok(counts) => counts,
            Err(_) => return 0,
        };
        let candidates: Vec<_> = self
            .paths
            .iter()
            .filter_map(|((type_id, path), entry)| {
                (entry.state == LoadState::Loaded
                    && counts
                        .get(&(entry.id, entry.generation))
                        .copied()
                        .unwrap_or(0)
                        == 0)
                    .then_some((*type_id, path.clone(), entry.id, entry.generation))
            })
            .collect();
        drop(counts);

        let mut evicted = 0;
        for (type_id, path, id, generation) in candidates {
            let removed = self
                .asset_removers
                .get(&type_id)
                .copied()
                .is_some_and(|remove| remove(&mut self.assets, id, generation));
            if !removed {
                continue;
            }
            if let Some(entry) = self.paths.get_mut(&(type_id, path.clone())) {
                if entry.id == id && entry.generation == generation {
                    entry.state = LoadState::Unloaded;
                    entry.source_id = None;
                    entry.modified = None;
                    entry.reload_pending = false;
                    self.revisions.remove(&(id, generation));
                    self.pending_events.push(AssetEvent::Unloaded {
                        id: id.get(),
                        generation,
                        path,
                    });
                    self.last_reload_errors.remove(&(id, generation));
                    evicted += 1;
                }
            }
        }
        evicted
    }

    /// Poll changes. AssetPlugin and servers using `request` reload on workers.
    /// Synchronous-only direct users keep the legacy immediate behavior.
    pub fn poll_hot_reload(&mut self) -> Vec<AssetEvent> {
        if self.async_hot_reload {
            self.poll_hot_reload_async()
        } else {
            self.poll_hot_reload_sync()
        }
    }

    /// Synchronous fallback for legacy direct `AssetServer` users.
    fn poll_hot_reload_sync(&mut self) -> Vec<AssetEvent> {
        #[cfg(not(any(feature = "hot_reload", debug_assertions)))]
        {
            return Vec::new();
        }
        #[cfg(any(feature = "hot_reload", debug_assertions))]
        {
            if self.last_poll.elapsed() < self.poll_interval {
                return Vec::new();
            }
            self.last_poll = Instant::now();
            let changed: Vec<_> = self
                .paths
                .iter()
                .filter_map(|((type_id, path), entry)| {
                    if entry.state != LoadState::Loaded || entry.reload_pending {
                        return None;
                    }
                    let source_id = entry.source_id?;
                    let source = self.sources.iter().find(|source| source.id == source_id)?;
                    let modified = source.source.modified(path);
                    (modified != entry.modified).then(|| {
                        (
                            *type_id,
                            path.clone(),
                            entry.id,
                            entry.generation,
                            source_id,
                            entry.extension.clone(),
                            modified,
                        )
                    })
                })
                .collect();
            let mut events = Vec::new();
            for (type_id, path, id, generation, source_id, extension, modified) in changed {
                let read = self
                    .sources
                    .iter()
                    .find(|source| source.id == source_id)
                    .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "asset source removed"))
                    .and_then(|source| source.source.read(&path));
                let decoded = match read {
                    Ok(bytes) => self
                        .loaders
                        .get(&extension)
                        .cloned()
                        .ok_or_else(|| {
                            LoadError::new(
                                LoadErrorKind::NoLoader,
                                &path,
                                format!("no loader registered for extension `{extension}`"),
                            )
                        })
                        .and_then(|loader| {
                            let mut context = LoadContext { path: path.clone() };
                            loader
                                .load(&bytes, &mut context)
                                .map(|asset| (loader, asset))
                        }),
                    Err(error) => Err(LoadError::new(LoadErrorKind::Io, &path, error.to_string())),
                };
                let result = decoded.and_then(|(loader, asset)| {
                    if loader.asset_type() != type_id {
                        return Err(LoadError::new(
                            LoadErrorKind::TypeMismatch,
                            &path,
                            "reloaded value has a different asset type",
                        ));
                    }
                    loader.store(&mut self.assets, id, generation, asset)
                });
                match result {
                    Ok(()) => {
                        if let Some(entry) = self.paths.get_mut(&(type_id, path.clone())) {
                            entry.modified = modified;
                            entry.state = LoadState::Loaded;
                            entry.reload_pending = false;
                        }
                        let revision = self.revisions.entry((id, generation)).or_insert(1);
                        *revision = revision.saturating_add(1);
                        self.last_reload_errors.remove(&(id, generation));
                        events.push(AssetEvent::Modified { id: id.get() });
                    }
                    Err(error) => {
                        if let Some(entry) = self.paths.get_mut(&(type_id, path.clone())) {
                            entry.modified = modified;
                            entry.reload_pending = false;
                        }
                        self.last_reload_errors
                            .insert((id, generation), error.clone());
                        events.push(AssetEvent::ReloadFailed {
                            id: id.get(),
                            generation,
                            path,
                            error,
                        });
                    }
                }
            }
            events
        }
    }

    /// Queue changed paths for worker-thread reload; commit results via `poll_pending_loads`.
    fn poll_hot_reload_async(&mut self) -> Vec<AssetEvent> {
        #[cfg(not(any(feature = "hot_reload", debug_assertions)))]
        {
            return Vec::new();
        }
        #[cfg(any(feature = "hot_reload", debug_assertions))]
        {
            if self.last_poll.elapsed() < self.poll_interval {
                return Vec::new();
            }
            self.last_poll = Instant::now();
            let changed: Vec<_> = self
                .paths
                .iter()
                .filter_map(|((type_id, path), entry)| {
                    if entry.state != LoadState::Loaded || entry.reload_pending {
                        return None;
                    }
                    let source_id = entry.source_id?;
                    let source = self.sources.iter().find(|source| source.id == source_id)?;
                    let modified = source.source.modified(path);
                    (modified != entry.modified).then(|| {
                        (
                            *type_id,
                            path.clone(),
                            entry.id,
                            entry.generation,
                            source_id,
                            entry.extension.clone(),
                            modified,
                        )
                    })
                })
                .collect();
            let mut events = Vec::new();
            for (type_id, path, id, generation, source_id, extension, modified) in changed {
                let source = self
                    .sources
                    .iter()
                    .find(|source| source.id == source_id)
                    .map(|source| SourceSnapshot {
                        id: source.id,
                        source: Arc::clone(&source.source),
                    });
                let loader = self.loaders.get(&extension).cloned();
                let error = match (source.as_ref(), loader.as_ref()) {
                    (None, _) => Some(LoadError::new(
                        LoadErrorKind::Io,
                        &path,
                        "the asset source for this loaded path is no longer available",
                    )),
                    (_, None) => Some(LoadError::new(
                        LoadErrorKind::NoLoader,
                        &path,
                        format!("no loader registered for extension `{extension}`"),
                    )),
                    (Some(_), Some(loader)) if loader.asset_type() != type_id => {
                        Some(LoadError::new(
                            LoadErrorKind::TypeMismatch,
                            &path,
                            "the current extension loader produces a different asset type",
                        ))
                    }
                    _ => None,
                };
                if let Some(error) = error {
                    if let Some(entry) = self.paths.get_mut(&(type_id, path.clone())) {
                        entry.modified = modified;
                    }
                    self.last_reload_errors
                        .insert((id, generation), error.clone());
                    events.push(AssetEvent::ReloadFailed {
                        id: id.get(),
                        generation,
                        path,
                        error,
                    });
                    continue;
                }

                if self.workers.is_none() {
                    match WorkerPool::new(self.worker_count) {
                        Ok(pool) => self.workers = Some(pool),
                        Err(worker_error) => {
                            let error = LoadError::new(
                                LoadErrorKind::WorkerUnavailable,
                                &path,
                                format!("could not start asset worker pool: {worker_error}"),
                            );
                            let should_report =
                                self.last_reload_errors.get(&(id, generation)) != Some(&error);
                            self.last_reload_errors
                                .insert((id, generation), error.clone());
                            if should_report {
                                events.push(AssetEvent::ReloadFailed {
                                    id: id.get(),
                                    generation,
                                    path,
                                    error,
                                });
                            }
                            continue;
                        }
                    }
                }

                let key = (type_id, path.clone());
                let token = self
                    .paths
                    .get(&key)
                    .map_or(1, |entry| entry.reload_token.wrapping_add(1).max(1));
                let reload = ReloadRequest {
                    token,
                    observed_modified: modified,
                };
                let source = source.expect("source validated before queueing reload");
                let loader = loader.expect("loader validated before queueing reload");
                let completion_tx = self.completion_tx.clone();
                let request_path = path.clone();
                let request_loader = Arc::clone(&loader);
                let request_source = source.clone();
                let task: WorkerTask = Box::new(move || {
                    let modified_before = request_source.source.modified(&request_path);
                    let result = request_source
                        .source
                        .read(&request_path)
                        .map_err(|error| {
                            LoadError::new(LoadErrorKind::Io, &request_path, error.to_string())
                        })
                        .and_then(|bytes| {
                            let modified_after = request_source.source.modified(&request_path);
                            if modified_before != modified_after {
                                return Err(LoadError::new(
                                    LoadErrorKind::Io,
                                    &request_path,
                                    "asset changed while hot reload was reading it; \
                                     retrying after the next poll",
                                ));
                            }
                            let mut context = LoadContext {
                                path: request_path.clone(),
                            };
                            request_loader
                                .load(&bytes, &mut context)
                                .map(|value| DecodedAsset {
                                    loader: Arc::clone(&request_loader),
                                    value,
                                    source_id: request_source.id,
                                    modified: modified_after,
                                })
                        });
                    let _ = completion_tx.send(LoadCompletion {
                        type_id,
                        path: request_path,
                        id,
                        generation,
                        reload: Some(reload),
                        result,
                    });
                });
                let queue_result = self.workers.as_ref().map(|workers| workers.execute(task));
                match queue_result {
                    Some(Ok(())) => {
                        if let Some(entry) = self.paths.get_mut(&key) {
                            entry.reload_pending = true;
                            entry.reload_token = token;
                        }
                    }
                    Some(Err(mpsc::TrySendError::Full(_))) => {
                        // Leave the old modification stamp in place. The next hot-reload poll will
                        // retry queue admission after other pending asset work has drained.
                    }
                    Some(Err(mpsc::TrySendError::Disconnected(_))) => {
                        self.workers = None;
                        let error = LoadError::new(
                            LoadErrorKind::WorkerUnavailable,
                            &path,
                            "asset worker queue is unavailable for hot reload",
                        );
                        let should_report =
                            self.last_reload_errors.get(&(id, generation)) != Some(&error);
                        self.last_reload_errors
                            .insert((id, generation), error.clone());
                        if should_report {
                            events.push(AssetEvent::ReloadFailed {
                                id: id.get(),
                                generation,
                                path,
                                error,
                            });
                        }
                    }
                    None => {
                        let error = LoadError::new(
                            LoadErrorKind::WorkerUnavailable,
                            &path,
                            "asset worker pool was not initialized",
                        );
                        let should_report =
                            self.last_reload_errors.get(&(id, generation)) != Some(&error);
                        self.last_reload_errors
                            .insert((id, generation), error.clone());
                        if should_report {
                            events.push(AssetEvent::ReloadFailed {
                                id: id.get(),
                                generation,
                                path,
                                error,
                            });
                        }
                    }
                }
            }
            events
        }
    }

    fn begin_loading(&mut self, key: &(TypeId, String), extension: &str) -> (AssetId, u32) {
        if let Some((old_id, old_generation)) = self.paths.get(key).and_then(|entry| {
            matches!(&entry.state, LoadState::Unloaded).then_some((entry.id, entry.generation))
        }) {
            let (id, generation) = if let Some(generation) = old_generation.checked_add(1) {
                (old_id, generation)
            } else {
                (self.allocate_id(), 1)
            };
            if let Some(entry) = self.paths.get_mut(key) {
                entry.id = id;
                entry.generation = generation;
                entry.source_id = None;
                entry.modified = None;
                entry.extension = extension.to_owned();
                entry.state = LoadState::Loading;
                entry.reload_pending = false;
            }
            self.pending_events.push(AssetEvent::Loading {
                id: id.get(),
                generation,
                path: key.1.clone(),
            });
            return (id, generation);
        }

        let id = self.allocate_id();
        self.paths.insert(
            key.clone(),
            PathEntry {
                id,
                generation: 1,
                source_id: None,
                modified: None,
                extension: extension.to_owned(),
                state: LoadState::Loading,
                reload_pending: false,
                reload_token: 0,
            },
        );
        self.pending_events.push(AssetEvent::Loading {
            id: id.get(),
            generation: 1,
            path: key.1.clone(),
        });
        (id, 1)
    }

    fn allocate_id(&mut self) -> AssetId {
        let id = AssetId(self.next_id);
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("asset ID space exhausted");
        id
    }

    fn record_request_failure(
        &mut self,
        key: &(TypeId, String),
        extension: &str,
        error: LoadError,
    ) -> LoadError {
        let (id, generation) = self.begin_loading(key, extension);
        self.fail_path(key, id, generation, &error);
        error
    }

    fn fail_path(
        &mut self,
        key: &(TypeId, String),
        id: AssetId,
        generation: u32,
        error: &LoadError,
    ) {
        if let Some(entry) = self.paths.get_mut(key) {
            if entry.id == id && entry.generation == generation {
                entry.state = LoadState::Failed(error.clone());
            }
        }
        self.pending_events.push(AssetEvent::Failed {
            id: id.get(),
            generation,
            path: key.1.clone(),
            error: error.clone(),
        });
    }

    fn load_bytes(&self, path: &str) -> Result<(Vec<u8>, u64, Option<SystemTime>), LoadError> {
        let sources: Vec<_> = self
            .sources
            .iter()
            .map(|source| SourceSnapshot {
                id: source.id,
                source: Arc::clone(&source.source),
            })
            .collect();
        read_from_sources(&sources, path)
    }

    fn storage_mut<T: Asset>(&mut self) -> &mut Assets<T> {
        self.asset_removers
            .entry(TypeId::of::<T>())
            .or_insert(remove_asset_from_map::<T>);
        self.assets
            .entry(TypeId::of::<T>())
            .or_insert_with(|| Box::new(Assets::<T>::default()))
            .downcast_mut::<Assets<T>>()
            .expect("typed asset storage is created for its TypeId")
    }
}

fn remove_asset_from_map<T: Asset>(
    assets: &mut ErasedAssetMap,
    id: AssetId,
    generation: u32,
) -> bool {
    assets
        .get_mut(&TypeId::of::<T>())
        .and_then(|storage| storage.downcast_mut::<Assets<T>>())
        .is_some_and(|storage| storage.remove(Handle::new(id, generation)))
}

fn extension_for(path: &str) -> String {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn read_from_sources(
    sources: &[SourceSnapshot],
    path: &str,
) -> Result<(Vec<u8>, u64, Option<SystemTime>), LoadError> {
    if sources.is_empty() {
        return Err(LoadError::new(
            LoadErrorKind::MissingSource,
            path,
            "no asset sources are registered",
        ));
    }
    let mut last_io_error = None;
    for entry in sources {
        match entry.source.read(path) {
            Ok(bytes) => {
                return Ok((bytes, entry.id, entry.source.modified(path)));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => last_io_error = Some(error),
        }
    }
    if let Some(error) = last_io_error {
        Err(LoadError::new(LoadErrorKind::Io, path, error.to_string()))
    } else {
        Err(LoadError::new(
            LoadErrorKind::MissingSource,
            path,
            "no registered source contains this path",
        ))
    }
}

/// Byte asset used with [`BytesLoader`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BytesAsset(pub Vec<u8>);

/// UTF-8 text asset used with [`TextLoader`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextAsset(pub String);

/// Loader that keeps the source bytes unchanged.
pub struct BytesLoader;

impl AssetLoader for BytesLoader {
    type Asset = BytesAsset;

    fn extensions(&self) -> &[&str] {
        &["bin", "bytes"]
    }

    fn load(&self, bytes: &[u8], _context: &mut LoadContext) -> Result<Self::Asset, LoadError> {
        Ok(BytesAsset(bytes.to_vec()))
    }
}

/// Loader for UTF-8 text files.
pub struct TextLoader;

impl AssetLoader for TextLoader {
    type Asset = TextAsset;

    fn extensions(&self) -> &[&str] {
        &["txt", "text"]
    }

    fn load(&self, bytes: &[u8], context: &mut LoadContext) -> Result<Self::Asset, LoadError> {
        String::from_utf8(bytes.to_vec())
            .map(TextAsset)
            .map_err(|error| {
                LoadError::new(LoadErrorKind::Decode, &context.path, error.to_string())
            })
    }
}

/// Installs an asset server and polls CPU load completions and hot reload once per frame.
#[derive(Default)]
pub struct AssetPlugin;

impl Plugin for AssetPlugin {
    fn name(&self) -> &'static str {
        "AssetPlugin"
    }

    fn build(&self, app: &mut App) {
        if app.world.try_resource::<AssetServer>().is_none() {
            app.insert_resource(AssetServer::default());
        }
        app.world
            .resource_mut::<AssetServer>()
            .set_async_hot_reload(true);
        app.add_event::<AssetEvent>();
        app.add_system(Stage::First, update_asset_server);
    }
}

fn update_asset_server(world: &mut World) {
    world.resource_scope::<AssetServer, _>(|world, server| {
        for event in server.poll_pending_loads() {
            world.send_event(event);
        }
        for event in server.poll_hot_reload() {
            world.send_event(event);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{
        AssetEvent, AssetServer, BytesAsset, BytesLoader, LoadErrorKind, LoadState, MemorySource,
        TextAsset, TextLoader,
    };
    use std::time::Duration;

    #[test]
    fn memory_source_loads_typed_assets() {
        let source = MemorySource::default();
        source.insert("ui/title.txt", b"Voxelworks".to_vec());
        let mut assets = AssetServer::default();
        assets.add_source(0, source);
        assets.register_loader(TextLoader);
        let handle = assets.load::<TextAsset>("ui/title.txt").unwrap();
        assert_eq!(
            assets.get(handle),
            Some(&TextAsset("Voxelworks".to_owned()))
        );
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
    fn polling_reloads_memory_assets_without_changing_the_handle() {
        let source = MemorySource::default();
        source.insert("config.txt", b"before".to_vec());
        let mut assets = AssetServer::default();
        assets.add_source(0, source.clone());
        assets.register_loader(TextLoader);
        assets.set_poll_interval(Duration::ZERO);
        let handle = assets.load::<TextAsset>("config.txt").unwrap();
        source.insert("config.txt", b"after".to_vec());
        assert_eq!(
            assets.poll_hot_reload(),
            [AssetEvent::Modified { id: handle.id() }]
        );
        assert_eq!(assets.get(handle), Some(&TextAsset("after".to_owned())));
    }
}
