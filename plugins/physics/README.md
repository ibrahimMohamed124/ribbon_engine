# plugin_physics

Purpose: platform-independent body collision and movement helpers for voxel and native 3D games.

Reusable APIs include voxel-body collision and axis-separated sliding, plus `Aabb3` and
`slide_capsule_xz` for a vertical first-person capsule moving through static world-space AABBs.
The capsule helper resolves X and Z separately so the player can slide along walls. It currently targets static, axis-aligned boxes and a vertical, fixed-height player; it does not yet implement gravity, steps, slopes, dynamic rigid-body contacts or ray/shape queries. Revisit a Rapier-backed query/controller when a game needs those broader features.

Allowed dependencies: `engine_math`, `plugin_voxel`.
Does not: depend on the window/input layer or define a game's input bindings, gravity, jumping,
interactions, or gameplay movement rules.
