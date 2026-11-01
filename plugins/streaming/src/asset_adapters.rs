//! Type-erased adapters that let SceneDoc component codecs declare typed asset dependencies
//! without making the scene crate depend on AssetServer or individual game asset types.

use ribbon_assets::{Asset, AssetRef, AssetServer, LoadError, LoadState};
use std::any::{type_name, Any};
use std::collections::HashMap;
use std::marker::PhantomData;

/// Readiness reported for one strong, typed SceneDoc dependency lease.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StreamingDependencyState {
    Loading,
    Ready {
        estimated_bytes: u64,
    },
    /// The path now names a newer generation than the currently retained lease.
    Stale,
    Failed(String),
}

type ErasedLease = Box<dyn Any + Send + Sync>;

trait StreamingAssetAdapter: Send + Sync {
    fn request(
        &self,
        server: &mut AssetServer,
        path: &str,
        priority: i32,
    ) -> Result<ErasedLease, LoadError>;
    fn readiness(
        &self,
        server: &AssetServer,
        path: &str,
        lease: &(dyn Any + Send + Sync),
    ) -> StreamingDependencyState;
    fn release(&self, server: &mut AssetServer, lease: ErasedLease);
}

struct TypedAssetAdapter<T: Asset> {
    estimate_size: Box<dyn Fn(&T) -> u64 + Send + Sync>,
    _asset: PhantomData<fn() -> T>,
}

impl<T: Asset> StreamingAssetAdapter for TypedAssetAdapter<T> {
    fn request(
        &self,
        server: &mut AssetServer,
        path: &str,
        priority: i32,
    ) -> Result<ErasedLease, LoadError> {
        server
            .retry_ref_with_priority::<T>(path, priority)
            .map(|lease| Box::new(lease) as ErasedLease)
    }

    fn readiness(
        &self,
        server: &AssetServer,
        path: &str,
        erased_lease: &(dyn Any + Send + Sync),
    ) -> StreamingDependencyState {
        let Some(lease) = erased_lease.downcast_ref::<AssetRef<T>>() else {
            return StreamingDependencyState::Failed(format!(
                "streaming dependency lease type mismatch for `{}`",
                type_name::<T>()
            ));
        };
        if server.current_handle::<T>(path) != Some(lease.handle()) {
            return StreamingDependencyState::Stale;
        }
        if let Some(asset) = server.get(lease.handle()) {
            return StreamingDependencyState::Ready {
                estimated_bytes: (self.estimate_size)(asset),
            };
        }
        match server.state::<T>(path) {
            Some(LoadState::Loading) | Some(LoadState::Unloaded) => {
                StreamingDependencyState::Loading
            }
            Some(LoadState::Failed(error)) => StreamingDependencyState::Failed(error.to_string()),
            Some(LoadState::Loaded) => StreamingDependencyState::Failed(format!(
                "asset `{path}` is marked Loaded but has no resident CPU value"
            )),
            None => StreamingDependencyState::Failed(format!(
                "asset `{path}` disappeared from AssetServer"
            )),
        }
    }

    fn release(&self, server: &mut AssetServer, erased_lease: ErasedLease) {
        if let Ok(lease) = erased_lease.downcast::<AssetRef<T>>() {
            let handle = lease.handle();
            drop(lease);
            let _ = server.cancel_if_unleased(handle);
        }
    }
}

/// Registry of typed CPU asset adapters for resources discovered in serialized prefab components.
///
/// Register adapters from the plugin that owns each asset type. The key is `type_name::<T>()`,
/// which must match the stable type string emitted by its `PrefabCodecRegistry` dependency
/// declaration. Each adapter owns a strong `AssetRef<T>` lease until the containing region unloads.
#[derive(Default)]
pub struct StreamingAssetAdapters {
    adapters: HashMap<String, Box<dyn StreamingAssetAdapter>>,
}

impl StreamingAssetAdapters {
    /// Register an asset type and its memory-size estimator. The estimator must return a CPU
    /// decoded-size estimate (zero is valid when the type has no useful accounting method).
    pub fn register<T: Asset>(
        &mut self,
        estimate_size: impl Fn(&T) -> u64 + Send + Sync + 'static,
    ) {
        self.adapters.insert(
            type_name::<T>().to_owned(),
            Box::new(TypedAssetAdapter::<T> {
                estimate_size: Box::new(estimate_size),
                _asset: PhantomData,
            }),
        );
    }

    /// Whether a declared stable type name has a request/readiness/release adapter.
    pub fn contains(&self, asset_type: &str) -> bool {
        self.adapters.contains_key(asset_type)
    }

    pub(crate) fn request(
        &self,
        server: &mut AssetServer,
        asset_type: &str,
        path: &str,
        priority: i32,
    ) -> Result<ErasedLease, LoadError> {
        self.adapters
            .get(asset_type)
            .ok_or_else(|| {
                LoadError::new(
                    ribbon_assets::LoadErrorKind::TypeMismatch,
                    path,
                    format!("no streaming asset adapter registered for `{asset_type}`"),
                )
            })?
            .request(server, path, priority)
    }

    pub(crate) fn readiness(
        &self,
        server: &AssetServer,
        asset_type: &str,
        path: &str,
        lease: &(dyn Any + Send + Sync),
    ) -> StreamingDependencyState {
        match self.adapters.get(asset_type) {
            Some(adapter) => adapter.readiness(server, path, lease),
            None => StreamingDependencyState::Failed(format!(
                "no streaming asset adapter registered for `{asset_type}`"
            )),
        }
    }

    pub(crate) fn release(&self, server: &mut AssetServer, asset_type: &str, lease: ErasedLease) {
        if let Some(adapter) = self.adapters.get(asset_type) {
            adapter.release(server, lease);
        }
        // If an adapter was removed/replaced before region release, dropping the erased lease
        // still releases its strong pin. Cancellation is best-effort in that unusual case.
    }
}

#[cfg(test)]
mod tests {
    use super::{StreamingAssetAdapters, StreamingDependencyState};
    use ribbon_assets::{
        AssetLoader, AssetRef, AssetServer, LoadContext, LoadError, LoadErrorKind, MemorySource,
    };

    #[derive(Clone, Debug)]
    struct TextAsset(String);

    struct TextLoader;

    impl AssetLoader for TextLoader {
        type Asset = TextAsset;

        fn extensions(&self) -> &[&str] {
            &["streamtext"]
        }

        fn load(&self, bytes: &[u8], context: &mut LoadContext) -> Result<Self::Asset, LoadError> {
            String::from_utf8(bytes.to_vec())
                .map(TextAsset)
                .map_err(|error| {
                    LoadError::new(LoadErrorKind::Decode, &context.path, error.to_string())
                })
        }
    }

    #[test]
    fn typed_adapter_requests_pins_checks_and_releases_the_asset_generation() {
        let source = MemorySource::default();
        source.insert("loading.streamtext", b"pending".to_vec());
        source.insert("ready.streamtext", b"hello".to_vec());
        let mut server = AssetServer::default();
        server.register_loader(TextLoader);
        server.add_source(0, source);
        // Load the ready case synchronously as deterministic fixture setup. The production
        // request path remains asynchronous; this test does not depend on worker timing.
        let ready_handle = server.load::<TextAsset>("ready.streamtext").unwrap();
        let mut adapters = StreamingAssetAdapters::default();
        adapters.register::<TextAsset>(|asset| asset.0.len() as u64);
        let asset_type = std::any::type_name::<TextAsset>();

        let loading = adapters
            .request(&mut server, asset_type, "loading.streamtext", 10)
            .unwrap();
        assert!(matches!(
            adapters.readiness(&server, asset_type, "loading.streamtext", loading.as_ref()),
            StreamingDependencyState::Loading
        ));
        adapters.release(&mut server, asset_type, loading);

        let lease = adapters
            .request(&mut server, asset_type, "ready.streamtext", 10)
            .unwrap();
        assert_eq!(
            lease
                .downcast_ref::<AssetRef<TextAsset>>()
                .unwrap()
                .handle(),
            ready_handle
        );
        assert_eq!(
            adapters.readiness(&server, asset_type, "ready.streamtext", lease.as_ref()),
            StreamingDependencyState::Ready { estimated_bytes: 5 }
        );
        adapters.release(&mut server, asset_type, lease);
        assert_eq!(server.strong_ref_count(ready_handle), 0);
    }

    #[test]
    fn missing_adapter_is_reported_instead_of_silently_ready() {
        let adapters = StreamingAssetAdapters::default();
        assert!(!adapters.contains("missing::Asset"));
    }
}
