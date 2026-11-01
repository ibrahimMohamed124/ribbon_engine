# Authored world partition assets

Ribbon's streaming plugin can load versioned RON world descriptions through the normal
`AssetServer` worker queue. Files use the `.worldpart` extension and are registered by
`StreamingPlugin` as `WorldPartitionAsset` values. Loading a description does not spawn ECS
entities, create GPU resources, or register physics objects; those remain coordinator and plugin
responsibilities.

## Minimal example

See [`examples/streaming/starter.worldpart`](../../examples/streaming/starter.worldpart).

The example includes minimal, valid `.scenedoc` files under `examples/streaming/scenes/`; copy the
partition and scene files into the paths exposed by the application's `AssetServer` source before
loading the example. The empty prefabs intentionally contain no game-specific entities.

The top-level fields are:

- `format_version`: currently `1`.
- `id`, `name`: stable authored partition identity and display name.
- `regions`: independent region definitions.

Each region provides a stable `id`, a display `name`, world-space `bounds_min`/`bounds_max`,
optional `scene_doc_path`, optional path-backed `models`, region `dependencies`, `neighbors`, and
an optional `policy`. Model transforms use three-element translation and scale arrays plus a
`rotation_xyzw` quaternion array. Missing translation, rotation, and scale default to zero,
identity, and one respectively.

Distances are measured in world units from each observer to the region AABB. The policy must
satisfy `unload_distance >= preload_distance >= activation_distance >= 0`. Neighbor and dependency
references must resolve either within this partition or to a region already registered with the
coordinator. Dependencies are topologically sorted before registration; unknown references and
cycles reject the partition before the coordinator is mutated. Scene documents load through the
AssetServer worker queue. Their required component dependencies are discovered through
`PrefabCodecRegistry`; required model assets are GPU-warmed before prefab instantiation. The physics
plugin's prefab codecs serialize `Collider`, `CollisionLayers`, and `RigidBody` configuration, and
region activation waits until all prefab-owned colliders have been reconciled by Rapier.

## Runtime flow

```rust,ignore
let lease = app.world.resource_mut::<AssetServer>()
    .request_ref_with_priority::<WorldPartitionAsset>("worlds/starter.worldpart", 100)?;
// After AssetServer reports the decoded asset as CPU-ready:
let partition = app.world.resource::<AssetServer>().get(lease.handle()).unwrap();
let registered_regions = app.world
    .resource_mut::<StreamingCoordinator>()
    .register_partition(partition)?;
```

Keep the `AssetRef<WorldPartitionAsset>` while the loaded description is needed. Region assets and
runtime entities are independently retained by their own streaming region leases. The code sample
is illustrative; callers should drive the request and readiness checks from their existing app
schedule and must not synchronously wait for the worker.

## Custom typed dependencies and generic GPU resources

A component codec may declare a typed CPU dependency. The plugin that owns the asset type registers
an adapter once after `StreamingPlugin` is installed:

```rust,ignore
app.world
    .resource_mut::<StreamingAssetAdapters>()
    .register::<MyDecodedAsset>(|asset| asset.estimated_bytes());
```

The adapter keeps a strong CPU lease through region residency. Required dependencies without a
registered adapter fail the region with an actionable error; they are never treated as ready.

For renderer resources not represented by a glTF `ModelAsset`, use `GpuResourceWarmup` with a stable
`GpuResourceKey` and increment its `generation` whenever source content changes. The current generic
path supports indexed triangle meshes and PBR materials with embedded RGBA8 texture maps. Once both
leases report `GpuResourceWarmupStatus::Ready`, `GpuMeshInstance3D::from_ready_leases` creates an ECS
render instance. The renderer owns all actual uploads and releases, and the shared upload operation
and estimated-byte limits continue to apply. Standalone shader/pipeline and animation GPU payloads
are not currently exposed by the renderer API and are not claimed as supported by this path.
