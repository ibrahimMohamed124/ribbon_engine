use super::*;

impl AssetServer {
    /// Commit completed worker requests and return lifecycle events for the caller's thread.
    ///
    /// The method does not create GPU resources. A rendering plugin should observe loaded CPU-side
    /// assets and upload them from its render pass, where its GPU device/queue is available.
    pub fn poll_pending_loads(&mut self) -> Vec<AssetEvent> {
        self.source_read_time_ns_last_poll = 0;
        self.decode_time_ns_last_poll = 0;
        // Keep the completion staging queue bounded independently from the worker channel. A
        // producer can never make the main thread allocate an unbounded vector of decoded assets.
        if let Ok(receiver) = self.completion_rx.lock() {
            while self.pending_completions.len() < MAX_PENDING_ASSET_COMPLETIONS {
                match receiver.try_recv() {
                    Ok(completion) => self.pending_completions.push_back(completion),
                    Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected) => break,
                }
            }
        }

        let mut processed = 0usize;
        let mut processed_bytes = 0u64;
        while processed < self.max_completions_per_poll {
            let Some(next_bytes) = self.pending_completions.front().map(|completion| {
                completion
                    .result
                    .as_ref()
                    .ok()
                    .map(|decoded| decoded.estimated_size_bytes)
                    .unwrap_or(0)
            }) else {
                break;
            };
            if let Some(byte_budget) = self.max_completion_bytes_per_poll {
                // Always admit one result so a single large asset cannot starve forever.
                if processed > 0 && processed_bytes.saturating_add(next_bytes) > byte_budget {
                    break;
                }
            }
            let Some(completion) = self.pending_completions.pop_front() else {
                break;
            };
            self.source_read_time_ns_last_poll = self
                .source_read_time_ns_last_poll
                .saturating_add(completion.source_read_time_ns);
            self.decode_time_ns_last_poll = self
                .decode_time_ns_last_poll
                .saturating_add(completion.decode_time_ns);
            self.source_read_time_ns_total = self
                .source_read_time_ns_total
                .saturating_add(completion.source_read_time_ns);
            self.decode_time_ns_total = self
                .decode_time_ns_total
                .saturating_add(completion.decode_time_ns);
            processed = processed.saturating_add(1);
            processed_bytes = processed_bytes.saturating_add(next_bytes);

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
                let DecodedAsset {
                    loader,
                    value,
                    source_id,
                    modified,
                    estimated_size_bytes,
                    memory_breakdown,
                } = decoded;
                if loader.asset_type() != completion.type_id {
                    return Err(LoadError::new(
                        LoadErrorKind::TypeMismatch,
                        &completion.path,
                        "asynchronous decoder returned a different asset type",
                    ));
                }
                loader
                    .store(
                        &mut self.assets,
                        completion.id,
                        completion.generation,
                        value,
                    )
                    .map(|()| (source_id, modified, estimated_size_bytes, memory_breakdown))
            });

            match result {
                Ok((source_id, modified, estimated_size_bytes, memory_breakdown)) => {
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

        self.completions_processed_last_poll = processed;
        if self.auto_evict_unused {
            self.evict_unused();
        }
        self.enforce_cpu_memory_budget();
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
                    entry.estimated_size_bytes = 0;
                    entry.memory_breakdown = AssetMemoryBreakdown::default();
                    entry.load_sequence = 0;
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

    /// Enforce the configured CPU cache budget by evicting unleased low-priority entries first.
    ///
    /// The budget counts only unpinned cached values. Strongly leased assets are never evicted;
    /// when pinned values dominate the working set, `cpu_memory_stats` exposes that separately.
    /// Returns the number of values evicted by this call.
    pub fn enforce_cpu_memory_budget(&mut self) -> usize {
        let Some(budget_bytes) = self.cpu_memory_budget_bytes else {
            return 0;
        };
        let mut unpinned_bytes = self.cpu_memory_stats().unpinned_bytes_estimate;
        if unpinned_bytes <= budget_bytes {
            return 0;
        }

        let counts = match self.strong_refs.lock() {
            Ok(counts) => counts,
            // Never free cache values if lease accounting cannot be trusted.
            Err(_) => return 0,
        };
        let mut candidates: Vec<_> = self
            .paths
            .iter()
            .filter_map(|((type_id, path), entry)| {
                let unleased = counts
                    .get(&(entry.id, entry.generation))
                    .copied()
                    .unwrap_or(0)
                    == 0;
                (entry.state == LoadState::Loaded && unleased && entry.estimated_size_bytes > 0)
                    .then_some((
                        entry.priority,
                        entry.load_sequence,
                        *type_id,
                        path.clone(),
                        entry.id,
                        entry.generation,
                        entry.estimated_size_bytes,
                    ))
            })
            .collect();
        drop(counts);
        // Lower priority values leave first; oldest resident entries break ties (FIFO).
        candidates.sort_by_key(|(priority, sequence, _, _, _, _, _)| (*priority, *sequence));

        let mut evicted = 0usize;
        for (_, _, type_id, path, id, generation, estimated_size_bytes) in candidates {
            if unpinned_bytes <= budget_bytes {
                break;
            }
            let still_unleased = self
                .strong_refs
                .lock()
                .map(|counts| counts.get(&(id, generation)).copied().unwrap_or(0) == 0)
                .unwrap_or(false);
            if !still_unleased {
                continue;
            }
            let removed = self
                .asset_removers
                .get(&type_id)
                .copied()
                .is_some_and(|remove| remove(&mut self.assets, id, generation));
            if !removed {
                continue;
            }
            if let Some(entry) = self.paths.get_mut(&(type_id, path.clone())) {
                if entry.id != id || entry.generation != generation {
                    continue;
                }
                entry.state = LoadState::Unloaded;
                entry.source_id = None;
                entry.modified = None;
                entry.reload_pending = false;
                entry.estimated_size_bytes = 0;
                entry.memory_breakdown = AssetMemoryBreakdown::default();
                entry.load_sequence = 0;
            }
            self.revisions.remove(&(id, generation));
            self.pending_events.push(AssetEvent::Unloaded {
                id: id.get(),
                generation,
                path,
            });
            self.last_reload_errors.remove(&(id, generation));
            unpinned_bytes = unpinned_bytes.saturating_sub(estimated_size_bytes);
            self.budget_evictions_cumulative = self.budget_evictions_cumulative.saturating_add(1);
            evicted = evicted.saturating_add(1);
        }
        evicted
    }
}
