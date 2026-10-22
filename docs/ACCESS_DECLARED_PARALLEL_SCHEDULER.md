# Access-Declared Parallel Scheduler

## Status

Implemented in source; target-side compilation/runtime verification and performance comparison remain pending. No tests were added or run for this change. Existing scheduler test source was retained unchanged.

## Public API

Legacy callbacks remain supported:

```rust
SystemConfig::new(|world: &mut World| {
    // unrestricted ECS and resource-registry access
})
```

A legacy callback receives unrestricted exclusive world access, so the scheduler makes it a serial barrier. New concurrent callbacks opt into the narrower shared-world interface and declare each type they touch:

```rust
SystemConfig::parallel(|world| {
    for (_, (mut position, velocity)) in
        world.query::<(&mut Position, &Velocity)>().iter()
    {
        position.0 += velocity.0;
    }
})
.write_component::<Position>()
.read_component::<Velocity>()
.name("integrate")
```

If a single system reads and writes the same component/resource type, declare that type once as a write. Repeated declarations are rejected during `build()`. Parallel callbacks must declare at least one access. Declaring access metadata on a legacy exclusive system is rejected because that callback can access the whole `&mut World` and its real footprint cannot be inferred.

### Resources

Parallel callbacks receive `&World`, so mutable resource state must use interior mutability. `engine_ecs::SyncResource<T>` wraps a standard-library `RwLock<T>`:

```rust
world.insert_resource(SyncResource::new(0_u64));

SystemConfig::parallel(|world| {
    *world.resource::<SyncResource<u64>>().write().unwrap() += 1;
})
.write_resource::<SyncResource<u64>>()
.name("count_frames");
```

The lock protects the resource's memory. Declaring the wrapper as a resource write additionally causes the scheduler to serialize systems with conflicting reads/writes. A caller can use other synchronized types (such as `Mutex<T>` or `RwLock<T>`), but must declare the exact resource type.

Structural operations—spawn/despawn, component insertion/removal, and resource registry mutation—remain exclusive operations. They belong in legacy systems or serialized command-application phases. Read-only component queries and resources can be declared as reads. `World::query::<&mut T>()` uses hecs dynamic borrow checks, which protect component borrows if concurrent callbacks reach the same component storage. Incorrect access declarations can still cause borrow panics or incorrect ordering of user-managed interior-mutability state; the scheduler cannot infer a closure's access footprint.

## Build algorithm and ordering

1. Validate unique system names, unique labels within each stage, all declared `before`/`after` labels, and absence of dependency cycles.
2. Validate per-system access declarations, rejecting missing and repeated component/resource types.
3. Compute the stable topological order induced by authored ordering constraints.
4. For access conflicts, insert deterministic ordering edges according to that stable declared order. Reads may overlap reads; any write conflicts with another read or write to the same typed component/resource. Legacy systems and conditional systems act as exclusive barriers because their access footprint cannot be inferred safely.
5. Build stable topological frontiers from the combined dependency/conflict graph. Each frontier is exposed as a batch through `Schedule::execution_plan(stage)`.
6. When parallelism is enabled, compatible `SystemConfig::parallel` callbacks in a batch are dispatched across at most `Schedule::max_parallelism()` scoped workers. Batches are joined before the next batch starts. Worker completion order is not deterministic, but systems in a batch must be independent by contract; the batch plan and conflict ordering are deterministic.

`Schedule::set_parallel_enabled(false)` runs the built topological order serially. `Schedule::set_max_parallelism(1)` is the single-worker path. Before `build()`, the schedule preserves the prior safe insertion-order behavior and does not try to parallelize unvalidated callbacks.

## Deliberate boundaries

- The API avoids adding a third-party scheduler dependency; worker dispatch uses `std::thread::scope` and a bounded worker count per eligible batch.
- A persistent worker pool is not included in this first layer. Thread startup overhead must be profiled before claiming end-to-end speedups or adding pool lifecycle complexity.
- Run-condition closures have no access metadata of their own; conditional systems are serial barriers and their conditions are evaluated immediately before the system.
- Access metadata is a typed runtime planning contract, not compile-time filtered system parameters. A future `SystemParam`/access-capability system could strengthen enforcement but would be a separate API migration.
- No renderer/GPU work is moved onto scheduler workers. GPU context ownership remains with the rendering plugin's intended thread.

## Verification and acceptance

No Rust build/test/lint command or runtime was executed while making this change. No new tests were added, and the existing scheduler test module was copied over unchanged. If replacing an earlier archive in an existing checkout and rustdoc reports that `engine_ecs::SyncResource` is missing, remove stale build artifacts once before retesting; the current source exports `SyncResource` from `engine_ecs` and `engine_app` re-exports it intentionally:

```powershell
cargo clean
```

On the target Windows checkout, then run:

```powershell
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace -j 1
cargo test --workspace --no-default-features -j 1
python tools/check_deps.py
```

Then compare representative systems under `Schedule::set_parallel_enabled(false)` and the default parallel mode. Compare final component/resource values—not only timings—and profile an independent read-only batch, a disjoint-component writer batch, resource conflicts, dependency barriers, conditional systems, and one-worker fallback. Existing STUDS golden assertions must remain unchanged. The stage is not accepted for performance based solely on successful compilation.
