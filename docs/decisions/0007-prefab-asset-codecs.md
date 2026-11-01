# ADR 0007: Restore prefab assets through caller-provided codecs

## Decision

`ribbon_scene` provides a type-keyed `PrefabLoadContext` and `PrefabCodecRegistry` without depending on `ribbon_assets` or renderer crates. Asset-backed components serialize normalized paths through a codec and resolve those paths through an `AssetServer` supplied by the caller. `AssetServer::path(handle)` provides reverse lookup for live generation-checked handles.

## Consequences

The render3d plugin registers reflection for mesh/model-instance and light components and a path-based codec for `ModelInstance`. Missing model loads return `SceneError::MissingAsset`; prefab instantiation rolls back entities spawned before a failed component restore. Existing `capture` and `instantiate` remain available for components that do not require a custom codec.

## Validation boundary

The source contains a round-trip test for `Transform3D`, `ModelInstance`, and `SpotLight`, plus a missing-asset test asserting error reporting and rollback. Run the Windows workspace matrix before considering these tests verified.
