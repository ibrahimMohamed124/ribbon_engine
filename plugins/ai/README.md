# ribbon_plugin_ai

Reusable FSMs, behavior trees, grid A* navigation, and raycast-based sight perception. Navigation is
single-layer 2.5D; steering is output data and does not move entities itself.

```rust
let mut grid = NavGrid::new(Vec2::ZERO, 1.0, 64, 64);
grid.set_walkable(5, 5, false);
world.insert_resource(NavWorld { grid });
world.spawn((NavAgent::default(), NavDestination(goal), Transform3D::default()));
// In the game movement system, convert desired velocity to the next fixed-step displacement.
let velocity = world.get::<NavSteering>(agent_entity).map_or(Vec3::ZERO, |s| s.desired_velocity);
if let Some(mut intent) = world.get_mut::<CharacterMovementIntent>(agent_entity) {
    intent.desired_displacement = velocity * fixed_dt;
}
```

Navigation is a single 2.5D layer. There is no navmesh/Recast, multi-floor routing, dynamic
avoidance/crowd steering, or automatic entity movement. Perception only provides budgeted sight;
there is no hearing, memory, utility AI, or GOAP.
