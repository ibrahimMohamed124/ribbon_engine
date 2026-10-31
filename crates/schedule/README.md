# Ribbon Schedule (`ribbon_schedule`)

Provides deterministic stages, dependency ordering, run conditions, execution-plan inspection,
optional per-system timing, and access-declared parallel execution.

## System kinds

- `SystemConfig::new` and `Schedule::add_system` retain unrestricted `&mut World` access. These
  legacy systems are exclusive barriers and execute serially.
- `SystemConfig::parallel` accepts `Fn(&World) + Send + Sync` and requires component/resource
  access declarations. Compatible parallel systems can run at the same time. Component mutation
  is possible with `world.query::<&mut T>()`, which uses hecs dynamic borrow checking.
- For writable resources, store a lock-backed value such as `SyncResource<T>` in `World`. Declare
  the wrapper type with `read_resource` or `write_resource`; `SyncResource::read` and `write`
  expose its protected value.
- Structural operations (spawn/despawn, component insertion/removal, resource registry changes)
  remain in exclusive systems or command-application phases.

Read declarations can overlap with reads. Any write conflicts with another read/write of the same
component or resource type. Access declarations must exactly describe what each parallel system
uses. The schedule rejects missing declarations, repeated declarations, unknown/duplicate labels,
duplicate system names, and dependency cycles. A read+write use of one type is declared once as a
write. Conditional systems form a serial barrier because closure access cannot be inferred safely.

`Schedule::execution_plan(stage)` returns the built conflict-free batches. Set
`Schedule::set_parallel_enabled(false)` or `set_max_parallelism(1)` to force the serial fallback.
The implementation uses a bounded number of scoped worker threads per parallel batch; a persistent
pool is intentionally deferred until target-side profiling justifies its lifecycle/complexity.

Allowed workspace dependency: `ribbon_ecs`; no third-party scheduler/runtime dependency was added.
