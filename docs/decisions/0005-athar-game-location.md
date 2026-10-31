## Context

The first-person horror game and its content tools were under examples.

## Decision

Move the game to `games/athar` and game-specific tools/docs beside it.

## Rejected alternatives

Leaving game-specific tooling in global tools obscures ownership.

## Consequences

Workspace, package name, and references must use `athar`; gameplay logic is otherwise preserved.
