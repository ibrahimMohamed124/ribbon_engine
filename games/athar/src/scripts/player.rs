//! First-person input and the Rapier-backed character-controller bridge.
//!
//! Mouse look is collected before the engine's fixed step. Movement intent is submitted in
//! `FixedUpdate` before `PhysicsPlugin` runs, and the camera is synchronized from the simulated
//! kinematic capsule after the fixed steps for the current rendered frame.

use ribbon_app::{FixedTime, World};
use ribbon_ecs::Entity;
use ribbon_platform::input::Key;
use ribbon_platform::runtime::CursorGrabState;
use ribbon_platform::Input;
use ribbon_plugin_physics::CharacterMovementIntent;
use ribbon_plugin_render3d::glam::{Quat, Vec3};
use ribbon_plugin_render3d::{SpotLight, Transform3D};

/// Capsule dimensions used by ATHAR's first-person character, in meters.
pub(crate) const PLAYER_RADIUS: f32 = 0.28;
pub(crate) const PLAYER_HALF_SEGMENT: f32 = 0.58;
pub(crate) const PLAYER_CAMERA_OFFSET: f32 = 0.79;

/// Game-specific player input/collision state. Collision shapes and queries belong to Rapier.
pub(crate) struct FirstPersonController {
    pub(crate) yaw: f32,
    pub(super) pitch: f32,
    pub(crate) player_entity: Option<Entity>,
    /// Keep the body on the room side of the exit until all village models are GPU-ready.
    pub(crate) exit_blocked: bool,
    pub(super) cursor_initialized: bool,
    pub(super) skip_mouse_delta: bool,
}

impl FirstPersonController {
    pub(crate) fn from_room(room: &crate::RoomDefinition) -> Self {
        Self {
            yaw: room.player_yaw,
            pitch: -0.12,
            player_entity: None,
            exit_blocked: true,
            cursor_initialized: false,
            skip_mouse_delta: true,
        }
    }

    pub(crate) fn player_body_translation(camera_translation: Vec3) -> Vec3 {
        camera_translation - Vec3::Y * PLAYER_CAMERA_OFFSET
    }
}

/// Update cursor capture and orientation before any engine fixed-physics step.
pub(crate) fn first_person_controller(world: &mut World) {
    let (escape_pressed, mouse_left_down, focused, cursor_state, mouse_delta) = {
        let input = world.resource::<Input>();
        (
            input.just_pressed(Key::Escape),
            input.mouse_buttons.left,
            input.focused,
            input.cursor_grab,
            input.mouse_delta,
        )
    };

    world.resource_scope::<FirstPersonController, _>(|world, controller| {
        let mut requested_cursor = cursor_state;
        if !focused {
            requested_cursor = CursorGrabState::Released;
            controller.skip_mouse_delta = true;
        } else if escape_pressed {
            requested_cursor = if cursor_state == CursorGrabState::Released {
                CursorGrabState::Locked
            } else {
                CursorGrabState::Released
            };
            controller.skip_mouse_delta = true;
        } else if !controller.cursor_initialized {
            requested_cursor = CursorGrabState::Locked;
            controller.cursor_initialized = true;
            controller.skip_mouse_delta = true;
        } else if cursor_state == CursorGrabState::Released && mouse_left_down {
            requested_cursor = CursorGrabState::Locked;
            controller.skip_mouse_delta = true;
        }
        world.resource_mut::<Input>().cursor_grab = requested_cursor;

        let is_captured = focused && requested_cursor != CursorGrabState::Released;
        let ignore_delta = controller.skip_mouse_delta || requested_cursor != cursor_state;
        controller.skip_mouse_delta = false;
        if is_captured && !ignore_delta {
            const SENSITIVITY: f32 = 0.0022;
            controller.yaw =
                (controller.yaw + mouse_delta.0 * SENSITIVITY).rem_euclid(std::f32::consts::TAU);
            controller.pitch = (controller.pitch - mouse_delta.1 * SENSITIVITY).clamp(-1.32, 1.32);
        }
    });
}

/// Submit one displacement per fixed step. This system is registered before PhysicsPlugin's
/// `FixedUpdate` system so the Rapier character controller consumes the same-step intent.
pub(crate) fn submit_character_movement(world: &mut World) {
    let (player_entity, yaw, exit_blocked) = {
        let controller = world.resource::<FirstPersonController>();
        (
            controller.player_entity,
            controller.yaw,
            controller.exit_blocked,
        )
    };
    let Some(player_entity) = player_entity else {
        return;
    };

    let (forward_down, backward_down, left_down, right_down, sprint_down, focused, cursor_state) = {
        let input = world.resource::<Input>();
        (
            input.is_down(Key::W),
            input.is_down(Key::S),
            input.is_down(Key::A),
            input.is_down(Key::D),
            input.is_down(Key::LeftShift) || input.is_down(Key::RightShift),
            input.focused,
            input.cursor_grab,
        )
    };
    let mut displacement = if focused && cursor_state != CursorGrabState::Released {
        let forward =
            (if forward_down { 1.0 } else { 0.0 }) - (if backward_down { 1.0 } else { 0.0 });
        let strafe = (if right_down { 1.0 } else { 0.0 }) - (if left_down { 1.0 } else { 0.0 });
        let speed = if sprint_down { 4.7 } else { 3.0 };
        let dt = world.resource::<FixedTime>().step.as_secs_f32();
        let delta = movement_delta(yaw, forward, strafe, speed, dt);
        Vec3::new(delta[0], 0.0, delta[1])
    } else {
        Vec3::ZERO
    };
    if exit_blocked {
        if let Some(position) = world
            .get::<Transform3D>(player_entity)
            .map(|transform| transform.translation)
        {
            displacement = clamp_blocked_exit_movement(position, displacement);
        }
    }
    let _ = world.insert(
        player_entity,
        CharacterMovementIntent {
            desired_displacement: displacement,
        },
    );
}

/// Follow the physics capsule after all fixed steps and update the rendered camera/light pose.
pub(crate) fn sync_player_camera(world: &mut World) {
    let (player_entity, yaw, pitch) = {
        let controller = world.resource::<FirstPersonController>();
        (controller.player_entity, controller.yaw, controller.pitch)
    };
    let camera_entity = world
        .resource::<crate::scripts::AdventureState>()
        .camera_entity;
    let Some(player_entity) = player_entity else {
        return;
    };
    let Some(body_translation) = world
        .get::<Transform3D>(player_entity)
        .map(|transform| transform.translation)
    else {
        return;
    };

    let rotation = Quat::from_rotation_y(-yaw) * Quat::from_rotation_x(pitch);
    let camera_pose = {
        let Some(mut transform) = world.get_mut::<Transform3D>(camera_entity) else {
            return;
        };
        transform.translation = body_translation + Vec3::Y * PLAYER_CAMERA_OFFSET;
        transform.rotation = rotation;
        *transform
    };
    let direction = (camera_pose.rotation * Vec3::NEG_Z).normalize_or_zero();
    let right = camera_pose.rotation * Vec3::X;
    let up = camera_pose.rotation * Vec3::Y;
    let light_position = camera_pose.translation + direction * 0.20 + right * 0.035 - up * 0.025;
    for (_, light) in world.query_mut::<&mut SpotLight>() {
        light.position = light_position;
        light.direction = direction;
    }
}

/// Keep the player in the room while required village resources are being prepared. Only the
/// outward crossing of the authored doorway is clamped; lateral movement along the wall remains
/// available and the rest of the room uses Rapier normally.
fn clamp_blocked_exit_movement(position: Vec3, displacement: Vec3) -> Vec3 {
    const EXIT_BLOCK_MAX_X: f32 = 5.94;
    const EXIT_CENTER_Z: f32 = -2.0;
    const EXIT_HALF_WIDTH: f32 = 0.92;
    let next = position + displacement;
    if displacement.x > 0.0
        && (position.z - EXIT_CENTER_Z).abs() < EXIT_HALF_WIDTH
        && (next.z - EXIT_CENTER_Z).abs() < EXIT_HALF_WIDTH
        && next.x > EXIT_BLOCK_MAX_X
    {
        Vec3::new(
            (EXIT_BLOCK_MAX_X - position.x).max(0.0),
            displacement.y,
            displacement.z,
        )
    } else {
        displacement
    }
}

/// Convert normalized input axes to a world-space X/Z displacement for one simulation step.
fn movement_delta(yaw: f32, forward: f32, strafe: f32, speed: f32, dt: f32) -> [f32; 2] {
    let mut forward = forward;
    let mut strafe = strafe;
    let length = (forward * forward + strafe * strafe).sqrt();
    if length > 1.0 {
        forward /= length;
        strafe /= length;
    }
    let forward_x = yaw.sin();
    let forward_z = -yaw.cos();
    let right_x = yaw.cos();
    let right_z = yaw.sin();
    [
        (forward_x * forward + right_x * strafe) * speed * dt,
        (forward_z * forward + right_z * strafe) * speed * dt,
    ]
}

#[cfg(test)]
mod tests {
    use super::{
        clamp_blocked_exit_movement, movement_delta, FirstPersonController, PLAYER_HALF_SEGMENT,
        PLAYER_RADIUS,
    };
    use ribbon_app::{App, Time};
    use ribbon_plugin_physics::{
        CharacterController, CharacterMovementIntent, Collider, PhysicsPlugin, RigidBody,
    };
    use ribbon_plugin_render3d::glam::Vec3;
    use ribbon_plugins::AppPluginExt;
    use ribbon_scene::{ScenePlugin, Transform3D};
    use std::f32::consts::FRAC_PI_2;
    use std::time::Duration;

    #[test]
    fn blocked_exit_clamps_only_outward_doorway_crossing() {
        let position = Vec3::new(5.8, 0.0, -2.0);
        let clamped = clamp_blocked_exit_movement(position, Vec3::new(0.4, 0.0, 0.0));
        assert!((clamped.x - 0.14).abs() < 1.0e-5);
        assert_eq!(
            clamp_blocked_exit_movement(position, Vec3::new(-0.2, 0.0, 0.0)),
            Vec3::new(-0.2, 0.0, 0.0)
        );
        assert_eq!(
            clamp_blocked_exit_movement(Vec3::new(5.8, 0.0, 0.0), Vec3::new(0.4, 0.0, 0.0)),
            Vec3::new(0.4, 0.0, 0.0)
        );
    }

    #[test]
    fn forward_movement_matches_camera_heading() {
        let along_negative_z = movement_delta(0.0, 1.0, 0.0, 3.0, 0.5);
        assert!(along_negative_z[0].abs() < 1e-6);
        assert!((along_negative_z[1] + 1.5).abs() < 1e-6);

        let along_positive_x = movement_delta(FRAC_PI_2, 1.0, 0.0, 3.0, 0.5);
        assert!((along_positive_x[0] - 1.5).abs() < 1e-6);
        assert!(along_positive_x[1].abs() < 1e-6);
    }

    #[test]
    fn diagonal_movement_is_normalized_and_idle_is_stationary() {
        let diagonal = movement_delta(0.37, 1.0, 1.0, 4.7, 0.05);
        let distance = (diagonal[0] * diagonal[0] + diagonal[1] * diagonal[1]).sqrt();
        assert!((distance - 4.7 * 0.05).abs() < 1e-6);
        assert_eq!(movement_delta(1.2, 0.0, 0.0, 3.0, 0.05), [0.0, 0.0]);
    }

    #[test]
    fn athar_character_movement_is_resolved_by_rapier_against_engine_colliders() {
        let mut app = App::new();
        app.add_plugin(ScenePlugin).expect("scene plugin");
        app.add_plugin(PhysicsPlugin).expect("physics plugin");
        app.world.spawn((
            Transform3D::from_translation(Vec3::new(0.0, -0.5, 0.0)),
            Collider::cuboid(Vec3::new(5.0, 0.5, 5.0)),
        ));
        app.world.spawn((
            Transform3D::from_translation(Vec3::new(0.7, 1.0, 0.0)),
            Collider::cuboid(Vec3::new(0.1, 1.0, 2.0)),
        ));
        // Match ATHAR's key: overlap events are useful for gameplay, but its sensor must not
        // interfere with the Rapier character's shape cast while the player moves toward it.
        let mut key_sensor = Collider::cuboid(Vec3::new(0.25, 0.5, 0.15));
        key_sensor.sensor = true;
        app.world.spawn((
            Transform3D::from_translation(Vec3::new(0.0, 0.86, 0.5)),
            key_sensor,
        ));
        let character = app.world.spawn((
            Transform3D::from_translation(Vec3::new(0.0, 0.86, 0.0)),
            RigidBody::kinematic_position_based(),
            Collider::capsule_y(PLAYER_HALF_SEGMENT, PLAYER_RADIUS),
            CharacterController::default(),
            CharacterMovementIntent::default(),
        ));

        // First fixed step reconciles room geometry and establishes the Rapier broad phase.
        app.world.resource_mut::<Time>().delta = Duration::from_secs_f64(1.0 / 60.0);
        app.update();
        app.world
            .insert(
                character,
                CharacterMovementIntent {
                    desired_displacement: Vec3::new(1.0, 0.0, 0.7),
                },
            )
            .expect("movement intent component");
        app.world.resource_mut::<Time>().delta = Duration::from_secs_f64(1.0 / 60.0);
        app.update();

        let result = *app
            .world
            .get::<ribbon_plugin_physics::CharacterMovementResult>(character)
            .expect("Rapier controller result");
        assert!(
            result.obstructed,
            "wall must obstruct the movement: {result:?}"
        );
        assert!(
            result.applied_displacement.x < 0.65,
            "character passed through the wall: {result:?}"
        );
        assert!(
            result.applied_displacement.z > 0.2,
            "character should slide along the wall: {result:?}"
        );
        let body_spawn = FirstPersonController::player_body_translation(Vec3::new(0.0, 1.65, 0.0));
        assert!((body_spawn.y - 0.86).abs() < 1.0e-6);
    }
}
