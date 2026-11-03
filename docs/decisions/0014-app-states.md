# ADR 0014: Application state transitions

Application states are typed resources owned by `ribbon_app`. Games request changes through `NextState`; the app applies them at `PreUpdate`, invokes exit hooks, updates the current state, emits one transition event, then invokes enter hooks. Initial enter hooks run on the first `PreUpdate`. Requests made by hooks are deferred until the next frame.

This keeps state policy game-owned while providing deterministic transition timing and run conditions. State transitions are not applied immediately, and re-entering the current state is ignored.
