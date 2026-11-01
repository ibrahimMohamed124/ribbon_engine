## Context

The workspace had both a legacy CPU raycaster and a modern mesh renderer.

## Decision

Keep `ribbon_render` renderer-neutral and `ribbon_plugin_render3d` responsible for native mesh/PBR rendering.

## Rejected alternatives

Putting game-specific mesh generation into the engine would reverse dependency direction.

## Consequences

Game-specific adapters live in the game tree and depend only on public renderer APIs.
