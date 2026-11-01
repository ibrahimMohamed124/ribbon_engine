# Prefab asset references

## Serialization contract

Asset-backed components must serialize stable asset paths, never runtime `Handle<T>` IDs or generations. A component-specific `PrefabCodecRegistry` is registered by a crate that already depends on both the component and its asset service. The codec receives the entity and `PrefabLoadContext`, so the scene crate does not depend on `ribbon_assets`, a renderer, or any game. `AssetServer::path(handle)` resolves a live generation-checked handle back to its normalized source path.

## Restoration

The caller builds a `PrefabLoadContext` and inserts the required service (normally an `AssetServer`). `instantiate_with_context` validates entity IDs and parent references, then delegates asset-backed components to their registered decoder. A decoder loads the serialized path through the supplied server and returns a reflected component; missing paths must return `SceneError::MissingAsset`, not panic. If any component or hierarchy restoration fails, all entities spawned for that prefab are despawned.

## Component registration

The scene registry remains responsible for component identity and insertion. The codec registry is separate: renderer/plugin crates register codecs for components that contain handles and provide ordinary reflection for their other components. This avoids a `ribbon_scene -> ribbon_assets` dependency and lets the same scene format be used by headless tools. Existing `capture` and `instantiate` APIs remain available for components that do not need custom asset restoration; callers opt into the context-aware APIs when using codecs.

## Missing assets and compatibility

A missing asset is a recoverable load error containing the requested path. Unknown component names retain the existing strict/lenient behavior. The prefab format version stays at `1`; paths are portable identifiers, while the asset server owns path normalization and loader selection.

## Current implementation boundary

The context/codec extension point, reverse handle-to-path lookup, `ModelInstance` path codec, and reflected mesh/light components are implemented in `plugins/render3d/src/prefab.rs`. The targeted tests cover a RON round-trip for `Transform3D` + `ModelInstance` + `SpotLight`, and missing-model failure with rollback of partially spawned entities. These tests are source-level acceptance coverage; they must still be run in the target Windows checkout before the feature is declared verified.
