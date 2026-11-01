# ADR 0011: Restore the recoverable Athar village scene

## Context

The saved Oct 28 source archive predates the last saved playable Wet Creek iteration. A separate, later village overlay contains the room-to-village gameplay scripts, native-window HUD pass, village models, texture maps, and matching manifest. The current project has moved the game from `examples/horror_slice` to `games/athar` and renamed its crate directories, so the overlay cannot be applied verbatim.

## Decision

Restore the saved scene and assets into `games/athar`, rewrite package and tool paths to the current workspace, and keep the room quest, village layout, and rain simulation inside the game. Restore the lightweight text overlay as a generic `ribbon_render` utility. Add only the direct `ribbon_ecs` dependency that the game scripts use for entity handles, and update the dependency allowlist and lockfile metadata with it.

## Rejected alternatives

- Ship the single-room snapshot and silently drop the saved village gameplay.
- Put the quest, village layout, or rain particles into engine crates or plugins.
- Recreate missing flashlight/audio generation scripts from memory without their source files.

## Consequences

Athar restores the key pickup, door unlock/animation, one-way transition, village meshes, collision bounds, rain streaks, and HUD. `games/athar/tools` can regenerate the game assets. The generic text pass remains renderer infrastructure, while all game-specific scene behavior stays in `games/athar`. Native Windows rendering, movement, and gameplay still require verification on the target machine.
