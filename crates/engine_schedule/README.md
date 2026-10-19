# engine_schedule
Provides deterministic stages, ordered systems, run conditions, and optional timing statistics.
Systems receive `&mut World` directly; parameter injection and parallel execution are not supported.
Allowed workspace dependency: `engine_ecs`; see `tools/allowed_deps.toml`.
It does not drive application lifecycle or fixed-time accumulation.
