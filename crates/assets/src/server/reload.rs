use super::*;

impl AssetServer {
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
                            loader.load(&bytes, &mut context).map(|asset| {
                                let estimated_size_bytes =
                                    loader.estimated_size_bytes(asset.as_ref());
                                (loader, asset, estimated_size_bytes)
                            })
                        }),
                    Err(error) => Err(LoadError::new(LoadErrorKind::Io, &path, error.to_string())),
                };
                let result = decoded.and_then(|(loader, asset, estimated_size_bytes)| {
                    if loader.asset_type() != type_id {
                        return Err(LoadError::new(
                            LoadErrorKind::TypeMismatch,
                            &path,
                            "reloaded value has a different asset type",
                        ));
                    }
                    loader
                        .store(&mut self.assets, id, generation, asset)
                        .map(|()| estimated_size_bytes)
                });
                match result {
                    Ok(estimated_size_bytes) => {
                        let load_sequence = self.allocate_load_sequence();
                        if let Some(entry) = self.paths.get_mut(&(type_id, path.clone())) {
                            entry.modified = modified;
                            entry.state = LoadState::Loaded;
                            entry.reload_pending = false;
                            entry.estimated_size_bytes = estimated_size_bytes;
                            entry.load_sequence = load_sequence;
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
                let cancellation = CancellationToken::default();
                let task_cancellation = cancellation.clone();
                let task: WorkerTask = Box::new(move || {
                    let mut source_read_time_ns = 0_u64;
                    let mut decode_time_ns = 0_u64;
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        if task_cancellation.is_cancelled() {
                            return Err(LoadError::new(
                                LoadErrorKind::Cancelled,
                                &request_path,
                                "hot reload was cancelled",
                            ));
                        }
                        let modified_before = request_source.source.modified(&request_path);
                        let source_started = Instant::now();
                        let read_result = request_source
                            .source
                            .read_cancellable(&request_path, &task_cancellation)
                            .map_err(|error| {
                                if error.kind() == std::io::ErrorKind::Interrupted
                                    && task_cancellation.is_cancelled()
                                {
                                    LoadError::new(
                                        LoadErrorKind::Cancelled,
                                        &request_path,
                                        "hot reload source read was cancelled",
                                    )
                                } else {
                                    LoadError::new(
                                        LoadErrorKind::Io,
                                        &request_path,
                                        error.to_string(),
                                    )
                                }
                            });
                        source_read_time_ns =
                            source_started.elapsed().as_nanos().min(u64::MAX as u128) as u64;
                        read_result.and_then(|bytes| {
                            task_cancellation.check(&request_path)?;
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
                            let decode_started = Instant::now();
                            let decoded = request_loader
                                .load_cancellable(&bytes, &mut context, &task_cancellation)
                                .map(|value| {
                                    let estimated_size_bytes =
                                        request_loader.estimated_size_bytes(value.as_ref());
                                    DecodedAsset {
                                        loader: Arc::clone(&request_loader),
                                        value,
                                        source_id: request_source.id,
                                        modified: modified_after,
                                        estimated_size_bytes,
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
                            "asset source or loader panicked during hot reload",
                        ))
                    });
                    let _ = completion_tx.send(LoadCompletion {
                        type_id,
                        path: request_path,
                        id,
                        generation,
                        reload: Some(reload),
                        source_read_time_ns,
                        decode_time_ns,
                        result,
                    });
                });
                // Hot reload is deliberately slightly below an explicit fresh-load request with
                // the same retention priority, so changed files cannot starve visible assets.
                let reload_priority = self
                    .paths
                    .get(&key)
                    .map_or(-1, |entry| entry.priority.saturating_sub(1));
                let queue_result = self.workers.as_ref().map(|workers| {
                    workers.execute((id.get(), generation), reload_priority, cancellation, task)
                });
                match queue_result {
                    Some(Ok(())) => {
                        if let Some(entry) = self.paths.get_mut(&key) {
                            entry.reload_pending = true;
                            entry.reload_token = token;
                        }
                    }
                    Some(Err(WorkerSubmitError::Full)) => {
                        // Leave the old modification stamp in place. The next hot-reload poll will
                        // retry queue admission after other pending asset work has drained.
                    }
                    Some(Err(WorkerSubmitError::Disconnected)) => {
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
}
