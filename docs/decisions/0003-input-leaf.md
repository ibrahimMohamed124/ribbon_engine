## Context

Reusable action bindings lived in the platform crate while STUDS maintained its own game action names.

## Decision

Move platform-neutral keyboard and mouse state, action/binding sources, rebinding data, scripted input queues, and generic buffered input into `ribbon_input`. Re-export those types from `ribbon_platform`; keep OS event conversion/window lifecycle in platform and STUDS action names/default bindings plus input-frame merge policy in the game.

## Rejected alternatives

Keeping generic binding logic in platform couples headless/game input to window lifecycle.

## Consequences

The new crate has no external or workspace dependencies. `ribbon_platform` converts OS events and feeds them into `ribbon_input` before draining scripted events each `First` stage; focus loss releases all held keys and mouse buttons. STUDS stores pending `InputFrame`s through the generic buffer while preserving its domain-specific merge semantics.
