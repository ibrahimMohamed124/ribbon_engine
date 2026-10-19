# engine_events
Provides typed, double-buffered events with independent read cursors.
Events can be registered and accessed as resources on `engine_ecs::World`.
Allowed workspace dependency: `engine_ecs`; see `tools/allowed_deps.toml`.
It does not provide asynchronous delivery or cross-thread event queues.
