# engine_ecs
Provides the engine-owned entity/component world, typed resources, and deferred commands.
It wraps `hecs` so other engine crates depend only on this API.
Allowed dependency: `hecs` only; see `tools/allowed_deps.toml`.
It does not provide scheduling, reflection, or parallel system execution.
