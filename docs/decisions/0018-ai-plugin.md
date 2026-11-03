# 0018 — Gameplay AI building blocks

## Decision

`ribbon_plugin_ai` sits above the scene and physics plugins and below games. It provides generic
state machines, callback-driven behavior trees, blackboards, 2.5D grid A*, physics-assisted grid
baking, deterministic path smoothing, steering output, and line-of-sight perception. Games own the
actual movement policy and agent-specific leaf actions.

Grid navigation uses `pathfinding` A* with an expansion limit. Physics rays exclude sensors when
baking and sight checks. Recast/navmesh was intentionally excluded from this phase.

## Limits

There is no navmesh, multi-floor routing, dynamic obstacle avoidance, crowd steering, hearing,
perception memory, utility AI, or GOAP. Agents consume `NavSteering` themselves; the plugin does not
move entities.
