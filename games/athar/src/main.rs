//! Horror Slice: escape the locked room, then explore a rain-soaked abandoned village.
//!
//! Controls: WASD to walk, mouse to look, Shift to move faster, E to interact, Esc to release
//! the pointer, and left-click to capture it again. Run `cargo run -p athar`.
//! Optional profile capture: `cargo run -p athar --release -- --frames 1800 --profile-csv target/profiles/athar.csv`.

mod audio;
mod scripts;

use ribbon_app::{App, AppExit, Stage, Time, World};
use ribbon_assets::{AssetEvent, AssetPlugin, AssetRef, AssetServer, FsSource};
use ribbon_events::{EventCursor, WorldEventsExt};
use ribbon_platform::{PlatformConfig, PlatformPlugin};
use ribbon_plugin_audio::{AcousticZone, AudioPlugin, AudioVec3, AudioZoneVolume};
use ribbon_plugin_gltf::{GltfLoader, ModelAsset};
use ribbon_plugin_physics::{
    CharacterController, CharacterMovementIntent, Collider, PhysicsPlugin, RigidBody,
};
use ribbon_plugin_render3d::glam::{Quat, Vec3};
use ribbon_plugin_render3d::{
    Camera3D, FogSettings, ModelInstance, PointLight, Render3dPlugin, SpotLight, SunLight,
    Transform3D,
};
use ribbon_plugin_streaming::{
    RegionDefinition, RegionId, RegionPolicy, SceneDefinition, SceneId, StreamObserver,
    StreamingConfig, StreamingCoordinator, StreamingPlugin,
};
use ribbon_plugins::AppPluginExt;
use ribbon_render::gpu_window::{
    ClearColor, GpuWindow, GpuWindowPlugin, WindowTextOverlay, WindowTextOverlayPass,
};
use ribbon_render::profiling::RenderProfilePlugin;
use ribbon_scene::ScenePlugin;
use scripts::player::{
    first_person_controller, submit_character_movement, sync_player_camera, FirstPersonController,
    PLAYER_HALF_SEGMENT, PLAYER_RADIUS,
};
use scripts::{AdventureState, GameModelHandles};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::path::{Path, PathBuf};

struct FrameLimit(Option<u64>);

/// Pins room models while they stream in and throughout gameplay.
struct GameModelLeases {
    // One strong lease per asset path. A CPU retry replaces the old generation instead of
    // accumulating duplicate strong references across repeated user retries.
    _leases: HashMap<String, AssetRef<ModelAsset>>,
}

/// Per-system cursor for asynchronous load failures and hot-reload diagnostics.
struct AssetEventCursor(EventCursor<AssetEvent>);

fn report_asset_failures(world: &mut World) {
    world.resource_scope::<AssetEventCursor, _>(|world, cursor| {
        let messages: Vec<_> = cursor
            .0
            .read(world.events::<AssetEvent>())
            .filter_map(|event| match event {
                AssetEvent::Failed { path, error, .. } => {
                    Some(format!("asset load failed for `{path}`: {error}"))
                }
                AssetEvent::ReloadFailed { path, error, .. } => Some(format!(
                    "asset hot reload failed for `{path}`; keeping last good asset: {error}"
                )),
                _ => None,
            })
            .collect();
        for message in messages {
            eprintln!("{message}");
        }
    });
}

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

fn load_room(asset_root: &Path) -> Result<RoomDefinition, Box<dyn Error>> {
    let path = asset_root.join("room.ron");
    let source = std::fs::read_to_string(&path)?;
    let room: RoomDefinition = ron::from_str(&source)?;
    if room.version != 1 {
        return Err(format!("unsupported room.ron version {}", room.version).into());
    }
    Ok(room)
}

/// Resolve the distributable `assets/` root without relying on a build-machine checkout path.
/// Release binaries intentionally have no `CARGO_MANIFEST_DIR` fallback.
fn resolve_asset_root(explicit: Option<PathBuf>) -> Result<PathBuf, Box<dyn Error>> {
    let mut candidates: Vec<(&'static str, PathBuf)> = Vec::new();
    if let Some(path) = explicit {
        candidates.push(("--assets", path));
    }
    if let Some(path) = std::env::var_os("RIBBON_ATHAR_ASSETS") {
        candidates.push(("RIBBON_ATHAR_ASSETS", PathBuf::from(path)));
    }
    if let Ok(executable) = std::env::current_exe() {
        if let Some(directory) = executable.parent() {
            candidates.push(("executable-adjacent assets", directory.join("assets")));
        }
    }
    #[cfg(debug_assertions)]
    candidates.push((
        "development fallback",
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets"),
    ));

    let mut rejected = Vec::new();
    for (source, candidate) in candidates {
        if candidate.join("room.ron").is_file() && candidate.join("models").is_dir() {
            return Ok(candidate.canonicalize().unwrap_or(candidate));
        }
        rejected.push(format!("{source}: {}", candidate.display()));
    }
    Err(format!(
        "could not resolve ATHAR assets; each candidate must contain room.ron and models/: {}. Supply --assets PATH or RIBBON_ATHAR_ASSETS",
        rejected.join("; ")
    )
    .into())
}

fn validate_asset_root(asset_root: &Path, room: &RoomDefinition) -> Result<usize, Box<dyn Error>> {
    let mut required_models: HashSet<&str> = room
        .entities
        .iter()
        .map(|entity| entity.asset.as_str())
        .collect();
    required_models.extend(crate::scripts::village::VILLAGE_MODEL_ASSETS);
    for model_name in &required_models {
        let model = asset_root.join("models").join(model_name);
        if !model.is_file() {
            return Err(format!(
                "ATHAR package is missing required model `{}`",
                model.display()
            )
            .into());
        }
    }
    for required in ["CREDITS.md", "asset_manifest.json"] {
        if !asset_root.join(required).is_file() {
            return Err(format!("ATHAR package is missing assets/{required}").into());
        }
    }
    for sound in crate::audio::ATHAR_AUDIO_ASSETS {
        if !asset_root.join("audio").join(sound).is_file() {
            return Err(format!("ATHAR package is missing audio asset audio/{sound}").into());
        }
    }
    Ok(required_models.len())
}

/// Spawn an engine-owned physics proxy for a box-shaped room/village obstacle.
fn spawn_physics_box(
    world: &mut World,
    translation: Vec3,
    yaw: f32,
    scale: Vec3,
    half_extents: Vec3,
    body: Option<RigidBody>,
    sensor: bool,
) -> ribbon_ecs::Entity {
    let transform = Transform3D {
        translation,
        rotation: Quat::from_rotation_y(yaw),
        scale,
    };
    let mut collider = Collider::cuboid(half_extents);
    collider.sensor = sensor;
    match body {
        Some(body) => world.spawn((transform, body, collider)),
        None => world.spawn((transform, collider)),
    }
}

fn spawn_static_room_colliders(
    world: &mut World,
    room: &RoomDefinition,
    tracked_entities: &mut Vec<ribbon_ecs::Entity>,
) {
    for collider in &room.static_colliders {
        let physics_entity = spawn_physics_box(
            world,
            Vec3::from_array(collider.center),
            0.0,
            Vec3::ONE,
            Vec3::from_array(collider.half_extents),
            None,
            false,
        );
        tracked_entities.push(physics_entity);
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut frame_limit = None;
    let mut profile_csv: Option<String> = None;
    let mut profile_warmup = 120_u64;
    let mut assets_override: Option<PathBuf> = None;
    let mut validate_assets_only = false;
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
            "--profile-csv" => {
                profile_csv = Some(args.next().ok_or("--profile-csv expects a path")?);
            }
            "--profile-warmup" => {
                profile_warmup = args
                    .next()
                    .ok_or("--profile-warmup expects a number")?
                    .parse::<u64>()?;
            }
            "--assets" => {
                assets_override = Some(PathBuf::from(
                    args.next().ok_or("--assets expects a directory")?,
                ));
            }
            "--validate-assets" => validate_assets_only = true,
            "--help" | "-h" => {
                println!(concat!(
                    "Ribbon Horror Slice\n\n",
                    "Controls:\n",
                    "  WASD       Walk\n",
                    "  Mouse      Look around\n",
                    "  Shift      Walk faster\n",
                    "  E          Pick up the brass key / unlock the exit\n",
                    "  Escape     Release/capture pointer\n",
                    "  Left click Capture pointer again after releasing it\n\n",
                    "Options:\n",
                    "  --frames N Exit after N frames\n",
                    "  --profile-csv PATH Write a bounded post-warmup render profile\n",
                    "  --profile-warmup N Frames excluded before capture\n",
                    "  --assets PATH      Use this packaged asset directory\n",
                    "  --validate-assets  Validate runtime assets, then exit\n",
                    "  -h, --help Show this help\n"
                ));
                return Ok(());
            }
            other => return Err(format!("unknown argument `{other}`").into()),
        }
    }

    let asset_root = resolve_asset_root(assets_override)?;
    println!("[ATHAR] runtime asset root: {}", asset_root.display());
    let room = load_room(&asset_root)?;
    let model_asset_count = validate_asset_root(&asset_root, &room)?;
    if validate_assets_only {
        println!(
            "[ATHAR] package validation passed: {} room entities, {} unique room/village models, {} audio assets",
            room.entities.len(),
            model_asset_count,
            crate::audio::ATHAR_AUDIO_ASSETS.len()
        );
        return Ok(());
    }
    let streaming_profile_enabled = profile_csv.is_some();
    let mut app = App::new();
    app.insert_resource(PlatformConfig {
        title: "Ribbon | Horror Slice".to_owned(),
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
    app.insert_resource(WindowTextOverlay {
        lines: vec![
            "HORROR SLICE - THE LOST KEY".to_owned(),
            "WASD MOVE | MOUSE LOOK | SHIFT SPRINT".to_owned(),
            "E INTERACT | ESC RELEASE / CLICK RECAPTURE".to_owned(),
            "OBJECTIVE: FIND THE BRASS KEY".to_owned(),
        ],
        visible: true,
    });

    // Native window / scene / asset / mesh rendering path; no offscreen readback bridge.
    app.add_plugin(PlatformPlugin)?;
    app.add_plugin(ScenePlugin)?;
    // These producers must be registered before PhysicsPlugin so intent is consumed in the same
    // engine fixed step and mouse-look is current before physics reconciliation.
    app.add_system(Stage::PreUpdate, first_person_controller);
    app.add_system(Stage::FixedUpdate, submit_character_movement);
    app.add_plugin(PhysicsPlugin)?;
    app.add_plugin(AssetPlugin)?;
    app.add_plugin(AudioPlugin)?;
    app.world
        .insert_resource(audio::GameplayAudioState::default());
    audio::initialize_output(&mut app.world);
    app.add_plugin(GpuWindowPlugin)?;
    app.add_plugin(Render3dPlugin)?;
    app.add_plugin(StreamingPlugin::new(StreamingConfig {
        max_new_region_requests_per_update: 4,
        max_region_activations_per_update: 1,
        max_entity_spawns_per_update: 256,
        max_active_regions: 8,
        profile_cpu: streaming_profile_enabled,
        ..StreamingConfig::default()
    }))?;
    app.world
        .resource_mut::<GpuWindow>()
        .add_pass(Box::new(WindowTextOverlayPass::default()));
    if let Some(path) = profile_csv {
        app.add_plugin(RenderProfilePlugin::new(path).warmup_frames(profile_warmup))?;
    }

    let mut requested_assets: Vec<String> = room
        .entities
        .iter()
        .map(|entity| entity.asset.clone())
        .collect();
    requested_assets.sort();
    requested_assets.dedup();
    let (handles, leases) = {
        let server = app.world.resource_mut::<AssetServer>();
        server.register_loader(GltfLoader);
        server.add_source(0, FsSource::new(asset_root.join("models")));
        let mut map = HashMap::new();
        let mut leases = HashMap::new();
        for asset in requested_assets {
            let lease = server
                .request_ref::<ModelAsset>(&asset)
                .map_err(|error| format!("could not queue `{asset}`: {}", error.message))?;
            map.insert(asset.clone(), lease.handle());
            leases.insert(asset, lease);
        }
        (map, leases)
    };
    app.insert_resource(GameModelLeases { _leases: leases });
    app.insert_resource(GameModelHandles(handles.clone()));
    app.insert_resource(AssetEventCursor(EventCursor::default()));

    let spawn = Vec3::from_array(room.player_spawn);
    let camera_rotation =
        Quat::from_rotation_y(-room.player_yaw) * Quat::from_rotation_x(-0.12_f32);
    let camera_entity = app.world.spawn((
        Transform3D {
            translation: spawn,
            rotation: camera_rotation,
            ..Transform3D::default()
        },
        Camera3D {
            fov_y: 68.0_f32.to_radians(),
            near: 0.04,
            far: 100.0,
        },
        ribbon_plugin_audio::SpatialAudioListener::default(),
    ));
    app.world.spawn((
        Transform3D {
            translation: Vec3::new(0.0, 1.5, 0.0),
            ..Transform3D::default()
        },
        AudioZoneVolume {
            id: 1,
            priority: 10,
            half_extents: AudioVec3::new(6.0, 7.0, 3.0),
            blend_distance: 0.8,
            acoustics: AcousticZone {
                id: 1,
                wet_mix: 0.24,
                low_pass_hz: 17_500.0,
                reverb_seconds: 1.15,
                ambience_gain: 0.88,
            },
        },
    ));
    // Rapier's capsule is centered on the body; the rendered camera sits at a fixed eye offset.
    let player_body_entity = app.world.spawn((
        Transform3D {
            translation: FirstPersonController::player_body_translation(spawn),
            ..Transform3D::default()
        },
        RigidBody::kinematic_position_based(),
        Collider::capsule_y(PLAYER_HALF_SEGMENT, PLAYER_RADIUS),
        CharacterController::default(),
        CharacterMovementIntent::default(),
    ));
    app.world
        .resource_mut::<FirstPersonController>()
        .player_entity = Some(player_body_entity);

    // ATHAR is a consumer of the generic authored-region coordinator. The engine plugin owns
    // AssetServer leases, GPU warmup references, collider activation, and streamed entity cleanup.
    let village_layout = scripts::village::VillageLayout::deterministic();
    let village_region_id = RegionId::new(scripts::village::VILLAGE_REGION_ID);
    let village_region = RegionDefinition::new(
        village_region_id.clone(),
        "Wet Creek village",
        ribbon_plugin_streaming::RegionBounds::new(
            Vec3::new(-132.0, -6.0, -132.0),
            Vec3::new(132.0, 16.0, 132.0),
        )
        .map_err(|message| std::io::Error::new(std::io::ErrorKind::InvalidInput, message))?,
    )
    .with_policy(RegionPolicy {
        preload_distance: 260.0,
        activation_distance: 12.0,
        unload_distance: 360.0,
        base_priority: 100,
        auto_activate: false,
        preview_when_ready: true,
    });
    let village_region = village_layout
        .streaming_models()
        .into_iter()
        .fold(village_region, |region, model| region.with_model(model));
    let village_region = village_layout
        .streaming_colliders()
        .into_iter()
        .fold(village_region, |region, collider| {
            region.with_collider(collider)
        });
    let village_region = village_layout
        .streaming_point_lights()
        .into_iter()
        .fold(village_region, |region, light| {
            region.with_point_light(light)
        });
    {
        let coordinator = app.world.resource_mut::<StreamingCoordinator>();
        coordinator.register_region(village_region)?;
        coordinator.register_scene(SceneDefinition::new(
            SceneId::new(scripts::village::VILLAGE_SCENE_ID),
            "ATHAR Village",
            [village_region_id.clone()],
        ))?;
        coordinator.add_observer(StreamObserver::new(1, spawn))?;
        coordinator.request_scene(&SceneId::new(scripts::village::VILLAGE_SCENE_ID))?;
    }

    let mut room_entities = Vec::new();
    let mut key_entity = None;
    let mut door_entity = None;
    let mut door_collider_entity = None;
    let mut door_collider_half_height = 1.2;
    let mut key_position = Vec3::new(0.85, 0.025, 1.55);
    let mut door_position = Vec3::new(5.84, 0.0, -2.0);
    for entity in &room.entities {
        let handle = *handles
            .get(&entity.asset)
            .ok_or("room entity asset handle missing")?;
        let spawned = app.world.spawn((
            Transform3D {
                translation: Vec3::from_array(entity.position),
                rotation: Quat::from_rotation_y(entity.yaw),
                scale: Vec3::from_array(entity.scale),
            },
            ModelInstance { model: handle },
        ));
        room_entities.push(spawned);
        match entity.kind.as_str() {
            "Key" => {
                key_entity = Some(spawned);
                key_position = Vec3::from_array(entity.position);
                // A key is a trigger, not a blocking object. The current interaction prompt and
                // pickup remain proximity-based; the sensor also exposes overlap events.
                if let Some(half_extents) = entity.collider_half_extents {
                    let mut sensor = Collider::cuboid(Vec3::from_array(half_extents));
                    sensor.sensor = true;
                    app.world
                        .insert(spawned, sensor)
                        .expect("key entity still exists");
                }
            }
            "Door" => {
                door_entity = Some(spawned);
                door_position = Vec3::from_array(entity.position);
                if let Some(half_extents) = entity.collider_half_extents {
                    let position = Vec3::from_array(entity.position);
                    let scale = Vec3::from_array(entity.scale);
                    door_collider_half_height = half_extents[1] * scale.y;
                    let collider_entity = spawn_physics_box(
                        &mut app.world,
                        position + Vec3::Y * door_collider_half_height,
                        entity.yaw,
                        scale,
                        Vec3::from_array(half_extents),
                        // This door is a scripted obstacle, not a body that must push dynamic
                        // actors. A fixed proxy is synchronized immediately during PreUpdate, so
                        // ATHAR's character shape-cast sees the animated door pose in the same
                        // fixed step instead of the previous kinematic pose.
                        None,
                        false,
                    );
                    room_entities.push(collider_entity);
                    door_collider_entity = Some(collider_entity);
                }
            }
            "StaticRoomGeometry" => {}
            _ => {
                // The save-point prop and static monster silhouette keep the authored physical
                // dimensions but use engine-owned collider proxies centered at their feet offset.
                if let Some(half_extents) = entity.collider_half_extents {
                    let position = Vec3::from_array(entity.position);
                    let scale = Vec3::from_array(entity.scale);
                    let collider_entity = spawn_physics_box(
                        &mut app.world,
                        position + Vec3::Y * (half_extents[1] * scale.y),
                        entity.yaw,
                        scale,
                        Vec3::from_array(half_extents),
                        None,
                        false,
                    );
                    room_entities.push(collider_entity);
                }
            }
        }
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

    // Authored wall/floor boxes are registered as fixed Rapier colliders. The room shell GLB
    // remains visual-only and does not stand in for a triangle-mesh collision shape.
    spawn_static_room_colliders(&mut app.world, &room, &mut room_entities);

    // This reusable spotlight follows the camera pose every frame, including mouse look.
    let direction = (camera_rotation * Vec3::NEG_Z).normalize_or_zero();
    let right = camera_rotation * Vec3::X;
    let up = camera_rotation * Vec3::Y;
    // Keep the player flashlight through the exit transition; other room lights are scoped to
    // the room and removed when the village loads.
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
    let save_light = app.world.spawn((PointLight {
        position: save + Vec3::new(0.0, 1.05, 0.0),
        color: Vec3::new(0.66, 0.92, 0.72),
        intensity: 24.0,
        range: 6.0,
    },));
    let key_light = app.world.spawn((PointLight {
        position: key_position + Vec3::new(0.0, 0.42, 0.0),
        color: Vec3::new(1.0, 0.62, 0.18),
        intensity: 8.0,
        range: 3.4,
    },));
    app.insert_resource(AdventureState::new(
        camera_entity,
        player_body_entity,
        room_entities,
        vec![save_light],
        key_entity,
        Some(key_light),
        door_entity,
        door_collider_entity,
        door_collider_half_height,
        key_position,
        door_position,
        Vec3::from_array(room.save_point),
    ));

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
    println!(concat!(
        "Controls: WASD move, mouse look, Shift sprint, E interact, ",
        "Esc releases pointer; left-click recaptures."
    ));
    app.add_system(Stage::Update, sync_player_camera);
    app.add_system(Stage::Update, scripts::gameplay::gameplay_update);
    // Keep the weather update after gameplay so a room-to-village transition activates rain in
    // this frame, while the plugin remains responsible for particle state and recycling.
    app.add_plugin(ribbon_plugin_weather::WeatherPlugin)?;
    app.add_system(Stage::Update, report_asset_failures);
    app.add_system(Stage::Update, enforce_frame_limit);
    app.build()?.run();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{load_room, Vec3};
    use std::path::Path;

    #[test]
    fn explicit_asset_root_is_accepted_independently_of_working_directory() {
        use super::resolve_asset_root;
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("ribbon-athar-assets-{unique}"));
        std::fs::create_dir_all(root.join("models")).expect("create models directory");
        std::fs::write(root.join("room.ron"), "").expect("create room marker");
        let resolved = resolve_asset_root(Some(root.clone())).expect("explicit root resolves");
        assert_eq!(resolved, root.canonicalize().expect("canonical temp path"));
        std::fs::remove_dir_all(root).expect("remove temp assets");
    }

    #[test]
    fn room_static_geometry_is_registered_in_rapier_and_raycastable() {
        use super::spawn_static_room_colliders;
        use ribbon_app::{App, Time};
        use ribbon_plugin_physics::{
            raycast, Collider, PhysicsDebugData, PhysicsPlugin, RaycastQuery,
        };
        use ribbon_plugins::AppPluginExt;
        use ribbon_scene::ScenePlugin;
        use std::time::Duration;

        let asset_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
        let room = super::load_room(&asset_root).expect("room manifest parses");
        let mut app = App::new();
        app.add_plugin(ScenePlugin).expect("scene plugin");
        app.add_plugin(PhysicsPlugin).expect("physics plugin");
        let mut tracked = Vec::new();
        spawn_static_room_colliders(&mut app.world, &room, &mut tracked);

        assert_eq!(tracked.len(), room.static_colliders.len());
        let collider_count = app.world.query::<&Collider>().iter().count();
        assert_eq!(collider_count, room.static_colliders.len());
        assert!(tracked
            .iter()
            .all(|entity| app.world.get::<Collider>(*entity).is_some()));

        app.world.resource_mut::<Time>().delta = Duration::from_secs_f64(1.0 / 60.0);
        app.update();
        assert_eq!(
            app.world.resource::<PhysicsDebugData>().colliders.len(),
            room.static_colliders.len(),
            "all authored room boxes must be reconciled into the Rapier backend"
        );
        let hit = raycast(
            &app.world,
            RaycastQuery::new(Vec3::new(0.0, 1.0, 0.0), -Vec3::Y, 2.0),
        )
        .expect("room floor must be queryable by Rapier");
        assert_eq!(hit.entity, tracked[0]);
        assert!(hit.position.y.abs() < 0.05, "unexpected floor hit: {hit:?}");
    }

    #[test]
    fn room_definition_contains_a_key_door_and_existing_models() {
        let asset_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
        let room = load_room(&asset_root).expect("the packaged room definition must parse");
        assert_eq!(room.version, 1);
        assert!(room.entities.iter().any(|entity| entity.kind == "Key"));
        assert!(room.entities.iter().any(|entity| entity.kind == "Door"));
        assert!(room
            .entities
            .iter()
            .any(|entity| entity.kind == "StaticRoomGeometry"));

        let model_root = asset_root.join("models");
        for entity in &room.entities {
            assert!(
                model_root.join(&entity.asset).is_file(),
                "room entity `{}` references missing model `{}`",
                entity.name,
                entity.asset
            );
        }

        let collider_names: Vec<_> = room
            .static_colliders
            .iter()
            .map(|collider| collider.name.as_str())
            .collect();
        assert!(collider_names.contains(&"right_wall_front"));
        assert!(collider_names.contains(&"right_wall_back"));
        assert!(collider_names.contains(&"right_wall_over_door"));
        assert!(collider_names.contains(&"floor"));
    }
}
