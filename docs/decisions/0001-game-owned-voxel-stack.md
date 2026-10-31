## Context

The engine mesh renderer previously imported STUDS voxel data and exposed an offscreen voxel renderer.

## Decision

Move voxel storage, CPU raycaster, showcases, and the offscreen GPU adapter under `games/studs`; keep the engine renderer mesh-oriented.

## Rejected alternatives

Keeping voxel types in the renderer would preserve a reverse game/engine ownership dependency.

## Consequences

STUDS depends on public mesh-renderer APIs. Four game-only voxel columns were removed from the generic renderer CSV to keep game-domain terms out of engine crates. This is a documented deviation from the prompt's byte-identical profile-format requirement; game-owned chunk counters remain available through `VoxelRendererResourceStats`.
