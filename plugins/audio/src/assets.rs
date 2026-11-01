//! Integration with Ribbon's typed asset pipeline.
//!
//! Encoded bytes are shared between asset snapshots and playback requests. The asset server owns
//! the decoded asset; `AudioAssetHandle` retains a strong `AssetRef` so cache eviction cannot
//! invalidate a sound which is still referenced by gameplay. Successful hot reload increments the
//! server revision and is observed by [`AudioAssetHandle::refresh`].

use std::sync::Arc;

use ribbon_assets::{
    AssetLoader, AssetRef, AssetServer, LoadContext, LoadError, LoadErrorKind, LoadState,
};

/// Encoded audio source kept by Ribbon's asset cache.
#[derive(Clone, Debug)]
pub struct EncodedAudioAsset {
    pub path: String,
    pub bytes: Arc<[u8]>,
}

impl EncodedAudioAsset {
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

#[derive(Default)]
pub struct AudioAssetLoader;

impl AssetLoader for AudioAssetLoader {
    type Asset = EncodedAudioAsset;

    fn extensions(&self) -> &[&str] {
        &["wav", "ogg", "oga", "mp3", "flac", "aiff", "aif"]
    }

    fn load(&self, bytes: &[u8], context: &mut LoadContext) -> Result<Self::Asset, LoadError> {
        if bytes.is_empty() {
            return Err(LoadError::new(
                LoadErrorKind::Decode,
                &context.path,
                "audio file is empty",
            ));
        }
        let extension = context
            .path
            .rsplit('.')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        if !self.extensions().contains(&extension.as_str()) {
            return Err(LoadError::new(
                LoadErrorKind::NoLoader,
                &context.path,
                "unsupported audio extension",
            ));
        }
        Ok(EncodedAudioAsset {
            path: context.path.clone(),
            bytes: Arc::from(bytes),
        })
    }

    fn estimated_size_bytes(&self, asset: &Self::Asset) -> u64 {
        (std::mem::size_of::<Self::Asset>() + asset.path.capacity() + asset.bytes.len()) as u64
    }
}

/// Register Ribbon's typed audio loader on an existing asset server.
pub fn register_audio_assets(server: &mut AssetServer) {
    server.register_loader(AudioAssetLoader);
}

/// Result of polling a typed audio asset for initial readiness or a hot-reload revision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AudioAssetPoll {
    /// The asynchronous request is still being loaded.
    Loading,
    /// The current revision is ready. `changed` is true when a new encoded snapshot was captured.
    Ready { revision: u64, changed: bool },
    /// A reload failed; the last known-good snapshot (when present) remains available.
    ReloadFailed(LoadError),
    /// Initial loading failed or the current generation is no longer available.
    Failed(LoadError),
}

/// Immutable, revision-tagged encoded source suitable for passing to the audio worker.
#[derive(Clone, Debug)]
pub struct AudioAssetSnapshot {
    pub path: Arc<str>,
    pub asset_id: u64,
    pub generation: u32,
    pub revision: u64,
    pub bytes: Arc<[u8]>,
}

/// Strong typed handle to an audio asset stored by `ribbon_assets`.
///
/// Clone this handle to share the same pin. It keeps the asset resident until the final clone is
/// dropped. Call [`refresh`](Self::refresh) after `AssetServer::poll_pending_loads` to observe a
/// successful hot reload. A failed reload does not discard the last good bytes.
pub struct AudioAssetHandle {
    lease: AssetRef<EncodedAudioAsset>,
    path: Arc<str>,
    revision: Option<u64>,
    bytes: Option<Arc<[u8]>>,
    last_reload_error: Option<LoadError>,
}

impl Clone for AudioAssetHandle {
    fn clone(&self) -> Self {
        Self {
            lease: self.lease.clone(),
            path: Arc::clone(&self.path),
            revision: self.revision,
            bytes: self.bytes.as_ref().map(Arc::clone),
            last_reload_error: self.last_reload_error.clone(),
        }
    }
}

impl std::fmt::Debug for AudioAssetHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioAssetHandle")
            .field("path", &self.path)
            .field("asset_id", &self.lease.asset_id().get())
            .field("generation", &self.lease.generation())
            .field("revision", &self.revision)
            .field("ready", &self.bytes.is_some())
            .field("last_reload_error", &self.last_reload_error)
            .finish()
    }
}

impl AudioAssetHandle {
    /// Synchronously load and pin an audio asset.
    pub fn load(server: &mut AssetServer, path: impl AsRef<str>) -> Result<Self, LoadError> {
        let path = path.as_ref().to_owned();
        let lease = server.load_ref::<EncodedAudioAsset>(&path)?;
        let mut result = Self {
            lease,
            path: Arc::from(path),
            revision: None,
            bytes: None,
            last_reload_error: None,
        };
        let _ = result.refresh(server);
        if result.bytes.is_none() {
            return Err(LoadError::new(
                LoadErrorKind::Decode,
                &result.path,
                "audio asset loaded without a readable encoded payload",
            ));
        }
        Ok(result)
    }

    /// Queue an asynchronous load and pin the pending or loaded generation.
    pub fn request(server: &mut AssetServer, path: impl AsRef<str>) -> Result<Self, LoadError> {
        let path = path.as_ref().to_owned();
        let lease = server.request_ref::<EncodedAudioAsset>(&path)?;
        Ok(Self {
            lease,
            path: Arc::from(path),
            revision: None,
            bytes: None,
            last_reload_error: None,
        })
    }

    /// Stable typed asset handle used to query the asset server.
    pub fn handle(&self) -> ribbon_assets::Handle<EncodedAudioAsset> {
        self.lease.handle()
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn revision(&self) -> Option<u64> {
        self.revision
    }

    pub fn is_ready(&self) -> bool {
        self.bytes.is_some()
    }

    pub fn last_reload_error(&self) -> Option<&LoadError> {
        self.last_reload_error.as_ref()
    }

    /// Get a shared copy of the last good encoded payload, without copying the bytes.
    pub fn encoded_bytes(&self) -> Option<Arc<[u8]>> {
        self.bytes.as_ref().map(Arc::clone)
    }

    /// Build an immutable playback snapshot. Returns `None` while an asynchronous load is pending.
    pub fn snapshot(&self) -> Option<AudioAssetSnapshot> {
        Some(AudioAssetSnapshot {
            path: Arc::clone(&self.path),
            asset_id: self.lease.asset_id().get(),
            generation: self.lease.generation(),
            revision: self.revision?,
            bytes: self.bytes.as_ref()?.clone(),
        })
    }

    /// Refresh encoded data after asset-server polling.
    ///
    /// A successful reload replaces the shared snapshot. A failed reload reports an error while
    /// retaining the previous good snapshot, allowing active and future voices to keep playing.
    pub fn refresh(&mut self, server: &AssetServer) -> AudioAssetPoll {
        let handle = self.lease.handle();
        if let Some(error) = server.last_reload_error(handle).cloned() {
            self.last_reload_error = Some(error.clone());
            return if self.bytes.is_some() {
                AudioAssetPoll::ReloadFailed(error)
            } else {
                AudioAssetPoll::Failed(error)
            };
        }

        if let Some(revision) = server.revision(handle) {
            if self.revision != Some(revision) {
                let Some(asset) = server.get(handle) else {
                    return AudioAssetPoll::Loading;
                };
                self.bytes = Some(Arc::clone(&asset.bytes));
                self.revision = Some(revision);
                self.last_reload_error = None;
                return AudioAssetPoll::Ready {
                    revision,
                    changed: true,
                };
            }
            self.last_reload_error = None;
            return AudioAssetPoll::Ready {
                revision,
                changed: false,
            };
        }

        match server.state::<EncodedAudioAsset>(&self.path) {
            Some(LoadState::Loading) => AudioAssetPoll::Loading,
            Some(LoadState::Failed(error)) => AudioAssetPoll::Failed(error.clone()),
            Some(LoadState::Unloaded) | None => AudioAssetPoll::Failed(LoadError::new(
                LoadErrorKind::Cancelled,
                &self.path,
                "audio asset is no longer resident or its request was cancelled",
            )),
            Some(LoadState::Loaded) => AudioAssetPoll::Loading,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ribbon_assets::MemorySource;

    #[test]
    fn preserves_audio_bytes_and_rejects_empty_files() {
        let loader = AudioAssetLoader;
        let mut context = LoadContext {
            path: "sfx/door.wav".into(),
        };
        let asset = loader.load(b"RIFF", &mut context).unwrap();
        assert_eq!(asset.bytes.as_ref(), b"RIFF");
        assert!(loader.load(b"", &mut context).is_err());
    }

    #[test]
    fn rejects_unknown_extensions() {
        let mut context = LoadContext {
            path: "audio/thing.txt".into(),
        };
        assert!(AudioAssetLoader.load(b"x", &mut context).is_err());
    }

    #[test]
    fn rejects_opus_until_the_selected_decoder_supports_it() {
        let mut context = LoadContext {
            path: "audio/thing.opus".into(),
        };
        let result = AudioAssetLoader.load(b"not really opus", &mut context);
        assert!(result.is_err());
    }

    #[test]
    fn strong_audio_handle_refreshes_on_revision_and_pins_asset_storage() {
        let source = MemorySource::default();
        source.insert("sfx/door.wav", b"first".to_vec());
        let mut server = AssetServer::default();
        server.add_source(0, source);
        register_audio_assets(&mut server);

        let mut audio = AudioAssetHandle::load(&mut server, "sfx/door.wav").unwrap();
        let first = audio.snapshot().unwrap();
        assert_eq!(first.bytes.as_ref(), b"first");
        assert_eq!(server.evict_unused(), 0);

        let handle = audio.handle();
        server.get_mut(handle).unwrap().bytes = Arc::from(&b"second revision"[..]);
        match audio.refresh(&server) {
            AudioAssetPoll::Ready { changed, .. } => assert!(changed),
            state => panic!("expected updated ready asset, got {state:?}"),
        }
        let second = audio.snapshot().unwrap();
        assert!(second.revision > first.revision);
        assert_eq!(second.bytes.as_ref(), b"second revision");
        assert_eq!(server.evict_unused(), 0);
    }

    #[test]
    fn failed_hot_reload_keeps_last_good_snapshot_and_recovers_on_next_revision() {
        use std::time::Duration;

        let source = MemorySource::default();
        source.insert("sfx/door.wav", b"first valid payload".to_vec());
        let mut server = AssetServer::default();
        server.add_source(0, source.clone());
        server.register_loader(AudioAssetLoader);
        server.set_poll_interval(Duration::ZERO);

        let mut audio = AudioAssetHandle::load(&mut server, "sfx/door.wav").unwrap();
        let first = audio.snapshot().unwrap();

        // Empty source data is rejected by AudioAssetLoader during the reload transaction.
        source.insert("sfx/door.wav", Vec::new());
        let events = server.poll_hot_reload();
        assert!(matches!(
            events.as_slice(),
            [ribbon_assets::AssetEvent::ReloadFailed { .. }]
        ));
        assert!(matches!(
            audio.refresh(&server),
            AudioAssetPoll::ReloadFailed(_)
        ));
        let retained = audio.snapshot().unwrap();
        assert_eq!(retained.revision, first.revision);
        assert_eq!(retained.bytes.as_ref(), b"first valid payload");

        source.insert("sfx/door.wav", b"recovered payload".to_vec());
        let events = server.poll_hot_reload();
        assert!(matches!(
            events.as_slice(),
            [ribbon_assets::AssetEvent::Modified { .. }]
        ));
        assert!(matches!(
            audio.refresh(&server),
            AudioAssetPoll::Ready { changed: true, .. }
        ));
        let recovered = audio.snapshot().unwrap();
        assert!(recovered.revision > first.revision);
        assert_eq!(recovered.bytes.as_ref(), b"recovered payload");
        assert!(audio.last_reload_error().is_none());
    }

    #[test]
    fn audio_asset_snapshot_shares_encoded_storage() {
        let bytes: Arc<[u8]> = Arc::from(&b"shared clip"[..]);
        let asset = EncodedAudioAsset {
            path: "shared.wav".into(),
            bytes: Arc::clone(&bytes),
        };
        let shared = Arc::clone(&asset.bytes);
        assert!(Arc::ptr_eq(&bytes, &shared));
    }
}
