use super::*;

impl AssetServer {
    /// Request an asset with a worker-scheduling and cache-retention priority.
    /// Higher values are scheduled first; queued work ages over time so background requests
    /// cannot starve forever. Raising a priority also updates a job that has not started yet.
    pub fn request_with_priority<T: Asset>(
        &mut self,
        path: impl AsRef<str>,
        priority: i32,
    ) -> Result<Handle<T>, LoadError> {
        self.request_with_priority_internal::<T>(path, priority)
    }

    /// Retry a failed asynchronous request while assigning its cache-retention priority.
    pub fn retry_with_priority<T: Asset>(
        &mut self,
        path: impl AsRef<str>,
        priority: i32,
    ) -> Result<Handle<T>, LoadError> {
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
        self.request_with_priority::<T>(path, priority)
    }

    /// Request and strongly pin an asset while assigning its cache-retention priority.
    pub fn request_ref_with_priority<T: Asset>(
        &mut self,
        path: impl AsRef<str>,
        priority: i32,
    ) -> Result<AssetRef<T>, LoadError> {
        let handle = self.request_with_priority::<T>(path, priority)?;
        self.strong_ref(handle).ok_or_else(|| {
            LoadError::new(
                LoadErrorKind::WorkerUnavailable,
                "<asset lease>",
                "could not create a strong lease for the requested handle",
            )
        })
    }

    /// Retry a failed asset request and immediately retain the resulting generation.
    pub fn retry_ref_with_priority<T: Asset>(
        &mut self,
        path: impl AsRef<str>,
        priority: i32,
    ) -> Result<AssetRef<T>, LoadError> {
        let handle = self.retry_with_priority::<T>(path, priority)?;
        self.strong_ref(handle).ok_or_else(|| {
            LoadError::new(
                LoadErrorKind::WorkerUnavailable,
                "<asset lease>",
                "could not create a strong lease for the retried handle",
            )
        })
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
                let estimated_size_bytes = loader.estimated_size_bytes(value.as_ref());
                let memory_breakdown = loader.memory_breakdown(value.as_ref());
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
                let load_sequence = self.allocate_load_sequence();
                if let Some(entry) = self.paths.get_mut(&key) {
                    entry.source_id = Some(source_id);
                    entry.modified = modified;
                    entry.state = LoadState::Loaded;
                    entry.reload_pending = false;
                    entry.estimated_size_bytes = estimated_size_bytes;
                    entry.memory_breakdown = memory_breakdown;
                    entry.load_sequence = load_sequence;
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
        self.request_with_priority_internal::<T>(path, 0)
    }

    fn request_with_priority_internal<T: Asset>(
        &mut self,
        path: impl AsRef<str>,
        priority: i32,
    ) -> Result<Handle<T>, LoadError> {
        self.async_hot_reload = true;
        let path = path.as_ref().to_owned();
        let key = (TypeId::of::<T>(), path.clone());
        if let Some((id, generation, state, old_priority)) = self.paths.get(&key).map(|entry| {
            (
                entry.id,
                entry.generation,
                entry.state.clone(),
                entry.priority,
            )
        }) {
            match state {
                LoadState::Loaded => {
                    if priority > old_priority {
                        if let Some(entry) = self.paths.get_mut(&key) {
                            entry.priority = priority;
                        }
                    }
                    return Ok(Handle::new(id, generation));
                }
                LoadState::Loading => {
                    if priority > old_priority {
                        if let Some(entry) = self.paths.get_mut(&key) {
                            entry.priority = priority;
                        }
                        if let Some(workers) = self.workers.as_ref() {
                            workers.set_priority((id.get(), generation), priority);
                        }
                    }
                    return Ok(Handle::new(id, generation));
                }
                LoadState::Failed(error) => return Err(error),
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
        if let Some(entry) = self.paths.get_mut(&key) {
            // A new generation receives the priority of its new request. Existing in-flight and
            // resident paths use max() above so a later consumer can only raise their priority.
            entry.priority = priority;
        }
        let effective_priority = self
            .paths
            .get(&key)
            .map_or(priority, |entry| entry.priority);
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
        let cancellation = self
            .paths
            .get(&key)
            .map(|entry| entry.cancellation.clone())
            .unwrap_or_default();
        let task_cancellation = cancellation.clone();
        let type_id = TypeId::of::<T>();
        let task: WorkerTask = Box::new(move || {
            // Third-party AssetSource and AssetLoader implementations are user code. Convert a
            // panic into a typed completion so the path cannot remain Loading forever and the
            // worker thread remains available for later requests.
            let mut source_read_time_ns = 0_u64;
            let mut decode_time_ns = 0_u64;
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let source_started = Instant::now();
                let read_result =
                    read_from_sources_cancellable(&sources, &request_path, &task_cancellation);
                source_read_time_ns =
                    source_started.elapsed().as_nanos().min(u64::MAX as u128) as u64;
                read_result.and_then(|(bytes, source_id, modified)| {
                    task_cancellation.check(&request_path)?;
                    let mut context = LoadContext {
                        path: request_path.clone(),
                    };
                    let decode_started = Instant::now();
                    let decoded = request_loader
                        .load_cancellable(&bytes, &mut context, &task_cancellation)
                        .map(|value| {
                            let estimated_size_bytes =
                                request_loader.estimated_size_bytes(value.as_ref());
                            let memory_breakdown = request_loader.memory_breakdown(value.as_ref());
                            DecodedAsset {
                                loader: Arc::clone(&request_loader),
                                value,
                                source_id,
                                modified,
                                estimated_size_bytes,
                                memory_breakdown,
                            }
                        });
                    decode_time_ns =
                        decode_started.elapsed().as_nanos().min(u64::MAX as u128) as u64;
                    decoded
                })
            }))
            .unwrap_or_else(|_| {
                Err(LoadError::new(
                    LoadErrorKind::Decode,
                    &request_path,
                    "asset source or loader panicked while processing the request",
                ))
            });
            let _ = completion_tx.send(LoadCompletion {
                type_id,
                path: request_path,
                id,
                generation,
                reload: None,
                source_read_time_ns,
                decode_time_ns,
                result,
            });
        });
        let queue_result = self.workers.as_ref().map(|workers| {
            workers.execute(
                (id.get(), generation),
                effective_priority,
                cancellation,
                task,
            )
        });
        match queue_result {
            Some(Ok(())) => {}
            Some(Err(WorkerSubmitError::Full)) => {
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
            Some(Err(WorkerSubmitError::Disconnected)) => {
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

    /// Cancel an asynchronous request by exact handle generation. Queued work is removed without
    /// running; active source reads/loaders receive a cooperative token and their results can never
    /// be committed. The path becomes Unloaded and a later request receives a new generation.
    pub fn cancel<T: Asset>(&mut self, handle: Handle<T>) -> bool {
        let key = self.paths.iter().find_map(|((type_id, path), entry)| {
            (*type_id == TypeId::of::<T>()
                && entry.id == handle.asset_id()
                && entry.generation == handle.generation()
                && entry.state == LoadState::Loading)
                .then_some((*type_id, path.clone()))
        });
        let Some(key) = key else {
            return false;
        };
        let Some((id, generation, cancellation)) = self
            .paths
            .get(&key)
            .map(|entry| (entry.id, entry.generation, entry.cancellation.clone()))
        else {
            return false;
        };
        cancellation.cancel();
        if let Some(workers) = self.workers.as_ref() {
            let _ = workers.cancel((id.get(), generation));
        }
        if let Some(entry) = self.paths.get_mut(&key) {
            if entry.id == id && entry.generation == generation {
                entry.state = LoadState::Unloaded;
                entry.reload_pending = false;
                entry.estimated_size_bytes = 0;
                entry.memory_breakdown = AssetMemoryBreakdown::default();
                entry.load_sequence = 0;
            }
        }
        self.pending_events.push(AssetEvent::Cancelled {
            id: id.get(),
            generation,
            path: key.1,
        });
        true
    }

    /// Cancel a pending load only when no other consumer holds a strong lease. This is the safe
    /// primitive for streaming-region teardown when paths may be shared by several regions.
    pub fn cancel_if_unleased<T: Asset>(&mut self, handle: Handle<T>) -> bool {
        if self.strong_ref_count(handle) != 0 {
            return false;
        }
        self.cancel(handle)
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
                && entry.id == handle.asset_id()
                && entry.generation == handle.generation()
                && matches!(&entry.state, LoadState::Loaded | LoadState::Loading)
        });
        if !is_live {
            return None;
        }
        let key = (handle.asset_id(), handle.generation());
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
                    .get(&(handle.asset_id(), handle.generation()))
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
                let fallback = &default as &(dyn Any + Send + Sync);
                let loader = self
                    .loaders
                    .get(&extension_for(&path))
                    .filter(|loader| loader.asset_type() == TypeId::of::<T>());
                let estimated_size_bytes = loader
                    .map(|loader| loader.estimated_size_bytes(fallback))
                    .unwrap_or(std::mem::size_of_val(&default) as u64);
                let memory_breakdown = loader
                    .map(|loader| loader.memory_breakdown(fallback))
                    .unwrap_or_else(|| AssetMemoryBreakdown::other(estimated_size_bytes));
                self.storage_mut::<T>().insert(handle, default);
                self.revisions.insert((id, generation), 1);
                let load_sequence = self.allocate_load_sequence();
                if let Some(entry) = self.paths.get_mut(&key) {
                    entry.state = LoadState::Loaded;
                    entry.reload_pending = false;
                    entry.estimated_size_bytes = estimated_size_bytes;
                    entry.memory_breakdown = memory_breakdown;
                    entry.load_sequence = load_sequence;
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

    /// Set the maximum number of decoded results committed by one `poll_pending_loads` call.
    /// The value is clamped to at least one so completed work cannot be permanently starved.
    pub fn set_max_completions_per_poll(&mut self, max: usize) {
        self.max_completions_per_poll = max.max(1);
    }

    /// Set a soft decoded-byte budget for one completion poll. One oversized first result is
    /// permitted to prevent starvation; remaining results wait for later frames. `None` disables
    /// the byte limit.
    pub fn set_max_completion_bytes_per_poll(&mut self, max_bytes: Option<u64>) {
        self.max_completion_bytes_per_poll = max_bytes;
    }

    /// Register a typed asset dependency edge. The graph is declarative metadata: it validates
    /// ordering/cycles and exposes dependency-first load order; callers decide when to submit each
    /// typed request so dependencies retain their own lease and scheduling policy.
    pub fn add_dependency<P: Asset, D: Asset>(
        &mut self,
        parent_path: impl AsRef<str>,
        dependency_path: impl AsRef<str>,
        kind: AssetDependencyKind,
    ) -> Result<(), AssetDependencyError> {
        self.dependency_graph.add::<P, D>(
            parent_path.as_ref().to_owned(),
            dependency_path.as_ref().to_owned(),
            kind,
        )
    }

    /// Replace all dependency edges for one typed asset. The graph update is transactional:
    /// invalid duplicates or cycles leave the previous dependency set intact.
    pub fn replace_dependencies<P: Asset>(
        &mut self,
        parent_path: impl AsRef<str>,
        dependencies: impl IntoIterator<Item = AssetDependency>,
    ) -> Result<(), AssetDependencyError> {
        self.dependency_graph
            .replace::<P>(parent_path.as_ref().to_owned(), dependencies)
    }

    /// Dependency graph for inspection, bulk validation, and dependency-first scheduling.
    pub fn dependency_graph(&self) -> &AssetDependencyGraph {
        &self.dependency_graph
    }

    /// Return dependency-first load order for a typed root asset, including the root last.
    pub fn dependency_load_order<T: Asset>(
        &self,
        path: impl AsRef<str>,
    ) -> Result<Vec<AssetDependencyKey>, AssetDependencyError> {
        self.dependency_graph.order_for::<T>(path)
    }

    /// Inspect the transitive required dependencies for a typed asset. Results are dependency-first
    /// and exclude optional dependencies and the requested root. Missing requests are reported as
    /// `NotRequested`; this API does not implicitly submit typed loads on the caller's behalf.
    pub fn required_dependency_statuses<T: Asset>(
        &self,
        path: impl AsRef<str>,
    ) -> Result<Vec<AssetDependencyStatus>, AssetDependencyError> {
        let keys = self
            .dependency_graph
            .required_order_for::<T>(path.as_ref())?;
        Ok(keys
            .into_iter()
            .map(|key| {
                let state = self
                    .paths
                    .get(&(key.type_id(), key.path.clone()))
                    .map(|entry| match &entry.state {
                        LoadState::Loading => AssetDependencyLoadState::Loading,
                        LoadState::Loaded => AssetDependencyLoadState::Loaded,
                        LoadState::Unloaded => AssetDependencyLoadState::Unloaded,
                        LoadState::Failed(error) => AssetDependencyLoadState::Failed(error.clone()),
                    })
                    .unwrap_or(AssetDependencyLoadState::NotRequested);
                AssetDependencyStatus { key, state }
            })
            .collect())
    }

    /// Whether every transitive required dependency has committed into the CPU asset cache.
    /// GPU readiness remains the responsibility of the owning renderer.
    pub fn required_dependencies_ready<T: Asset>(
        &self,
        path: impl AsRef<str>,
    ) -> Result<bool, AssetDependencyError> {
        let statuses = self.required_dependency_statuses::<T>(path)?;
        Ok(statuses
            .iter()
            .all(|status| status.state == AssetDependencyLoadState::Loaded))
    }

    /// Return the handle for the current generation associated with a typed path.
    ///
    /// Unlike retaining a handle returned by an old request, this lookup follows the current
    /// path generation after eviction/retry. It is useful for reconciling long-lived leases with
    /// asynchronous reloads without treating a still-loading current generation as stale.
    pub fn current_handle<T: Asset>(&self, path: impl AsRef<str>) -> Option<Handle<T>> {
        self.paths
            .get(&(TypeId::of::<T>(), path.as_ref().to_owned()))
            .map(|entry| Handle::new(entry.id, entry.generation))
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
        self.revisions
            .get(&(handle.asset_id(), handle.generation()))
            .copied()
    }

    /// Get the last hot-reload error for a handle without discarding its last good value.
    pub fn last_reload_error<T>(&self, handle: Handle<T>) -> Option<&LoadError> {
        self.last_reload_errors
            .get(&(handle.asset_id(), handle.generation()))
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
            .entry((handle.asset_id(), handle.generation()))
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
}
