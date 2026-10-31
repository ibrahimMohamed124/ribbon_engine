use super::*;

impl AssetServer {
    pub(super) fn begin_loading(
        &mut self,
        key: &(TypeId, String),
        extension: &str,
    ) -> (AssetId, u32) {
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
                entry.estimated_size_bytes = 0;
                entry.load_sequence = 0;
                entry.cancellation = CancellationToken::default();
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
                estimated_size_bytes: 0,
                load_sequence: 0,
                priority: 0,
                cancellation: CancellationToken::default(),
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

    pub(super) fn allocate_load_sequence(&mut self) -> u64 {
        let sequence = self.next_load_sequence;
        self.next_load_sequence = self.next_load_sequence.saturating_add(1);
        sequence
    }

    pub(super) fn record_request_failure(
        &mut self,
        key: &(TypeId, String),
        extension: &str,
        error: LoadError,
    ) -> LoadError {
        let (id, generation) = self.begin_loading(key, extension);
        self.fail_path(key, id, generation, &error);
        error
    }

    pub(super) fn fail_path(
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

    pub(super) fn load_bytes(
        &self,
        path: &str,
    ) -> Result<(Vec<u8>, u64, Option<SystemTime>), LoadError> {
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

    pub(crate) fn storage_mut<T: Asset>(&mut self) -> &mut Assets<T> {
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
