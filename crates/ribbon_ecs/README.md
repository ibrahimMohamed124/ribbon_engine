# Ribbon ECS (`ribbon_ecs`)
Provides the engine-owned entity/component world, typed resources, and deferred commands.
It wraps `hecs` so other engine crates depend only on this API.
Allowed dependency: `hecs` only; see `tools/allowed_deps.toml`.
It does not provide scheduling, reflection, or parallel system execution.

## Thread-safe resources for parallel systems

`SyncResource<T>` wraps a value in a standard-library `RwLock` and is intended for resources that
are read or written by `SystemConfig::parallel` callbacks. Store it as a normal world resource, then
use `world.resource::<SyncResource<T>>().read()` or `.write()` inside the callback. Declare that
wrapper type using `read_resource::<SyncResource<T>>()` or `write_resource::<SyncResource<T>>()` in
the system config so the scheduler inserts the required conflict barriers. Resource locking protects
memory safety; correct scheduler declarations are still required for intended ordering and to avoid
lock-order problems.
