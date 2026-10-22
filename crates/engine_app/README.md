# engine_app
Provides the application lifecycle, deterministic headless runner, time resources, and stage driving.
It owns the ECS world and schedule, supports access-declared parallel systems with a serial fallback,
and has no window or renderer implementation.
Allowed workspace dependencies: `engine_ecs`, `engine_events`, `engine_schedule`, and `engine_math`.
It does not register platform or rendering plugins itself.
