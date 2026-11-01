use std::time::Duration;

use glam::Vec3;
use ribbon_app::{App, Time};
use ribbon_events::EventCursor;
use ribbon_events::WorldEventsExt;
use ribbon_plugins::AppPluginExt;
use ribbon_scene::{ScenePlugin, Transform3D};

use crate::{
    raycast, Collider, CollisionStarted, PhysicsConfig, PhysicsDebugData, PhysicsDiagnostics,
    PhysicsPlugin, RaycastQuery, RigidBody,
};

fn physics_app() -> App {
    let mut app = App::new();
    app.add_plugin(ScenePlugin).expect("scene plugin registers");
    app.add_plugin(PhysicsPlugin)
        .expect("physics plugin registers");
    app
}

fn tick(app: &mut App, frames: usize) {
    for _ in 0..frames {
        app.world.resource_mut::<Time>().delta = Duration::from_secs_f64(1.0 / 60.0);
        app.update();
    }
}

#[test]
fn plugin_registers_configuration_events_diagnostics_and_debug_data() {
    let app = physics_app();
    assert!(app.world.try_resource::<PhysicsConfig>().is_some());
    assert!(app.world.try_resource::<PhysicsDiagnostics>().is_some());
    assert!(app.world.try_resource::<PhysicsDebugData>().is_some());
    assert!(app
        .world
        .try_resource::<ribbon_events::Events<CollisionStarted>>()
        .is_some());
}

#[test]
fn dynamic_body_falls_and_settles_on_a_fixed_floor() {
    let mut app = physics_app();
    app.world.spawn((
        Transform3D::from_translation(Vec3::new(0.0, -0.5, 0.0)),
        Collider::cuboid(Vec3::new(5.0, 0.5, 5.0)),
    ));
    let ball = app.world.spawn((
        Transform3D::from_translation(Vec3::new(0.0, 3.0, 0.0)),
        RigidBody::dynamic(),
        Collider::ball(0.5),
    ));

    tick(&mut app, 120);
    let transform = *app
        .world
        .get::<Transform3D>(ball)
        .expect("transform retained");
    assert!(
        transform.translation.y < 1.0,
        "body did not fall: {transform:?}"
    );
    assert!(
        transform.translation.y > 0.35,
        "body passed through the floor: {transform:?}"
    );
}

#[test]
fn physics_does_not_advance_without_an_engine_fixed_step() {
    let mut app = physics_app();
    let body = app.world.spawn((
        Transform3D::from_translation(Vec3::new(0.0, 2.0, 0.0)),
        RigidBody::dynamic(),
        Collider::ball(0.5),
    ));
    app.world.resource_mut::<Time>().delta = Duration::ZERO;
    app.update();
    assert_eq!(
        app.world.get::<Transform3D>(body).unwrap().translation.y,
        2.0
    );

    tick(&mut app, 1);
    assert!(app.world.get::<Transform3D>(body).unwrap().translation.y < 2.0);
}

#[test]
fn raycast_returns_ribbon_entity_and_world_hit_data() {
    let mut app = physics_app();
    let target = app.world.spawn((
        Transform3D::from_translation(Vec3::ZERO),
        Collider::cuboid(Vec3::splat(1.0)),
    ));
    tick(&mut app, 1);
    let hit = raycast(
        &app.world,
        RaycastQuery::new(Vec3::new(0.0, 5.0, 0.0), -Vec3::Y, 10.0),
    )
    .expect("ray should hit the box");
    assert_eq!(hit.entity, target);
    assert!((hit.position.y - 1.0).abs() < 0.05);
    assert!((hit.distance - 4.0).abs() < 0.05);
    assert!(hit.normal.y > 0.9);
}

#[test]
fn despawn_cleans_physics_debug_state_and_stale_handles() {
    let mut app = physics_app();
    let entity = app.world.spawn((
        Transform3D::from_translation(Vec3::ZERO),
        Collider::ball(0.5),
    ));
    tick(&mut app, 1);
    assert_eq!(app.world.resource::<PhysicsDebugData>().colliders.len(), 1);
    app.world.despawn(entity).unwrap();
    tick(&mut app, 1);
    assert!(app
        .world
        .resource::<PhysicsDebugData>()
        .colliders
        .is_empty());
    assert!(raycast(
        &app.world,
        RaycastQuery::new(Vec3::new(0.0, 2.0, 0.0), -Vec3::Y, 5.0)
    )
    .is_none());
}

#[test]
fn collision_start_event_is_emitted_once_for_an_overlapping_pair() {
    let mut app = physics_app();
    app.world.spawn((
        Transform3D::from_translation(Vec3::ZERO),
        Collider::ball(1.0),
    ));
    app.world.spawn((
        Transform3D::from_translation(Vec3::new(0.25, 0.0, 0.0)),
        RigidBody::dynamic(),
        Collider::ball(1.0),
    ));
    let mut cursor = EventCursor::<CollisionStarted>::default();
    tick(&mut app, 1);
    let starts = cursor.read(app.world.events::<CollisionStarted>()).count();
    assert_eq!(starts, 1, "overlap should emit one start event");
    tick(&mut app, 4);
    assert_eq!(
        cursor.read(app.world.events::<CollisionStarted>()).count(),
        0
    );
}

#[test]
fn invalid_gravity_is_reported_without_running_a_step() {
    let mut app = physics_app();
    app.world.resource_mut::<PhysicsConfig>().gravity = Vec3::splat(f32::NAN);
    tick(&mut app, 1);
    assert!(app
        .world
        .resource::<PhysicsDiagnostics>()
        .entries
        .iter()
        .any(|entry| entry.message.contains("gravity")));
}

#[test]
fn sensor_events_are_emitted_for_each_sensor_and_exit_on_separation() {
    use crate::SensorEntered;
    use crate::SensorExited;

    let mut app = physics_app();
    let mut trigger = Collider::ball(1.0);
    trigger.sensor = true;
    let sensor = app
        .world
        .spawn((Transform3D::from_translation(Vec3::ZERO), trigger));
    let other = app.world.spawn((
        Transform3D::from_translation(Vec3::new(0.25, 0.0, 0.0)),
        RigidBody::dynamic(),
        Collider::ball(0.25),
    ));
    let mut entered = EventCursor::<SensorEntered>::default();
    let mut exited = EventCursor::<SensorExited>::default();

    tick(&mut app, 1);
    let events: Vec<_> = entered
        .read(app.world.events::<SensorEntered>())
        .copied()
        .collect();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].sensor, sensor);
    assert_eq!(events[0].other, other);

    app.world
        .insert(
            other,
            Transform3D::from_translation(Vec3::new(5.0, 0.0, 0.0)),
        )
        .unwrap();
    tick(&mut app, 1);
    let events: Vec<_> = exited
        .read(app.world.events::<SensorExited>())
        .copied()
        .collect();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].sensor, sensor);
    assert_eq!(events[0].other, other);
}

#[test]
fn raycast_obeys_ribbon_collision_layer_filtering() {
    use crate::CollisionLayers;

    let mut app = physics_app();
    let target = app.world.spawn((
        Transform3D::from_translation(Vec3::ZERO),
        Collider::cuboid(Vec3::splat(1.0)),
        CollisionLayers::new(0b0010, 0b0001),
    ));
    tick(&mut app, 1);

    let mut query = RaycastQuery::new(Vec3::new(0.0, 5.0, 0.0), -Vec3::Y, 10.0);
    query.layers = CollisionLayers::new(0b0001, 0b0010);
    assert_eq!(raycast(&app.world, query).unwrap().entity, target);

    query.layers = CollisionLayers::new(0b0100, 0b0100);
    assert!(raycast(&app.world, query).is_none());
}

#[test]
fn removing_a_collider_removes_the_backend_body_without_despawning_entity() {
    let mut app = physics_app();
    let entity = app.world.spawn((
        Transform3D::from_translation(Vec3::ZERO),
        Collider::ball(0.5),
    ));
    tick(&mut app, 1);
    assert_eq!(app.world.resource::<PhysicsDebugData>().colliders.len(), 1);

    app.world.remove::<Collider>(entity).unwrap();
    tick(&mut app, 1);
    assert!(app.world.contains(entity));
    assert!(app
        .world
        .resource::<PhysicsDebugData>()
        .colliders
        .is_empty());
    assert!(raycast(
        &app.world,
        RaycastQuery::new(Vec3::new(0.0, 2.0, 0.0), -Vec3::Y, 5.0)
    )
    .is_none());
}

#[test]
fn character_controller_uses_rapier_to_slide_along_a_wall() {
    use crate::{CharacterController, CharacterMovementIntent, CharacterMovementResult};

    let mut app = physics_app();
    app.world.spawn((
        Transform3D::from_translation(Vec3::new(0.0, -0.5, 0.0)),
        Collider::cuboid(Vec3::new(5.0, 0.5, 5.0)),
    ));
    app.world.spawn((
        Transform3D::from_translation(Vec3::new(0.7, 0.8, 0.0)),
        Collider::cuboid(Vec3::new(0.1, 1.0, 2.0)),
    ));
    let character = app.world.spawn((
        Transform3D::from_translation(Vec3::new(0.0, 0.76, 0.0)),
        RigidBody::kinematic_position_based(),
        Collider::capsule_y(0.5, 0.25),
        CharacterController::default(),
        CharacterMovementIntent::default(),
    ));

    // Establish broad-phase entries before requesting a move.
    tick(&mut app, 1);
    app.world
        .insert(
            character,
            CharacterMovementIntent {
                desired_displacement: Vec3::new(1.0, 0.0, 0.7),
            },
        )
        .unwrap();
    tick(&mut app, 1);

    let result = *app
        .world
        .get::<CharacterMovementResult>(character)
        .expect("controller stores its last result");
    assert!(
        result.obstructed,
        "wall should obstruct the requested move: {result:?}"
    );
    assert!(
        result.applied_displacement.x < 0.65,
        "character passed through wall: {result:?}"
    );
    assert!(
        result.applied_displacement.z > 0.2,
        "character did not slide along wall: {result:?}"
    );
}

#[test]
fn character_controller_sees_new_colliders_on_its_first_fixed_step() {
    use crate::{CharacterController, CharacterMovementIntent, CharacterMovementResult};

    let mut app = physics_app();
    app.world.spawn((
        Transform3D::from_translation(Vec3::new(0.7, 1.0, 0.0)),
        Collider::cuboid(Vec3::new(0.1, 1.0, 2.0)),
    ));
    let character = app.world.spawn((
        Transform3D::from_translation(Vec3::new(0.0, 0.76, 0.0)),
        RigidBody::kinematic_position_based(),
        Collider::capsule_y(0.5, 0.25),
        CharacterController::default(),
        CharacterMovementIntent {
            desired_displacement: Vec3::new(1.0, 0.0, 0.0),
        },
    ));

    // No warm-up step: the wall is first registered in the same PreUpdate immediately before
    // the character's initial shape-cast query. It must not be absent from the query BVH.
    tick(&mut app, 1);

    let result = *app
        .world
        .get::<CharacterMovementResult>(character)
        .expect("character controller result");
    assert!(
        result.obstructed,
        "new wall should obstruct first movement: {result:?}"
    );
    assert!(
        result.applied_displacement.x < 0.5,
        "character should not pass through a wall added before its first step: {result:?}"
    );
}

#[test]
fn moved_fixed_obstacle_is_visible_to_character_query_in_the_same_fixed_step() {
    use crate::{CharacterController, CharacterMovementIntent, CharacterMovementResult};

    let mut app = physics_app();
    let wall = app.world.spawn((
        Transform3D::from_translation(Vec3::new(5.0, 0.76, 0.0)),
        Collider::cuboid(Vec3::new(0.1, 1.0, 2.0)),
    ));
    let character = app.world.spawn((
        Transform3D::from_translation(Vec3::new(0.0, 0.76, 0.0)),
        RigidBody::kinematic_position_based(),
        Collider::capsule_y(0.5, 0.25),
        CharacterController::default(),
        CharacterMovementIntent::default(),
    ));

    // Register the obstacle once, then move its ECS pose immediately before the character's next
    // fixed-step query. Fixed/scripted obstacles must refresh both the collider pose and query AABB
    // during PreUpdate; waiting for PhysicsPipeline::step would be too late for this shape cast.
    tick(&mut app, 1);
    app.world
        .insert(
            wall,
            Transform3D::from_translation(Vec3::new(0.7, 0.76, 0.0)),
        )
        .expect("wall transform exists");
    app.world
        .insert(
            character,
            CharacterMovementIntent {
                desired_displacement: Vec3::X,
            },
        )
        .expect("character movement intent exists");
    tick(&mut app, 1);

    let result = *app
        .world
        .get::<CharacterMovementResult>(character)
        .expect("character movement result");
    assert!(
        result.obstructed,
        "moved wall should block the character: {result:?}"
    );
    assert!(
        result.applied_displacement.x < 0.5,
        "character queried the wall's stale pose: {result:?}"
    );
}

#[test]
fn character_controller_teleport_is_not_overwritten_by_same_step_intent() {
    use crate::{CharacterController, CharacterMovementIntent, CharacterMovementResult};

    let mut app = physics_app();
    let character = app.world.spawn((
        Transform3D::from_translation(Vec3::new(0.0, 0.76, 0.0)),
        RigidBody::kinematic_position_based(),
        Collider::capsule_y(0.5, 0.25),
        CharacterController::default(),
        CharacterMovementIntent::default(),
    ));
    tick(&mut app, 1);

    app.world
        .insert(
            character,
            Transform3D::from_translation(Vec3::new(8.2, 0.76, -2.0)),
        )
        .expect("character transform exists");
    app.world
        .insert(
            character,
            CharacterMovementIntent {
                desired_displacement: Vec3::ZERO,
            },
        )
        .expect("character movement intent exists");
    tick(&mut app, 1);

    let transform = *app
        .world
        .get::<Transform3D>(character)
        .expect("character transform retained");
    assert!(
        (transform.translation.x - 8.2).abs() < 1.0e-3
            && (transform.translation.z + 2.0).abs() < 1.0e-3,
        "teleport should remain authoritative through the fixed step: {transform:?}"
    );
    let result = *app
        .world
        .get::<CharacterMovementResult>(character)
        .expect("character controller result");
    assert!(result.applied_displacement.length_squared() < 1.0e-6);
}

#[test]
fn character_controller_does_not_treat_sensors_as_solid_obstacles() {
    use crate::{CharacterController, CharacterMovementIntent, CharacterMovementResult};

    let mut app = physics_app();
    app.world.spawn((
        Transform3D::from_translation(Vec3::new(0.0, -0.5, 0.0)),
        Collider::cuboid(Vec3::new(5.0, 0.5, 5.0)),
    ));
    let mut sensor = Collider::cuboid(Vec3::new(0.25, 0.5, 0.15));
    sensor.sensor = true;
    app.world.spawn((
        Transform3D::from_translation(Vec3::new(0.0, 0.8, 0.5)),
        sensor,
    ));
    let character = app.world.spawn((
        Transform3D::from_translation(Vec3::new(0.0, 0.76, 0.0)),
        RigidBody::kinematic_position_based(),
        Collider::capsule_y(0.5, 0.25),
        CharacterController::default(),
        CharacterMovementIntent::default(),
    ));

    // Populate the broad phase before issuing a move through the trigger volume.
    tick(&mut app, 1);
    app.world
        .insert(
            character,
            CharacterMovementIntent {
                desired_displacement: Vec3::new(0.0, 0.0, 1.0),
            },
        )
        .unwrap();
    tick(&mut app, 1);

    let result = *app
        .world
        .get::<CharacterMovementResult>(character)
        .expect("controller stores its last result");
    assert!(
        result.applied_displacement.z > 0.9,
        "sensor trigger incorrectly blocked the character: {result:?}"
    );
    assert!(
        !result.obstructed,
        "sensor should not obstruct movement: {result:?}"
    );
}

#[test]
fn parented_dynamic_body_writes_back_in_local_space() {
    use ribbon_scene::{GlobalTransform3D, HierarchyExt};

    let mut app = physics_app();
    let parent = app.world.spawn((Transform3D {
        translation: Vec3::new(4.0, 0.0, 0.0),
        scale: Vec3::splat(2.0),
        ..Transform3D::default()
    },));
    let child = app.world.spawn((
        Transform3D::from_translation(Vec3::new(0.0, 2.0, 0.0)),
        RigidBody::dynamic(),
        Collider::ball(0.25),
    ));
    app.world.set_parent(child, parent).unwrap();

    tick(&mut app, 30);
    let local = *app.world.get::<Transform3D>(child).unwrap();
    let global = *app.world.get::<GlobalTransform3D>(child).unwrap();
    assert!(
        local.translation.y < 2.0,
        "local pose should reflect gravity: {local:?}"
    );
    assert!(
        (global.translation.x - 4.0).abs() < 1.0e-3,
        "world X drifted: {global:?}"
    );
    assert!(
        global.translation.y < 4.0,
        "world pose should reflect gravity: {global:?}"
    );
}

#[test]
fn parented_physics_rejects_non_uniform_ancestor_scale() {
    use ribbon_scene::HierarchyExt;

    let mut app = physics_app();
    let parent = app.world.spawn((Transform3D {
        scale: Vec3::new(2.0, 1.0, 1.0),
        ..Transform3D::default()
    },));
    let child = app.world.spawn((
        Transform3D::from_translation(Vec3::new(0.0, 1.0, 0.0)),
        Collider::cuboid(Vec3::splat(0.5)),
    ));
    app.world.set_parent(child, parent).unwrap();

    tick(&mut app, 1);
    assert!(app
        .world
        .resource::<PhysicsDebugData>()
        .colliders
        .is_empty());
    assert!(app
        .world
        .resource::<PhysicsDiagnostics>()
        .entries
        .iter()
        .any(|entry| entry.entity == Some(child) && entry.message.contains("uniform scale")));
}

#[test]
fn collision_stop_event_is_emitted_after_separation() {
    use crate::CollisionStopped;

    let mut app = physics_app();
    app.world.spawn((
        Transform3D::from_translation(Vec3::ZERO),
        Collider::ball(1.0),
    ));
    let moving = app.world.spawn((
        Transform3D::from_translation(Vec3::new(0.25, 0.0, 0.0)),
        RigidBody::dynamic(),
        Collider::ball(0.25),
    ));
    let mut cursor = EventCursor::<CollisionStopped>::default();

    tick(&mut app, 1);
    app.world
        .insert(
            moving,
            Transform3D::from_translation(Vec3::new(5.0, 0.0, 0.0)),
        )
        .unwrap();
    tick(&mut app, 1);

    let events: Vec<_> = cursor
        .read(app.world.events::<CollisionStopped>())
        .copied()
        .collect();
    assert_eq!(events.len(), 1);
}

#[test]
fn one_frame_can_run_multiple_engine_fixed_steps() {
    let mut batched = physics_app();
    let mut stepped = physics_app();
    let batched_body = batched.world.spawn((
        Transform3D::from_translation(Vec3::new(0.0, 2.0, 0.0)),
        RigidBody::dynamic(),
        Collider::ball(0.25),
    ));
    let stepped_body = stepped.world.spawn((
        Transform3D::from_translation(Vec3::new(0.0, 2.0, 0.0)),
        RigidBody::dynamic(),
        Collider::ball(0.25),
    ));

    batched.world.resource_mut::<Time>().delta = Duration::from_millis(51);
    batched.update();
    tick(&mut stepped, 3);

    let batched_y = batched
        .world
        .get::<Transform3D>(batched_body)
        .unwrap()
        .translation
        .y;
    let stepped_y = stepped
        .world
        .get::<Transform3D>(stepped_body)
        .unwrap()
        .translation
        .y;
    assert!((batched_y - stepped_y).abs() < 1.0e-4);
}

#[test]
fn velocity_kinematic_body_moves_from_its_configured_velocity() {
    let mut app = physics_app();
    let body = app.world.spawn((
        Transform3D::from_translation(Vec3::ZERO),
        RigidBody {
            kind: crate::RigidBodyKind::KinematicVelocityBased,
            linear_velocity: Vec3::X,
            ..RigidBody::default()
        },
        Collider::ball(0.25),
    ));

    tick(&mut app, 30);
    let transform = *app.world.get::<Transform3D>(body).unwrap();
    assert!(
        transform.translation.x > 0.4,
        "kinematic body did not advance: {transform:?}"
    );
}
