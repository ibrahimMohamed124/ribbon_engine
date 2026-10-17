//! Synchronous asset sources, typed handles, loaders, and optional polling reload.
//!
//! Asset handles are weak identifiers; they do not keep values alive independently of the server.

use engine_app::{App, Stage, World};
use engine_events::WorldEventsExt;
use engine_plugins::Plugin;
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::fmt;
use std::io;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Typed error kind for asset lookup, source access, routing, and decoding failures.
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
    fn new(kind: LoadErrorKind, path: &str, message: impl Into<String>) -> Self {
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

/// Weak typed identifier. The server owns the asset and generation validation.
pub struct Handle<T> {
    id: u64,
    generation: u32,
    marker: PhantomData<fn() -> T>,
}

impl<T> Handle<T> {
    fn new(id: u64, generation: u32) -> Self {
        Self {
            id,
            generation,
            marker: PhantomData,
        }
    }

    /// Stable identifier shared by all typed views of this loaded asset.
    pub fn id(self) -> u64 {
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
        let (generation, value) = self.values.get(&handle.id)?;
        (*generation == handle.generation).then_some(value)
    }

    /// Mutably get a value when both the identifier and generation match.
    pub fn get_mut(&mut self, handle: Handle<T>) -> Option<&mut T> {
        let (generation, value) = self.values.get_mut(&handle.id)?;
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
        self.values.insert(handle.id, (handle.generation, value));
    }
}

/// State of a synchronous asset load.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadState {
    /// The request has started but has not finished.
    Loading,
    /// Asset bytes decoded successfully.
    Loaded,
    /// Source lookup or decoding failed.
    Failed(LoadError),
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

/// Extension-based, synchronous asset decoder.
pub trait AssetLoader: Send + Sync + 'static {
    /// Asset type produced by this loader.
    type Asset: Asset;
    /// File extensions supported by this loader, without a leading dot.
    fn extensions(&self) -> &[&str];
    /// Decode asset bytes.
    fn load(&self, bytes: &[u8], context: &mut LoadContext) -> Result<Self::Asset, LoadError>;
}

trait ErasedLoader: Send + Sync {
    fn asset_type(&self) -> TypeId;
    fn load(
        &self,
        bytes: &[u8],
        context: &mut LoadContext,
    ) -> Result<Box<dyn Any + Send + Sync>, LoadError>;
    fn store(
        &self,
        assets: &mut HashMap<TypeId, Box<dyn Any + Send + Sync>>,
        id: u64,
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
        assets: &mut HashMap<TypeId, Box<dyn Any + Send + Sync>>,
        id: u64,
        generation: u32,
        value: Box<dyn Any + Send + Sync>,
    ) -> Result<(), LoadError> {
        let asset = value.downcast::<L::Asset>().map_err(|_| {
            LoadError::new(
                LoadErrorKind::TypeMismatch,
                "<reload>",
                "loader returned a value with the wrong concrete type",
            )
        })?;
        let storage = assets
            .entry(TypeId::of::<L::Asset>())
            .or_insert_with(|| Box::new(Assets::<L::Asset>::default()));
        let typed = storage.downcast_mut::<Assets<L::Asset>>().ok_or_else(|| {
            LoadError::new(
                LoadErrorKind::TypeMismatch,
                "<reload>",
                "asset storage has an unexpected concrete type",
            )
        })?;
        typed.insert(Handle::new(id, generation), *asset);
        Ok(())
    }
}

struct SourceEntry {
    priority: i32,
    source: Box<dyn AssetSource>,
}

struct PathEntry {
    id: u64,
    generation: u32,
    source_index: Option<usize>,
    modified: Option<SystemTime>,
    extension: String,
    state: LoadState,
}

/// Asset id and change notification emitted after a successful hot reload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetEvent {
    /// Contents changed while the weak handle stayed the same.
    Modified { id: u64 },
}

/// Synchronous priority-ordered asset server and typed cache.
pub struct AssetServer {
    sources: Vec<SourceEntry>,
    loaders: HashMap<String, Arc<dyn ErasedLoader>>,
    assets: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
    paths: HashMap<(TypeId, String), PathEntry>,
    next_id: u64,
    last_poll: Instant,
    poll_interval: Duration,
}

impl Default for AssetServer {
    fn default() -> Self {
        let mut server = Self {
            sources: Vec::new(),
            loaders: HashMap::new(),
            assets: HashMap::new(),
            paths: HashMap::new(),
            next_id: 1,
            last_poll: Instant::now(),
            poll_interval: Duration::from_millis(500),
        };
        server.register_loader(BytesLoader);
        server.register_loader(TextLoader);
        server
    }
}

impl AssetServer {
    /// Add a source. Higher priorities are consulted first; equal priorities keep insertion order.
    pub fn add_source(&mut self, priority: i32, source: impl AssetSource) {
        let index = self
            .sources
            .iter()
            .position(|entry| priority > entry.priority)
            .unwrap_or(self.sources.len());
        self.sources.insert(
            index,
            SourceEntry {
                priority,
                source: Box::new(source),
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
        for extension in extensions {
            self.loaders.insert(extension, Arc::clone(&loader));
        }
    }

    /// Set the interval used by polling hot reload.
    pub fn set_poll_interval(&mut self, interval: Duration) {
        self.poll_interval = interval;
    }

    /// Load synchronously and cache by asset type and path.
    pub fn load<T: Asset>(&mut self, path: impl AsRef<str>) -> Result<Handle<T>, LoadError> {
        let path = path.as_ref();
        let key = (TypeId::of::<T>(), path.to_owned());
        if let Some(entry) = self.paths.get(&key) {
            return match &entry.state {
                LoadState::Loaded => Ok(Handle::new(entry.id, entry.generation)),
                LoadState::Loading => Err(LoadError::new(
                    LoadErrorKind::Decode,
                    path,
                    "synchronous load was left in the loading state",
                )),
                LoadState::Failed(error) => Err(error.clone()),
            };
        }

        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        let extension = Path::new(path)
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        self.paths.insert(
            key.clone(),
            PathEntry {
                id,
                generation: 1,
                source_index: None,
                modified: None,
                extension: extension.clone(),
                state: LoadState::Loading,
            },
        );
        let result = self
            .load_bytes(path)
            .and_then(|(bytes, source_index, modified)| {
                let loader = self.loaders.get(&extension).cloned().ok_or_else(|| {
                    LoadError::new(
                        LoadErrorKind::NoLoader,
                        path,
                        format!("no loader registered for extension `{extension}`"),
                    )
                })?;
                if loader.asset_type() != TypeId::of::<T>() {
                    return Err(LoadError::new(
                        LoadErrorKind::TypeMismatch,
                        path,
                        "extension loader produces a different asset type",
                    ));
                }
                let mut context = LoadContext {
                    path: path.to_owned(),
                };
                let value = loader.load(&bytes, &mut context)?;
                let value = value.downcast::<T>().map_err(|_| {
                    LoadError::new(
                        LoadErrorKind::TypeMismatch,
                        path,
                        "loader returned a value with the wrong concrete type",
                    )
                })?;
                self.storage_mut::<T>().insert(Handle::new(id, 1), *value);
                if let Some(entry) = self.paths.get_mut(&key) {
                    entry.source_index = Some(source_index);
                    entry.modified = modified;
                    entry.state = LoadState::Loaded;
                }
                Ok(Handle::new(id, 1))
            });
        if let Err(error) = &result {
            if let Some(entry) = self.paths.get_mut(&key) {
                entry.state = LoadState::Failed(error.clone());
            }
        }
        result
    }

    /// Explicitly load a default value when source lookup or decoding fails.
    pub fn load_or<T: Asset>(&mut self, path: impl AsRef<str>, default: T) -> Handle<T> {
        let path = path.as_ref().to_owned();
        match self.load::<T>(&path) {
            Ok(handle) => handle,
            Err(_) => {
                let key = (TypeId::of::<T>(), path);
                let Some((id, generation)) = self
                    .paths
                    .get(&key)
                    .map(|entry| (entry.id, entry.generation))
                else {
                    return Handle::new(0, 0);
                };
                let handle = Handle::new(id, generation);
                self.storage_mut::<T>().insert(handle, default);
                if let Some(entry) = self.paths.get_mut(&key) {
                    entry.state = LoadState::Loaded;
                }
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

    /// Get a typed asset by handle.
    pub fn get<T: Asset>(&self, handle: Handle<T>) -> Option<&T> {
        self.assets
            .get(&TypeId::of::<T>())?
            .downcast_ref::<Assets<T>>()?
            .get(handle)
    }

    /// Mutably get a typed asset by handle.
    pub fn get_mut<T: Asset>(&mut self, handle: Handle<T>) -> Option<&mut T> {
        self.assets
            .get_mut(&TypeId::of::<T>())?
            .downcast_mut::<Assets<T>>()?
            .get_mut(handle)
    }

    /// Access typed storage directly.
    pub fn assets<T: Asset>(&self) -> Option<&Assets<T>> {
        self.assets.get(&TypeId::of::<T>())?.downcast_ref()
    }

    /// Poll changed loaded paths, replacing their values under the same handle.
    pub fn poll_hot_reload(&mut self) -> Vec<AssetEvent> {
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
                    if entry.state != LoadState::Loaded {
                        return None;
                    }
                    let source_index = entry.source_index?;
                    let modified = self.sources.get(source_index)?.source.modified(path);
                    (modified != entry.modified).then(|| {
                        (
                            *type_id,
                            path.clone(),
                            entry.id,
                            entry.generation,
                            source_index,
                            entry.extension.clone(),
                            modified,
                        )
                    })
                })
                .collect();
            let mut events = Vec::new();
            for (type_id, path, id, generation, source_index, extension, modified) in changed {
                let read = self.sources[source_index].source.read(&path);
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
                if let Some(entry) = self.paths.get_mut(&(type_id, path.clone())) {
                    match result {
                        Ok(()) => {
                            entry.modified = modified;
                            entry.state = LoadState::Loaded;
                            events.push(AssetEvent::Modified { id });
                        }
                        Err(error) => entry.state = LoadState::Failed(error),
                    }
                }
            }
            events
        }
    }

    fn load_bytes(&self, path: &str) -> Result<(Vec<u8>, usize, Option<SystemTime>), LoadError> {
        if self.sources.is_empty() {
            return Err(LoadError::new(
                LoadErrorKind::MissingSource,
                path,
                "no asset sources are registered",
            ));
        }
        let mut last_io_error = None;
        for (index, entry) in self.sources.iter().enumerate() {
            match entry.source.read(path) {
                Ok(bytes) => {
                    return Ok((bytes, index, entry.source.modified(path)));
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

    fn storage_mut<T: Asset>(&mut self) -> &mut Assets<T> {
        self.assets
            .entry(TypeId::of::<T>())
            .or_insert_with(|| Box::new(Assets::<T>::default()))
            .downcast_mut::<Assets<T>>()
            .expect("typed asset storage is created for its TypeId")
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

/// Installs an asset server and the configured polling reload system.
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
        app.add_event::<AssetEvent>();
        app.add_system(Stage::First, update_hot_reload);
    }
}

fn update_hot_reload(world: &mut World) {
    world.resource_scope::<AssetServer, _>(|world, server| {
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
