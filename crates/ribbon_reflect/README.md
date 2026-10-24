# Ribbon Reflection (`ribbon_reflect`)
Purpose: Runtime reflection and type registration.
Allowed dependencies: see `tools/allowed_deps.toml`.
Provides serializable reflected values, field metadata, and a component type registry.
Allowed workspace dependency: `ribbon_ecs`; allowed external dependency: `serde`.
It does not own entities, perform persistence I/O, or schedule systems.
