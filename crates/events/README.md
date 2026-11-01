# Ribbon Events (`ribbon_events`)
Provides typed, double-buffered events with independent read cursors.
Events can be registered and accessed as resources on `ribbon_ecs::World`.
Allowed workspace dependency: `ribbon_ecs`; see `tools/allowed_deps.toml`.
It does not provide asynchronous delivery or cross-thread event queues.
