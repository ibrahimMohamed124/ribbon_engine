//! Single-room horror environment on the native-surface renderer.
//!
//! Controls: WASD to walk, mouse to look, Shift to move faster, Esc to release
//! the pointer, and left-click to capture it again. Run `cargo run -p horror_slice`.

use engine_app::{App, AppExit, Stage, Time, World};
use engine_assets::{AssetPlugin, AssetServer, FsSource};
use engine_events::WorldEventsExt;
use engine_platform::input::Key;
use engine_platform::runtime::CursorGrabState;
use engine_platform::{Input, PlatformConfig, PlatformPlugin};
use engine_plugins::AppPluginExt;
use engine_render::gpu_window::{ClearColor, GpuWindowPlugin};
use engine_scene::ScenePlugin;
use plugin_gltf::{GltfLoader, ModelAsset};
use plugin_physics::{slide_capsule_xz, Aabb3};
use plugin_render3d::glam::{Quat, Vec3};
use plugin_render3d::{
    Camera3D, FogSettings, ModelInstance, PointLight, Render3dPlugin, SpotLight, SunLight,
    Transform3D,
};
use serde::Deserialize;
use std::collections::HashMap;
use std::error::Error;
use std::path::Path;

struct FrameLimit(Option<u64>);

fn enforce_frame_limit(world: &mut World) {
    let should_exit = world
        .resource::<FrameLimit>()
        .0
        .is_some_and(|limit| world.resource::<Time>().frame >= limit);
    if should_exit {
        world.send_event(AppExit);
    }
}

#[derive(Debug, Deserialize)]
struct RoomDefinition {
    version: u32,
    player_spawn: [f32; 3],
    player_yaw: f32,
    monster_spawn: [f32; 3],
    monster_patrol_waypoints: Vec<[f32; 3]>,
    save_point: [f32; 3],
    entities: Vec<RoomEntity>,
    static_colliders: Vec<StaticCollider>,
}

#[derive(Debug, Deserialize)]
struct RoomEntity {
    name: String,
    kind: String,
    asset: String,
    position: [f32; 3],
    yaw: f32,
    scale: [f32; 3],
    collider_half_extents: Option<[f32; 3]>,
}

#[derive(Debug, Deserialize)]
struct StaticCollider {
    name: String,
    center: [f32; 3],
    half_extents: [f32; 3],
}

/// Game-specific state for first-person input and camera-following flashlight.
/// Static collision math itself lives in `plugin_physics` for reuse by other games.
struct FirstPersonController {
    yaw: f32,
    pitch: f32,
    colliders: Vec<Aabb3>,
    cursor_initialized: bool,
    skip_mouse_delta: bool,
}

impl FirstPersonController {
    fn from_room(room: &RoomDefinition) -> Self {
        let mut colliders: Vec<Aabb3> = room
            .static_colliders
            .iter()
            .map(|collider| Aabb3::new(collider.center, collider.half_extents))
            .collect();

        // The shell mesh is visual; its authored wall/floor boxes above are the actual
        // collision geometry. Other prop collider centers are based at each model's feet.
        for entity in &room.entities {
            if entity.kind == "StaticRoomGeometry" {
                continue;
            }
            if let Some(half_extents) = entity.collider_half_extents {
                let center = [
                    entity.position[0],
                    entity.position[1] + half_extents[1],
                    entity.position[2],
                ];
                colliders.push(Aabb3::new(center, half_extents));
            }
        }

        Self {
            yaw: room.player_yaw,
            pitch: -0.12,
            colliders,
            cursor_initialized: false,
            skip_mouse_delta: true,
        }
    }
}

fn load_room() -> Result<RoomDefinition, Box<dyn Error>> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/room.ron");
    let source = std::fs::read_to_string(&path)?;
    let room: RoomDefinition = ron::from_str(&source)?;
    if room.version != 1 {
        return Err(format!("unsupported room.ron version {}", room.version).into());
    }
    Ok(room)
}

/// Apply WASD/mouse input, collide against the authored room, and keep the
/// flashlight mounted to the view direction. No platform-specific window calls
/// are made by the example; cursor capture goes through the platform input resource.
fn first_person_controller(world: &mut World) {
    let dt = world
        .resource::<Time>()
        .delta
        .as_secs_f32()
        .clamp(0.0, 0.05);
    let (
        forward_down,
        backward_down,
        left_down,
        right_down,
        sprint_down,
        escape_pressed,
        mouse_left_down,
        focused,
        cursor_state,
        mouse_delta,
    ) = {
        let input = world.resource::<Input>();
        (
            input.is_down(Key::W),
            input.is_down(Key::S),
            input.is_down(Key::A),
            input.is_down(Key::D),
            input.is_down(Key::LeftShift) || input.is_down(Key::RightShift),
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
            // Capture on the first focused frame, like a conventional first-person game.
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

        let mut camera_pose = None;
        if let Some((_, (pose, _camera))) = world
            .query_mut::<(&mut Transform3D, &Camera3D)>()
            .into_iter()
            .next()
        {
            let mut candidate = *pose;
            if is_captured {
                let mut forward: f32 = (if forward_down { 1.0 } else { 0.0 })
                    - (if backward_down { 1.0 } else { 0.0 });
                let mut strafe: f32 =
                    (if right_down { 1.0 } else { 0.0 }) - (if left_down { 1.0 } else { 0.0 });
                let length = (forward * forward + strafe * strafe).sqrt();
                if length > 1.0 {
                    forward /= length;
                    strafe /= length;
                }
                let speed = if sprint_down { 4.7 } else { 3.0 };
                let forward_x = controller.yaw.sin();
                let forward_z = -controller.yaw.cos();
                let right_x = controller.yaw.cos();
                let right_z = controller.yaw.sin();
                let desired_delta = [
                    (forward_x * forward + right_x * strafe) * speed * dt,
                    (forward_z * forward + right_z * strafe) * speed * dt,
                ];
                let moved = slide_capsule_xz(
                    candidate.translation.to_array(),
                    desired_delta,
                    0.28, // body radius
                    1.72, // body height
                    1.62, // eye height above the floor
                    &controller.colliders,
                );
                candidate.translation = Vec3::from_array(moved);
            }
            // Positive gameplay yaw turns toward +X; camera local forward is -Z.
            candidate.rotation =
                Quat::from_rotation_y(-controller.yaw) * Quat::from_rotation_x(controller.pitch);
            *pose = candidate;
            camera_pose = Some(candidate);
        }

        if let Some(camera_pose) = camera_pose {
            let rotation = camera_pose.rotation;
            let direction = (rotation * Vec3::NEG_Z).normalize_or_zero();
            let right = rotation * Vec3::X;
            let up = rotation * Vec3::Y;
            let light_position =
                camera_pose.translation + direction * 0.20 + right * 0.035 - up * 0.025;
            for (_, light) in world.query_mut::<&mut SpotLight>() {
                light.position = light_position;
                light.direction = direction;
            }
        }
    });
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut frame_limit = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--frames" => {
                frame_limit = Some(
                    args.next()
                        .ok_or("--frames expects a number")?
                        .parse::<u64>()?,
                );
            }
            "--help" | "-h" => {
                println!(concat!(
                    "Voxelworks Horror Slice\n\n",
                    "Controls:\n",
                    "  WASD       Walk\n",
                    "  Mouse      Look around\n",
                    "  Shift      Walk faster\n",
                    "  Escape     Release/capture pointer\n",
                    "  Left click Capture pointer again after releasing it\n\n",
                    "Options:\n",
                    "  --frames N Exit after N frames\n",
                    "  -h, --help Show this help\n"
                ));
                return Ok(());
            }
            other => return Err(format!("unknown argument `{other}`").into()),
        }
    }

    let room = load_room()?;
    let asset_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/models");
    let mut app = App::new();
    app.insert_resource(PlatformConfig {
        title: "Voxelworks | Horror Slice".to_owned(),
        width: 1280,
        height: 720,
    });
    app.insert_resource(ClearColor([0.003, 0.004, 0.004, 1.0]));
    app.insert_resource(FrameLimit(frame_limit));
    app.insert_resource(SunLight {
        direction: Vec3::new(0.0, -1.0, -0.15),
        color: Vec3::ZERO,
        ambient: Vec3::splat(0.032),
    });
    app.insert_resource(FogSettings {
        color: Vec3::new(0.018, 0.025, 0.021),
        density: 0.006,
        base_height: 0.0,
        height_falloff: 0.08,
    });
    app.insert_resource(FirstPersonController::from_room(&room));

    // Native window / scene / asset / mesh rendering path; no offscreen readback bridge.
    app.add_plugin(PlatformPlugin)?;
    app.add_plugin(ScenePlugin)?;
    app.add_plugin(AssetPlugin)?;
    app.add_plugin(GpuWindowPlugin)?;
    app.add_plugin(Render3dPlugin)?;

    let handles = {
        let server = app.world.resource_mut::<AssetServer>();
        server.register_loader(GltfLoader);
        server.add_source(0, FsSource::new(asset_root));
        let mut map = HashMap::new();
        for entity in &room.entities {
            if !map.contains_key(&entity.asset) {
                let handle = server.load::<ModelAsset>(&entity.asset).map_err(|error| {
                    format!("could not load `{}`: {}", entity.asset, error.message)
                })?;
                map.insert(entity.asset.clone(), handle);
            }
        }
        map
    };

    let spawn = Vec3::from_array(room.player_spawn);
    let camera_rotation =
        Quat::from_rotation_y(-room.player_yaw) * Quat::from_rotation_x(-0.12_f32);
    app.world.spawn((
        Transform3D {
            translation: spawn,
            rotation: camera_rotation,
            ..Transform3D::default()
        },
        Camera3D {
            fov_y: 68.0_f32.to_radians(),
            near: 0.04,
            far: 80.0,
        },
    ));

    for entity in &room.entities {
        let handle = *handles
            .get(&entity.asset)
            .ok_or("room entity asset handle missing")?;
        app.world.spawn((
            Transform3D {
                translation: Vec3::from_array(entity.position),
                rotation: Quat::from_rotation_y(entity.yaw),
                scale: Vec3::from_array(entity.scale),
            },
            ModelInstance { model: handle },
        ));
        println!(
            "room entity: {} ({}) -> {}{}",
            entity.name,
            entity.kind,
            entity.asset,
            entity
                .collider_half_extents
                .map(|half| format!(", collider half-extents {half:?}"))
                .unwrap_or_default()
        );
    }

    // This reusable spotlight follows the camera pose every frame, including mouse look.
    let direction = (camera_rotation * Vec3::NEG_Z).normalize_or_zero();
    let right = camera_rotation * Vec3::X;
    let up = camera_rotation * Vec3::Y;
    app.world.spawn((SpotLight {
        position: spawn + direction * 0.20 + right * 0.035 - up * 0.025,
        direction,
        color: Vec3::new(0.84, 0.93, 0.82),
        intensity: 360.0,
        range: 17.0,
        inner_cone_angle: 20.0_f32.to_radians(),
        outer_cone_angle: 39.0_f32.to_radians(),
        casts_shadows: true,
    },));
    let save = Vec3::from_array(room.save_point);
    app.world.spawn((PointLight {
        position: save + Vec3::new(0.0, 1.05, 0.0),
        color: Vec3::new(0.66, 0.92, 0.72),
        intensity: 24.0,
        range: 6.0,
    },));

    let collider_names: Vec<_> = room
        .static_colliders
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    println!(
        "loaded room v{}: {} entities, {} static collision boxes, {} monster waypoints; monster at {:?}",
        room.version,
        room.entities.len(),
        collider_names.len(),
        room.monster_patrol_waypoints.len(),
        room.monster_spawn
    );
    println!(
        "Controls: WASD walk, mouse look, Shift faster, Esc release pointer; click to recapture."
    );
    app.add_system(Stage::Update, first_person_controller);
    app.add_system(Stage::Update, enforce_frame_limit);
    app.build()?.run();
    Ok(())
}
