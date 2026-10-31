# Ribbon Scene (`ribbon_scene`)

Provides 2D-oriented and 3D transform components, shared entity hierarchy,
reflected prefab capture/instantiation, and RON scene documents. `Transform3D`
uses right-handed, Y-up `glam` translation/quaternion/per-axis scale data;
`GlobalTransform3D` is recomputed from the same `Parent`/`Children` hierarchy
by `ScenePlugin` during `PostUpdate`. Propagation snapshots local transforms and
parent links, recomputes only changed branches and descendants, reuses cached
ancestor globals, and avoids writing unchanged globals. Since transform fields
remain publicly mutable, detecting changes still requires scanning component
values each frame; see [the transform invalidation design](../../docs/architecture/TRANSFORM_INVALIDATION.md). Ancestors participate in
the 3D chain only when they also carry `Transform3D`; the legacy `Transform` and
3D transform conventions are not implicitly mixed. Transform composition uses
decomposed TRS and therefore does not preserve shear caused by a rotated child
under a non-uniformly scaled parent.

Allowed workspace dependencies: `ribbon_ecs`, `ribbon_reflect`, `ribbon_math`,
`ribbon_app`, and `ribbon_plugins`.
Allowed external dependencies: `serde`, `ron`, and `glam`.
It does not own render data, asset loading, or game-specific components.

## 3D transform usage

Add `ScenePlugin` to apps that use `ribbon_plugin_render3d`. A mesh entity carries a
local `Transform3D`; `ScenePlugin` computes its `GlobalTransform3D` in
`PostUpdate`, and the mesh renderer uses the global matrix. Use the shared
`World::set_parent(child, parent)` extension to create transform inheritance:

```rust
use ribbon_scene::{HierarchyExt, Transform3D};
use glam::Vec3;

let root = world.spawn((Transform3D::from_translation(Vec3::new(2.0, 0.0, 0.0)),));
let child = world.spawn((Transform3D::from_translation(Vec3::X),));
world.set_parent(child, root)?;
```

The child is rendered at the parent's translation plus the parent's rotated and
scaled local offset. The legacy `Transform` remains available for systems using
its existing yaw/pitch and 2D coordinate convention.
