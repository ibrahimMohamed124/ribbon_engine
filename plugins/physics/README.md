# `ribbon_plugin_physics`

Ribbon's general-purpose 3D physics integration uses Rapier 3D for rigid-body simulation,
collision detection, contact solving, CCD and ray queries. It is CPU-only and can run in a
headless `App`; it has no dependency on a renderer or GPU. One Ribbon world unit represents one
meter. The app owns the fixed-step accumulator and physics executes exactly once for each
`Stage::FixedUpdate` invocation.

## Compile and run the headless example

The real, workspace example is `examples/headless_physics.rs`:

```powershell
cargo run -p ribbon_plugin_physics --example headless_physics
```

It registers `ScenePlugin` before `PhysicsPlugin`, creates a fixed floor and a dynamic sphere,
drives the app with the same engine `Time` resource used by a headless runner, and exercises a
raycast. This example is built with the package's `--all-targets` checks.

## Install and create bodies

Register the scene plugin first because physics consumes `Transform3D` and `GlobalTransform3D`:

```rust,no_run
use glam::Vec3;
use ribbon_app::App;
use ribbon_plugin_physics::{Collider, PhysicsPlugin, RigidBody};
use ribbon_plugins::AppPluginExt;
use ribbon_scene::{ScenePlugin, Transform3D};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = App::new();
    app.add_plugin(ScenePlugin)?;
    app.add_plugin(PhysicsPlugin)?;
    app.world.spawn((
        Transform3D::from_translation(Vec3::new(0.0, -0.5, 0.0)),
        Collider::cuboid(Vec3::new(10.0, 0.5, 10.0)),
    ));
    app.world.spawn((
        Transform3D::from_translation(Vec3::new(0.0, 3.0, 0.0)),
        RigidBody::dynamic(),
        Collider::ball(0.5),
    ));
    Ok(())
}
```

`PhysicsConfig` is a world resource. Its default gravity is `(0, -9.81, 0)` and its default solver
budget is 4 iterations per fixed step (valid range 1–64). Invalid gravity or solver settings are
reported in `PhysicsDiagnostics`, and the step is skipped rather than passing invalid values into
Rapier. Use `RigidBody::fixed()`, `RigidBody::dynamic()`,
`RigidBody::kinematic_position_based()`, or `RigidBody::kinematic_velocity_based()` to choose the
body kind. Linear and angular velocities are in units/second and radians/second. Set
`RigidBody::ccd_enabled = Some(true)` to force CCD on or `Some(false)` to force it off; `None`
uses `PhysicsConfig::default_ccd_enabled`. Changing a `RigidBody` descriptor re-creates its backend body;
expect its solver state (including accumulated momentum) to reset when reconfiguring the descriptor.

## Colliders and collision layers

Supported primitives are `Collider::ball(radius)`, `Collider::cuboid(half_extents)`,
`Collider::capsule_y(half_height, radius)`, and `Collider::cylinder_y(half_height, radius)`.
Cuboid dimensions are half-extents. For capsules, `half_height` is the half-length of the
cylindrical segment between the caps. The first version supports one collider per entity. To build
a compound object, use multiple collider entities instead of assuming compound support.

Collider material defaults are friction `0.7`, restitution `0.0`, and density `1.0`. Restitution
must be in `[0, 1]`; friction and density must be finite and non-negative/positive respectively.
Sensor colliders report overlap events without contact response. Add `CollisionLayers` to choose
membership/filter bits; by default all layers interact. Balls require uniform scale, and vertical
capsules/cylinders require equal X/Z scale. Scales must be finite and strictly positive. Unsupported
geometry or transforms produce a `PhysicsDiagnostics` entry and are not silently approximated.

## Events and raycasts

The plugin emits `CollisionStarted`, `CollisionStopped`, `SensorEntered`, and `SensorExited` into
Ribbon's typed event storage. `CollisionStarted`/`CollisionStopped` include sensor pairs too; the
sensor-specific events are emitted in addition. Events are generated during `FixedUpdate`; multiple
physics steps in one rendered frame can emit multiple events and Ribbon's event buffers preserve
them for their normal two-buffer lifetime.

```rust,no_run
use glam::Vec3;
use ribbon_plugin_physics::{raycast, RaycastQuery};
use ribbon_ecs::World;

fn query(world: &World) {
    let query = RaycastQuery::new(Vec3::new(0.0, 2.0, 0.0), -Vec3::Y, 100.0);
    if let Some(hit) = raycast(world, query) {
        println!("hit {:?} at {:?}, normal {:?}, distance {}",
            hit.entity, hit.position, hit.normal, hit.distance);
    }
}
```

`RaycastQuery` supports entity exclusion, collision layers, maximum distance, and optional sensor
inclusion. Queries return Ribbon `Entity` values and positions/normals/distances, not Rapier handles.
The broad phase is refreshed during a fixed physics step, so query results after adding or moving a
collider become current after the next `FixedUpdate`.

## Character movement and debugging

For a basic kinematic character, use a vertical capsule with `RigidBody::kinematic_position_based()`,
add `CharacterController`, and submit `CharacterMovementIntent { desired_displacement }` before a
fixed step. The plugin consumes that intent once, asks Rapier's kinematic character controller to
resolve the move, and stores a `CharacterMovementResult` with applied displacement, ground contact,
obstruction, and contact count. Submit a new intent each step; jump velocity/air-control policy stays
in gameplay code.

`PhysicsDebugData` exposes a renderer-independent snapshot of collider shapes, world poses, and
sensor state. An editor or renderer can draw this data without adding a rendering dependency to the
physics plugin. `PhysicsDiagnostics` contains current-frame validation and synchronization issues.

## Timing, hierarchy and limitations

The plugin depends on `ScenePlugin`. It reconciles entity/component changes during `PreUpdate`,
steps Rapier once per engine `FixedUpdate` using `Time::delta`, then writes dynamic and
velocity-kinematic poses back to `Transform3D` before normal rendering. The app's own fixed-time
accumulator and step cap remain authoritative; physics does not advance a second clock. Parent
transforms are converted using Ribbon's `GlobalTransform3D` composition. Parent chains for physics
entities require uniformly scaled `Transform3D` ancestors; non-uniform ancestor scale is diagnosed
and the entity is excluded because a TRS hierarchy cannot represent the resulting shear reliably.
Dynamic bodies treat a user-edited transform or changed parent pose as a teleport. Entities must have
`Transform3D` and a valid world transform.

The first version deliberately does not claim triangle-mesh colliders, compound colliders, joints,
soft-body authoring, serialization of runtime physics state, or visible collider rendering. Character
control currently requires a vertical capsule attached to a position-based kinematic body. Scene
hierarchy's TRS representation cannot encode arbitrary affine shear; use transforms that are
representable by `GlobalTransform3D`. Existing `Aabb3` and `slide_capsule_xz` helpers remain unchanged
for ATHAR and older callers; they are separate compatibility helpers and are not the Rapier runtime.
