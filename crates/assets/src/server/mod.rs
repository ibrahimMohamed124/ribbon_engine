use super::*;

/// Priority-ordered asset server with synchronous compatibility and worker-backed streaming.
pub struct AssetServer {
    sources: Vec<SourceEntry>,
    loaders: HashMap<String, Arc<dyn ErasedLoader>>,
    asset_removers: HashMap<TypeId, AssetRemover>,
    assets: ErasedAssetMap,
    revisions: HashMap<(AssetId, u32), u64>,
    paths: HashMap<(TypeId, String), PathEntry>,
    dependency_graph: AssetDependencyGraph,
    next_id: u64,
    next_source_id: u64,
    #[cfg(any(feature = "hot_reload", debug_assertions))]
    last_poll: Instant,
    poll_interval: Duration,
    workers: Option<WorkerPool>,
    worker_count: usize,
    completion_tx: mpsc::SyncSender<LoadCompletion>,
    completion_rx: Mutex<mpsc::Receiver<LoadCompletion>>,
    pending_completions: VecDeque<LoadCompletion>,
    max_completions_per_poll: usize,
    max_completion_bytes_per_poll: Option<u64>,
    completions_processed_last_poll: usize,
    source_read_time_ns_last_poll: u64,
    decode_time_ns_last_poll: u64,
    source_read_time_ns_total: u64,
    decode_time_ns_total: u64,
    pending_events: Vec<AssetEvent>,
    strong_refs: Arc<Mutex<HashMap<(AssetId, u32), usize>>>,
    auto_evict_unused: bool,
    async_hot_reload: bool,
    cpu_memory_budget_bytes: Option<u64>,
    budget_evictions_cumulative: u64,
    next_load_sequence: u64,
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
            dependency_graph: AssetDependencyGraph::default(),
            next_id: 1,
            next_source_id: 1,
            #[cfg(any(feature = "hot_reload", debug_assertions))]
            last_poll: Instant::now(),
            poll_interval: Duration::from_millis(500),
            workers: None,
            worker_count,
            completion_tx,
            completion_rx: Mutex::new(completion_rx),
            pending_completions: VecDeque::new(),
            max_completions_per_poll: 8,
            max_completion_bytes_per_poll: Some(64 * 1024 * 1024),
            completions_processed_last_poll: 0,
            source_read_time_ns_last_poll: 0,
            decode_time_ns_last_poll: 0,
            source_read_time_ns_total: 0,
            decode_time_ns_total: 0,
            pending_events: Vec::new(),
            strong_refs: Arc::new(Mutex::new(HashMap::new())),
            auto_evict_unused: false,
            async_hot_reload: false,
            cpu_memory_budget_bytes: None,
            budget_evictions_cumulative: 0,
            next_load_sequence: 1,
            last_reload_errors: HashMap::new(),
        };
        server.register_loader(BytesLoader);
        server.register_loader(TextLoader);
        server
    }
}

mod cache;
mod config;
mod internal;
mod reload;
mod requests;
