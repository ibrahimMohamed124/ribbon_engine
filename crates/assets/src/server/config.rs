use super::*;

impl AssetServer {
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

    /// Return the normalized source path for a generation-checked handle, if it was loaded by path.
    /// This supports serializing asset references by path rather than persisting runtime IDs.
    pub fn path<T: Asset>(&self, handle: Handle<T>) -> Option<&str> {
        self.paths.iter().find_map(|((type_id, path), entry)| {
            (*type_id == TypeId::of::<T>()
                && entry.id == handle.asset_id()
                && entry.generation == handle.generation())
            .then_some(path.as_str())
        })
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

    /// Set the decoded CPU-cache budget in bytes. `None` disables budget-based eviction.
    ///
    /// The budget is soft with respect to pinned assets: values protected by [`AssetRef`] are never
    /// evicted to satisfy it. When the budget is enabled, [`poll_pending_loads`](Self::poll_pending_loads)
    /// evicts the lowest-priority, oldest unleased values until unpinned residency fits the
    /// budget.
    /// Call [`enforce_cpu_memory_budget`](Self::enforce_cpu_memory_budget) to apply a change
    /// immediately to values already resident in a synchronous-only application.
    pub fn set_cpu_memory_budget_bytes(&mut self, budget_bytes: Option<u64>) {
        self.cpu_memory_budget_bytes = budget_bytes;
    }

    /// Snapshot estimated decoded CPU cache residency and budget pressure.
    pub fn cpu_memory_stats(&self) -> AssetMemoryStats {
        let lease_counts = self.strong_refs.lock();
        let counts_poisoned = lease_counts.is_err();
        let counts = lease_counts.as_deref().ok();
        let mut stats = AssetMemoryStats {
            budget_bytes: self.cpu_memory_budget_bytes,
            budget_evictions_cumulative: self.budget_evictions_cumulative,
            ..AssetMemoryStats::default()
        };
        for ((_, _), entry) in &self.paths {
            if entry.state != LoadState::Loaded {
                continue;
            }
            stats.loaded_assets = stats.loaded_assets.saturating_add(1);
            stats.resident_bytes_estimate = stats
                .resident_bytes_estimate
                .saturating_add(entry.estimated_size_bytes);
            // If lease accounting is poisoned, conservatively classify the value as pinned.
            let lease_count = counts
                .and_then(|counts| counts.get(&(entry.id, entry.generation)).copied())
                .unwrap_or(if counts_poisoned { usize::MAX } else { 0 });
            if lease_count > 0 {
                stats.pinned_assets = stats.pinned_assets.saturating_add(1);
                stats.pinned_bytes_estimate = stats
                    .pinned_bytes_estimate
                    .saturating_add(entry.estimated_size_bytes);
            } else {
                stats.unpinned_bytes_estimate = stats
                    .unpinned_bytes_estimate
                    .saturating_add(entry.estimated_size_bytes);
            }
        }
        stats.over_budget_bytes = stats.budget_bytes.map_or(0, |budget| {
            stats.unpinned_bytes_estimate.saturating_sub(budget)
        });
        stats
    }

    /// Snapshot worker scheduling pressure without exposing worker queues or task closures.
    pub fn worker_stats(&self) -> AssetWorkerStats {
        let (queued_jobs, active_workers) = self
            .workers
            .as_ref()
            .map(WorkerPool::stats)
            .unwrap_or((0, 0));
        AssetWorkerStats {
            queued_jobs,
            active_workers,
            worker_count: self.worker_count,
            queue_capacity: MAX_QUEUED_ASSET_LOADS,
            completion_capacity: MAX_PENDING_ASSET_COMPLETIONS * 2,
            pending_completions: self.pending_completions.len(),
            completions_processed_last_poll: self.completions_processed_last_poll,
            source_read_time_ns_last_poll: self.source_read_time_ns_last_poll,
            decode_time_ns_last_poll: self.decode_time_ns_last_poll,
            source_read_time_ns_total: self.source_read_time_ns_total,
            decode_time_ns_total: self.decode_time_ns_total,
            max_completions_per_poll: self.max_completions_per_poll,
            max_completion_bytes_per_poll: self.max_completion_bytes_per_poll,
        }
    }
}
