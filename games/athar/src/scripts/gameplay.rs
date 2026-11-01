//! Key pickup, locked-door interaction, door animation, and room-to-village transition.

use super::player::{FirstPersonController, PLAYER_CAMERA_OFFSET};
use super::village;
use super::{AdventureState, GamePhase, VillageTransitionState};
use ribbon_app::{Time, World};
use ribbon_platform::input::Key;
use ribbon_platform::Input;
use ribbon_plugin_render3d::glam::{Quat, Vec3};
use ribbon_plugin_render3d::{Camera3D, FogSettings, SunLight, Transform3D};
use ribbon_plugin_streaming::{
    RegionId, RegionState, SceneId, SceneState, StreamObserver, StreamingCoordinator,
};
use ribbon_render::gpu_window::{ClearColor, WindowTextOverlay};
use std::time::Instant;

const KEY_INTERACT_RADIUS: f32 = 1.55;
const DOOR_INTERACT_RADIUS: f32 = 2.45;
const DOOR_ANIMATION_SECONDS: f32 = 0.88;
const DOOR_CLEAR_PROGRESS: f32 = 0.86;
// Stage the already-warmed renderables before the door reaches its collision-clear angle so the
// first fully open frame has real exterior geometry to draw.
const VILLAGE_PREVIEW_DOOR_PROGRESS: f32 = 0.70;
const EXIT_OUTER_X: f32 = 6.12;
const EXIT_CENTER_Z: f32 = -2.0;
const EXIT_HALF_WIDTH: f32 = 0.82;
const DOOR_HINGE: Vec3 = Vec3::new(5.84, 0.0, -2.96);
const DOOR_HINGE_TO_CENTER: Vec3 = Vec3::new(0.0, 0.0, 0.96);

fn horizontal_distance_squared(a: Vec3, b: Vec3) -> f32 {
    let delta = Vec3::new(a.x - b.x, 0.0, a.z - b.z);
    delta.length_squared()
}

fn near(a: Vec3, b: Vec3, radius: f32) -> bool {
    horizontal_distance_squared(a, b) <= radius * radius
}

fn can_enter_village(
    phase: GamePhase,
    door_unlocked: bool,
    door_progress: f32,
    player: Vec3,
) -> bool {
    phase == GamePhase::Room
        && door_unlocked
        && door_progress >= DOOR_CLEAR_PROGRESS
        && player.x > EXIT_OUTER_X
        && (player.z - EXIT_CENTER_Z).abs() < EXIT_HALF_WIDTH
}

fn village_transition_can_commit(phase: GamePhase, state: &VillageTransitionState) -> bool {
    phase == GamePhase::Room && matches!(state, VillageTransitionState::VillageReady)
}

fn next_door_open_progress(progress: f32, unlocked: bool, dt: f32) -> f32 {
    let progress = if progress.is_finite() {
        progress.clamp(0.0, 1.0)
    } else {
        0.0
    };
    if !unlocked || progress >= 1.0 {
        return progress;
    }
    let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
    (progress + dt / DOOR_ANIMATION_SECONDS).clamp(0.0, 1.0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RoomInteraction {
    PickUpKey,
    LockedDoor,
    UnlockDoor,
    DoorAlreadyUnlocked,
    NothingNearby,
}

fn choose_room_interaction(
    player: Vec3,
    key_collected: bool,
    door_unlocked: bool,
    key_position: Vec3,
    door_position: Vec3,
) -> RoomInteraction {
    if !key_collected && near(player, key_position, KEY_INTERACT_RADIUS) {
        RoomInteraction::PickUpKey
    } else if near(player, door_position, DOOR_INTERACT_RADIUS) {
        if !key_collected {
            RoomInteraction::LockedDoor
        } else if door_unlocked {
            RoomInteraction::DoorAlreadyUnlocked
        } else {
            RoomInteraction::UnlockDoor
        }
    } else {
        RoomInteraction::NothingNearby
    }
}

fn update_key_and_door_animation(world: &mut World, state: &mut AdventureState, dt: f32) {
    if !state.key_collected {
        if let Some(key) = state.key_entity {
            if let Some(mut transform) = world.get_mut::<Transform3D>(key) {
                transform.translation.y = 0.025 + (state.elapsed * 2.4).sin() * 0.045;
                transform.rotation = Quat::from_rotation_y(-0.32 + state.elapsed * 0.52);
            }
        }
    }

    let previous_progress = state.door_open_progress;
    state.door_open_progress = next_door_open_progress(previous_progress, state.door_unlocked, dt);
    if state.door_open_progress > previous_progress {
        let door_pose = if let Some(door) = state.door_entity {
            if let Some(mut transform) = world.get_mut::<Transform3D>(door) {
                let angle = state.door_open_progress * std::f32::consts::FRAC_PI_2;
                transform.translation =
                    DOOR_HINGE + Quat::from_rotation_y(angle) * DOOR_HINGE_TO_CENTER;
                transform.rotation = Quat::from_rotation_y(angle);
                Some((transform.translation, transform.rotation))
            } else {
                None
            }
        } else {
            None
        };
        // Keep the Rapier fixed proxy aligned with the animated door mesh until the doorway is
        // clear. Fixed proxies take ECS pose edits immediately in PreUpdate, before the player
        // character's shape-cast query runs. Its offset comes from the authored half-height.
        if let (Some((door_translation, door_rotation)), Some(collider_entity)) =
            (door_pose, state.door_collider_entity)
        {
            if let Some(mut collider_transform) = world.get_mut::<Transform3D>(collider_entity) {
                collider_transform.translation =
                    door_translation + Vec3::Y * state.door_collider_half_height;
                collider_transform.rotation = door_rotation;
            }
        }
    }

    if state.door_open_progress >= DOOR_CLEAR_PROGRESS && !state.door_collision_removed {
        // Despawning the collider proxy lets PhysicsPlugin reconcile and remove its Rapier
        // backend body on the next PreUpdate; no parallel legacy collision list is maintained.
        if let Some(collider_entity) = state.door_collider_entity.take() {
            let _ = world.despawn(collider_entity);
        }
        state.door_collision_removed = true;
        state.set_message("The door is open. Walk through the doorway.", 5.0);
    }
}

fn village_door_view_is_revealed(phase: GamePhase, door_progress: f32) -> bool {
    phase == GamePhase::Room && door_progress >= VILLAGE_PREVIEW_DOOR_PROGRESS
}

fn village_preview_is_resident(
    state: &RegionState,
    spawned_visual_entities: usize,
    required_visual_entities: usize,
) -> bool {
    matches!(
        state,
        RegionState::Ready
            | RegionState::PreparingPhysics
            | RegionState::Activating
            | RegionState::Active
    ) && spawned_visual_entities >= required_visual_entities
}

/// Reveal the outdoor backdrop as the door opens. Render-only previews are spawned by the
/// generic coordinator after CPU/GPU readiness; the gameplay layer only observes that state.
fn update_village_door_view(world: &mut World, state: &mut AdventureState) {
    if !village_door_view_is_revealed(state.phase, state.door_open_progress) {
        return;
    }

    const OUTDOOR_CLEAR_COLOR: [f64; 4] = [0.24, 0.29, 0.34, 1.0];
    if world.resource::<ClearColor>().0 != OUTDOOR_CLEAR_COLOR {
        world.resource_mut::<ClearColor>().0 = OUTDOOR_CLEAR_COLOR;
    }

    let region_id = RegionId::new(village::VILLAGE_REGION_ID);
    let preview_ready = {
        let coordinator = world.resource::<StreamingCoordinator>();
        let snapshot = coordinator.snapshot();
        let region = snapshot
            .regions
            .iter()
            .find(|region| region.id == region_id);
        region.is_some_and(|region| {
            let lifecycle = coordinator.region_state(&region_id);
            lifecycle.is_some_and(|lifecycle| {
                village_preview_is_resident(
                    lifecycle,
                    region.visual_entity_count,
                    region.model_count + region.point_light_count,
                )
            })
        })
    };
    if preview_ready && !state.village_preview_lighting_active {
        apply_village_lighting(world, state.camera_entity, false);
        state.village_preview_lighting_active = true;
    }
}

/// Apply the shared exterior environment. `commit_camera` changes the indoor camera's vertical FOV
/// for the full village; preview mode only extends its far plane so the view through the doorway
/// reveals the staged village without changing player pose.
fn apply_village_lighting(
    world: &mut World,
    camera_entity: ribbon_ecs::Entity,
    commit_camera: bool,
) {
    *world.resource_mut::<SunLight>() = SunLight {
        direction: Vec3::new(-0.35, -0.82, -0.44).normalize(),
        color: Vec3::new(0.38, 0.43, 0.49),
        ambient: Vec3::splat(0.27),
    };
    *world.resource_mut::<FogSettings>() = FogSettings {
        color: Vec3::new(0.27, 0.32, 0.36),
        density: 0.0021,
        base_height: 0.0,
        height_falloff: 0.018,
    };
    if let Some(mut camera) = world.get_mut::<Camera3D>(camera_entity) {
        camera.far = 260.0;
        if commit_camera {
            camera.fov_y = 72.0_f32.to_radians();
        }
    }
}

fn update_village_preparation(world: &mut World, state: &mut AdventureState) {
    if state.phase != GamePhase::Room
        || matches!(
            &state.transition_state,
            VillageTransitionState::CommittingTransition
                | VillageTransitionState::VillageActive
                | VillageTransitionState::TransitionFailed { .. }
        )
    {
        return;
    }

    let region_id = RegionId::new(village::VILLAGE_REGION_ID);
    let snapshot = world.resource::<StreamingCoordinator>().snapshot();
    let region = snapshot
        .regions
        .iter()
        .find(|region| region.id == region_id);
    let Some(region) = region else {
        state.transition_state = VillageTransitionState::TransitionFailed {
            asset: "<village-region>".to_owned(),
            reason: "the ATHAR village region is not registered with StreamingPlugin".to_owned(),
        };
        state.set_message("Village streaming region is unavailable.", 30.0);
        if world.try_resource::<FirstPersonController>().is_some() {
            world.resource_mut::<FirstPersonController>().exit_blocked = true;
        }
        return;
    };

    state.village_ready_models = region.gpu_ready_models;
    if let RegionState::Failed(reason) = &region.state {
        let reason = reason.clone();
        state.transition_state = VillageTransitionState::TransitionFailed {
            asset: "<village-region>".to_owned(),
            reason: reason.clone(),
        };
        state.transition_preparation_ms = Some(
            state
                .transition_preparation_started
                .elapsed()
                .as_millis()
                .min(u128::from(u64::MAX)) as u64,
        );
        state.set_message(
            format!("Village streaming failed: {reason}. Press E near the open exit to retry."),
            30.0,
        );
        if world.try_resource::<FirstPersonController>().is_some() {
            world.resource_mut::<FirstPersonController>().exit_blocked = true;
        }
        eprintln!("[ATHAR streaming] village region failed: {reason}");
        return;
    }

    let all_models_ready = region.gpu_ready_models == village::VILLAGE_MODEL_ASSETS.len();
    let preview_ready = village_preview_is_resident(
        &region.state,
        region.visual_entity_count,
        region.model_count + region.point_light_count,
    );
    if matches!(&region.state, RegionState::Ready) && all_models_ready && preview_ready {
        if !matches!(
            &state.transition_state,
            VillageTransitionState::VillageReady
        ) {
            state.transition_state = VillageTransitionState::VillageReady;
            state.transition_preparation_ms = Some(
                state
                    .transition_preparation_started
                    .elapsed()
                    .as_millis()
                    .min(u128::from(u64::MAX)) as u64,
            );
            state.set_message("Village prepared. The open exit is safe to enter.", 5.0);
            eprintln!(
                "[ATHAR streaming] village prepared: {} models GPU-ready, {} preview entities, {} ms",
                region.gpu_ready_models,
                region.visual_entity_count,
                state.transition_preparation_ms.unwrap_or_default(),
            );
        }
        if world.try_resource::<FirstPersonController>().is_some() {
            world.resource_mut::<FirstPersonController>().exit_blocked = false;
        }
    } else {
        if matches!(
            &state.transition_state,
            VillageTransitionState::VillageReady
        ) {
            state.transition_state = VillageTransitionState::PreparingVillage;
            state.transition_preparation_started = Instant::now();
            state.transition_preparation_ms = None;
        }
        if world.try_resource::<FirstPersonController>().is_some() {
            world.resource_mut::<FirstPersonController>().exit_blocked = true;
        }
    }
}

fn retry_failed_village_preparation(
    world: &mut World,
    state: &mut AdventureState,
    _failed_asset: &str,
) -> Result<(), String> {
    world
        .resource_mut::<StreamingCoordinator>()
        .retry_scene(&SceneId::new(village::VILLAGE_SCENE_ID))?;
    state.village_preview_lighting_active = false;
    state.transition_state = VillageTransitionState::PreparingVillage;
    state.village_ready_models = 0;
    state.transition_preparation_started = Instant::now();
    state.transition_preparation_ms = None;
    state.set_message(
        "Retrying village streaming in the background. Stay inside until ready.",
        6.0,
    );
    if world.try_resource::<FirstPersonController>().is_some() {
        world.resource_mut::<FirstPersonController>().exit_blocked = true;
    }
    Ok(())
}

fn begin_village_activation(world: &mut World, state: &mut AdventureState) -> Result<(), String> {
    if !village_transition_can_commit(state.phase, &state.transition_state) {
        return Err("village activation requested before CPU/GPU preparation completed".to_owned());
    }
    world
        .resource_mut::<StreamingCoordinator>()
        .activate_scene(&SceneId::new(village::VILLAGE_SCENE_ID))?;
    state.transition_state = VillageTransitionState::CommittingTransition;
    state.set_message("Preparing village collision and scene activation...", 5.0);
    if world.try_resource::<FirstPersonController>().is_some() {
        world.resource_mut::<FirstPersonController>().exit_blocked = true;
    }
    Ok(())
}

fn finish_village_activation_if_ready(world: &mut World, state: &mut AdventureState) {
    if state.phase != GamePhase::Room
        || !matches!(
            &state.transition_state,
            VillageTransitionState::CommittingTransition
        )
    {
        return;
    }
    let scene_id = SceneId::new(village::VILLAGE_SCENE_ID);
    match world
        .resource::<StreamingCoordinator>()
        .scene_state(&scene_id)
    {
        Some(SceneState::Active) => {
            if let Err(error) = enter_village(world, state) {
                state.transition_state = VillageTransitionState::TransitionFailed {
                    asset: "<village-activation>".to_owned(),
                    reason: error.clone(),
                };
                state.set_message(format!("Village activation failed: {error}"), 30.0);
                if world.try_resource::<FirstPersonController>().is_some() {
                    world.resource_mut::<FirstPersonController>().exit_blocked = true;
                }
            }
        }
        Some(SceneState::Failed(failed_regions)) => {
            let coordinator = world.resource::<StreamingCoordinator>();
            let reasons = failed_regions
                .iter()
                .map(|region_id| {
                    let reason = match coordinator.region_state(region_id) {
                        Some(RegionState::Failed(reason)) => reason.as_str(),
                        _ => "region failed without a diagnostic reason",
                    };
                    format!("{}: {reason}", region_id.as_str())
                })
                .collect::<Vec<_>>()
                .join("; ");
            state.transition_state = VillageTransitionState::TransitionFailed {
                asset: "<village-activation>".to_owned(),
                reason: reasons.clone(),
            };
            state.set_message(
                format!("Village scene activation failed: {reasons}. Press E to retry."),
                30.0,
            );
        }
        _ => {}
    }
}

fn enter_village(world: &mut World, state: &mut AdventureState) -> Result<(), String> {
    let region_id = RegionId::new(village::VILLAGE_REGION_ID);
    let scene_id = SceneId::new(village::VILLAGE_SCENE_ID);
    let coordinator = world.resource::<StreamingCoordinator>();
    if coordinator.scene_state(&scene_id) != Some(SceneState::Active)
        || !coordinator
            .region_state(&region_id)
            .is_some_and(|state| matches!(state, RegionState::Active))
    {
        return Err("streaming coordinator has not fully prepared and activated every village scene region and registered its required colliders".to_owned());
    }
    let snapshot = world.resource::<StreamingCoordinator>().snapshot();
    let region = snapshot
        .regions
        .iter()
        .find(|region| region.id == region_id)
        .ok_or_else(|| "village region diagnostics disappeared before handoff".to_owned())?;
    let model_count = region.model_count;
    let collider_count = region.collider_count;
    let visual_count = region.visual_entity_count;

    // All model/light entities were prepared by StreamingPlugin, and all required colliders were
    // confirmed by PhysicsPlugin before RegionState::Active. Keep the old room until this point.
    let camera_position = Vec3::new(8.2, 1.65, -2.0);
    let yaw = std::f32::consts::FRAC_PI_2;
    if let Some(mut transform) = world.get_mut::<Transform3D>(state.camera_entity) {
        transform.translation = camera_position;
        transform.rotation = Quat::from_rotation_y(-yaw) * Quat::from_rotation_x(-0.035);
    }
    if let Some(mut transform) = world.get_mut::<Transform3D>(state.player_body_entity) {
        transform.translation = camera_position - Vec3::Y * PLAYER_CAMERA_OFFSET;
        transform.rotation = Quat::IDENTITY;
    }
    if world.try_resource::<FirstPersonController>().is_some() {
        let controller = world.resource_mut::<FirstPersonController>();
        controller.yaw = yaw;
        controller.pitch = -0.035;
        controller.skip_mouse_delta = true;
        controller.cursor_initialized = true;
        controller.exit_blocked = false;
    }
    world.resource_mut::<ClearColor>().0 = [0.24, 0.29, 0.34, 1.0];
    apply_village_lighting(world, state.camera_entity, true);
    state.village_preview_lighting_active = true;
    world
        .resource_mut::<ribbon_plugin_weather::RainSettings>()
        .enabled = true;

    for entity in state.room_entities.drain(..) {
        let _ = world.despawn(entity);
    }
    world
        .resource_mut::<crate::GameModelLeases>()
        ._leases
        .clear();
    // Move the observer at the same time as the camera. StreamingPlugin runs before gameplay
    // systems on the next frame; publishing this immediately prevents the freshly activated
    // region from being considered outside its unload range for one frame.
    let _ = world
        .resource_mut::<StreamingCoordinator>()
        .update_observer(StreamObserver::new(1, camera_position));
    world
        .resource_mut::<StreamingCoordinator>()
        .release_scene_request(&SceneId::new(village::VILLAGE_SCENE_ID))?;
    for entity in state.room_lights.drain(..) {
        let _ = world.despawn(entity);
    }
    if let Some(key_light) = state.key_light.take() {
        let _ = world.despawn(key_light);
    }
    state.key_entity = None;
    state.door_entity = None;
    state.door_collider_entity = None;
    state.village_first_physics_profile_logged = false;
    state.village_transform_profile_logged = false;
    state.village_rain_profile_logged = false;
    if let Some(diagnostics) = world.try_resource::<ribbon_scene::TransformPropagationDiagnostics>()
    {
        let _ = diagnostics;
        let diagnostics = world.resource_mut::<ribbon_scene::TransformPropagationDiagnostics>();
        diagnostics.capture_next_3d = true;
        diagnostics.last_3d_duration_ns = 0;
        diagnostics.sample_pending = false;
    }
    if world
        .try_resource::<ribbon_plugin_physics::PhysicsDiagnostics>()
        .is_some()
    {
        let diagnostics = world.resource_mut::<ribbon_plugin_physics::PhysicsDiagnostics>();
        diagnostics.timing_enabled = true;
        diagnostics.last_reconcile_time_ns = 0;
        diagnostics.last_fixed_step_time_ns = 0;
    }
    state.phase = GamePhase::Village;
    state.transition_state = VillageTransitionState::VillageActive;
    state.set_message(
        "Welcome to Wet Creek. The village road disappears into the rain.",
        8.0,
    );
    // Release the room's strong CPU leases after its entities are removed. Village leases remain
    // owned by the region until it leaves every observer's unload range.
    eprintln!(
        "[ATHAR streaming] region activated: {model_count} model placements, {collider_count} required colliders, {visual_count} render entities"
    );
    Ok(())
}

/// Consume one E press per interaction; the engine input snapshot provides the edge-trigger.
pub fn gameplay_update(world: &mut World) {
    let dt = world
        .resource::<Time>()
        .delta
        .as_secs_f32()
        .clamp(0.0, 0.05);
    let interact_pressed = world.resource::<Input>().just_pressed(Key::E);
    let camera_entity = world.resource::<AdventureState>().camera_entity;
    let player_position = world
        .get::<Transform3D>(camera_entity)
        .map(|transform| transform.translation);
    if let Some(position) = player_position {
        let velocity = world
            .resource::<StreamingCoordinator>()
            .observer(1)
            .map(|previous| {
                if dt > f32::EPSILON {
                    (position - previous.position) / dt
                } else {
                    Vec3::ZERO
                }
            })
            .unwrap_or(Vec3::ZERO);
        let _ = world
            .resource_mut::<StreamingCoordinator>()
            .update_observer(StreamObserver::new(1, position).with_velocity(velocity));
    }

    world.resource_scope::<AdventureState, _>(|world, state| {
        state.elapsed += dt;
        state.message_seconds = (state.message_seconds - dt).max(0.0);
        if state.phase == GamePhase::Village && !state.village_transform_profile_logged {
            let transform_sample = world
                .try_resource::<ribbon_scene::TransformPropagationDiagnostics>()
                .filter(|diagnostics| diagnostics.sample_pending)
                .map(|diagnostics| diagnostics.last_3d_duration_ns);
            if let Some(duration_ns) = transform_sample {
                eprintln!(
                    "[ATHAR transition] first village 3D transform propagation: {} us",
                    duration_ns / 1_000,
                );
                let diagnostics =
                    world.resource_mut::<ribbon_scene::TransformPropagationDiagnostics>();
                diagnostics.sample_pending = false;
                diagnostics.last_3d_duration_ns = 0;
                state.village_transform_profile_logged = true;
            }
        }
        if state.phase == GamePhase::Village && !state.village_first_physics_profile_logged {
            let physics_timing = world
                .try_resource::<ribbon_plugin_physics::PhysicsDiagnostics>()
                .map(|physics| {
                    (
                        physics.last_reconcile_time_ns,
                        physics.last_fixed_step_time_ns,
                    )
                });
            if let Some((reconcile_ns, fixed_step_ns)) = physics_timing {
                if reconcile_ns > 0 && fixed_step_ns > 0 {
                    eprintln!(
                        "[ATHAR transition] first village physics pass: reconcile={} us, \
                         fixed_step={} us",
                        reconcile_ns / 1_000,
                        fixed_step_ns / 1_000,
                    );
                    world
                        .resource_mut::<ribbon_plugin_physics::PhysicsDiagnostics>()
                        .timing_enabled = false;
                    state.village_first_physics_profile_logged = true;
                }
            }
        }
        if state.phase == GamePhase::Village && !state.village_rain_profile_logged {
            if let Some(rain) = world.try_resource::<ribbon_plugin_weather::RainDiagnostics>() {
                if rain.particle_batches_created > 0 {
                    eprintln!(
                        "[ATHAR transition] rain particle batch: {} entities created in {} us",
                        rain.last_batch_particle_count,
                        rain.last_batch_creation_time_ns / 1_000,
                    );
                    state.village_rain_profile_logged = true;
                }
            }
        }
        update_village_preparation(world, state);
        finish_village_activation_if_ready(world, state);
        update_key_and_door_animation(world, state, dt);
        update_village_door_view(world, state);
        let Some(player) = player_position else {
            return;
        };

        if state.phase == GamePhase::Room {
            let retry_asset = match &state.transition_state {
                VillageTransitionState::TransitionFailed { asset, .. }
                    if interact_pressed
                        && state.door_unlocked
                        && state.door_open_progress >= DOOR_CLEAR_PROGRESS
                        && near(player, state.door_position, DOOR_INTERACT_RADIUS) =>
                {
                    Some(asset.clone())
                }
                _ => None,
            };
            if let Some(asset) = retry_asset {
                if let Err(error) = retry_failed_village_preparation(world, state, &asset) {
                    state.set_message(format!("Village retry failed: {error}"), 8.0);
                    eprintln!("[ATHAR transition] retry failed for `{asset}`: {error}");
                }
            } else if interact_pressed {
                match choose_room_interaction(
                    player,
                    state.key_collected,
                    state.door_unlocked,
                    state.key_position,
                    state.door_position,
                ) {
                    RoomInteraction::PickUpKey => {
                        state.key_collected = true;
                        if let Some(key) = state.key_entity.take() {
                            let _ = world.despawn(key);
                        }
                        if let Some(light) = state.key_light.take() {
                            let _ = world.despawn(light);
                        }
                        state.set_message("Brass key collected. Find the locked exit door.", 5.0);
                    }
                    RoomInteraction::LockedDoor => {
                        state.set_message("LOCKED. You need the brass key.", 3.0);
                    }
                    RoomInteraction::UnlockDoor => {
                        state.door_unlocked = true;
                        state.set_message("The key turns. The old door creaks open...", 4.0);
                    }
                    RoomInteraction::DoorAlreadyUnlocked => {
                        let text = if state.door_open_progress >= 1.0 {
                            "The exit is open. Step into the village."
                        } else {
                            "The old door is still creaking open..."
                        };
                        state.set_message(text, 2.5);
                    }
                    RoomInteraction::NothingNearby => {
                        state.set_message("Nothing close enough to interact with.", 1.7);
                    }
                }
            }

            // The doorway is intentionally traversable only after the door mesh swings clear.
            if can_enter_village(
                state.phase,
                state.door_unlocked,
                state.door_open_progress,
                player,
            ) && matches!(
                &state.transition_state,
                VillageTransitionState::VillageReady
            ) {
                if let Err(error) = begin_village_activation(world, state) {
                    eprintln!("[ATHAR transition] activation request refused: {error}");
                }
            }
        }
    });

    update_hud(world);
}

fn update_hud(world: &mut World) {
    let scene_model_count = world
        .resource::<StreamingCoordinator>()
        .snapshot()
        .regions
        .into_iter()
        .find(|region| region.id == RegionId::new(village::VILLAGE_REGION_ID))
        .map_or(0, |region| region.spawned_model_instances);
    let (
        phase,
        key_collected,
        door_progress,
        key_position,
        door_position,
        message,
        message_seconds,
        rain_count,
        scene_model_count,
        transition_state,
        village_ready_models,
        transition_preparation_ms,
    ) = {
        let state = world.resource::<AdventureState>();
        (
            state.phase,
            state.key_collected,
            state.door_open_progress,
            state.key_position,
            state.door_position,
            state.message.clone(),
            state.message_seconds,
            if state.phase == GamePhase::Village {
                world
                    .try_resource::<ribbon_plugin_weather::RainSettings>()
                    .map_or(0, ribbon_plugin_weather::RainSettings::active_particles)
            } else {
                0
            },
            scene_model_count,
            state.transition_state.clone(),
            state.village_ready_models,
            state.transition_preparation_ms,
        )
    };
    let player = world.resource::<AdventureState>().camera_entity;
    let player_position = world
        .get::<Transform3D>(player)
        .map(|transform| transform.translation)
        .unwrap_or(Vec3::ZERO);

    let mut lines = match phase {
        GamePhase::Room => {
            let mut lines = vec![
                "HORROR SLICE - THE LOST KEY".to_owned(),
                "WASD MOVE | MOUSE LOOK | SHIFT SPRINT".to_owned(),
                "E INTERACT | ESC RELEASE / CLICK RECAPTURE".to_owned(),
            ];
            if !key_collected {
                lines.push("OBJECTIVE: FIND THE BRASS KEY".to_owned());
                if near(player_position, key_position, KEY_INTERACT_RADIUS) {
                    lines.push("KEY IN REACH: PRESS E TO PICK UP".to_owned());
                } else {
                    lines.push("SEARCH THE ROOM FLOOR FOR A BRASS KEY".to_owned());
                }
            } else {
                lines.push("BRASS KEY: CARRIED".to_owned());
                if door_progress >= DOOR_CLEAR_PROGRESS {
                    lines.push("EXIT: WALK THROUGH THE OPEN DOOR".to_owned());
                } else if near(player_position, door_position, DOOR_INTERACT_RADIUS) {
                    lines.push("DOOR IN REACH: PRESS E TO UNLOCK".to_owned());
                } else {
                    lines.push("OBJECTIVE: UNLOCK THE EXIT DOOR".to_owned());
                }
            }
            match &transition_state {
                VillageTransitionState::PreparingVillage => {
                    lines.push(format!(
                        "PREPARING VILLAGE: {village_ready_models}/{} MODELS GPU-READY",
                        village::VILLAGE_MODEL_ASSETS.len()
                    ));
                    if door_progress >= DOOR_CLEAR_PROGRESS {
                        lines.push("EXIT HELD INSIDE UNTIL VILLAGE IS READY".to_owned());
                    }
                }
                VillageTransitionState::VillageReady => {
                    lines.push(format!(
                        "VILLAGE READY IN {} ms: EXIT IS SAFE",
                        transition_preparation_ms.unwrap_or_default()
                    ));
                }
                VillageTransitionState::TransitionFailed { asset, reason } => {
                    lines.push(format!("LOAD FAILED: {asset}"));
                    lines.push(format!("{reason}; PRESS E NEAR OPEN EXIT TO RETRY"));
                }
                VillageTransitionState::CommittingTransition => {
                    lines.push("ENTERING WET CREEK...".to_owned());
                }
                VillageTransitionState::VillageActive => {}
            }
            lines
        }
        GamePhase::Village => vec![
            "WET CREEK - ABANDONED VILLAGE".to_owned(),
            "WASD MOVE | MOUSE LOOK | SHIFT SPRINT".to_owned(),
            "ESC RELEASE MOUSE | CLICK TO RECAPTURE".to_owned(),
            format!("HEAVY RAIN: {rain_count} DROPS IN THE STORM"),
            format!("MODEL INSTANCES: {scene_model_count}"),
            "VILLAGE: 60 HOUSES | 44 TREES | 8 STREET LAMPS".to_owned(),
            "FOLLOW THE COBBLE ROAD TO THE OLD HALL".to_owned(),
        ],
    };
    if message_seconds > 0.0 && lines.len() < 11 {
        lines.push(message);
    }
    if world.try_resource::<WindowTextOverlay>().is_some() {
        *world.resource_mut::<WindowTextOverlay>() = WindowTextOverlay {
            lines,
            visible: true,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::super::GamePhase;
    use super::{
        can_enter_village, choose_room_interaction, horizontal_distance_squared, near,
        next_door_open_progress, village_transition_can_commit, RoomInteraction,
        DOOR_ANIMATION_SECONDS,
    };
    use ribbon_plugin_render3d::glam::Vec3;

    #[test]
    fn interaction_radius_uses_horizontal_distance_not_camera_pitch_or_height() {
        let floor_target = Vec3::new(1.0, 0.0, 2.0);
        let eye = Vec3::new(1.5, 12.0, 2.0);
        assert!(near(eye, floor_target, 0.6));
        assert!((horizontal_distance_squared(eye, floor_target) - 0.25).abs() < 1e-6);
        assert!(!near(eye, floor_target, 0.4));
    }

    #[test]
    fn key_door_interaction_requires_the_key_and_proximity() {
        let key = Vec3::new(0.85, 0.025, 1.55);
        let door = Vec3::new(5.84, 0.0, -2.0);
        assert_eq!(
            choose_room_interaction(key, false, false, key, door),
            RoomInteraction::PickUpKey
        );
        assert_eq!(
            choose_room_interaction(door, false, false, key, door),
            RoomInteraction::LockedDoor
        );
        assert_eq!(
            choose_room_interaction(door, true, false, key, door),
            RoomInteraction::UnlockDoor
        );
        assert_eq!(
            choose_room_interaction(door, true, true, key, door),
            RoomInteraction::DoorAlreadyUnlocked
        );
        assert_eq!(
            choose_room_interaction(Vec3::new(-4.0, 1.6, 4.0), false, false, key, door),
            RoomInteraction::NothingNearby
        );
    }

    #[test]
    fn village_commit_requires_ready_state_and_room_phase() {
        use super::super::VillageTransitionState;

        assert!(!village_transition_can_commit(
            GamePhase::Room,
            &VillageTransitionState::PreparingVillage
        ));
        assert!(!village_transition_can_commit(
            GamePhase::Room,
            &VillageTransitionState::TransitionFailed {
                asset: "village_house.glb".to_owned(),
                reason: "test failure".to_owned(),
            }
        ));
        assert!(village_transition_can_commit(
            GamePhase::Room,
            &VillageTransitionState::VillageReady
        ));
        assert!(!village_transition_can_commit(
            GamePhase::Room,
            &VillageTransitionState::CommittingTransition
        ));
        assert!(!village_transition_can_commit(
            GamePhase::Room,
            &VillageTransitionState::VillageActive
        ));
        assert!(!village_transition_can_commit(
            GamePhase::Village,
            &VillageTransitionState::VillageReady
        ));
    }

    #[test]
    fn streamed_preview_is_visible_only_after_region_readiness_and_all_visual_spawns() {
        use super::{
            village_door_view_is_revealed, village_preview_is_resident,
            VILLAGE_PREVIEW_DOOR_PROGRESS,
        };
        use ribbon_plugin_streaming::RegionState;

        assert!(!village_preview_is_resident(
            &RegionState::Unloaded,
            136,
            136
        ));
        assert!(!village_preview_is_resident(
            &RegionState::LoadingAssets,
            136,
            136
        ));
        assert!(!village_preview_is_resident(&RegionState::Ready, 135, 136));
        assert!(village_preview_is_resident(&RegionState::Ready, 136, 136));
        assert!(village_preview_is_resident(
            &RegionState::Activating,
            136,
            136
        ));
        assert!(!village_preview_is_resident(
            &RegionState::Failed("test".to_owned()),
            136,
            136
        ));

        assert!(!village_door_view_is_revealed(
            GamePhase::Room,
            VILLAGE_PREVIEW_DOOR_PROGRESS - 0.01,
        ));
        assert!(village_door_view_is_revealed(
            GamePhase::Room,
            VILLAGE_PREVIEW_DOOR_PROGRESS,
        ));
        assert!(!village_door_view_is_revealed(GamePhase::Village, 1.0));
    }

    #[test]
    fn open_door_reveals_backdrop_without_switching_lighting_before_streamed_preview() {
        use super::super::AdventureState;
        use super::{
            update_village_door_view, StreamingCoordinator, VILLAGE_PREVIEW_DOOR_PROGRESS,
        };
        use ribbon_app::World;
        use ribbon_plugin_render3d::{Camera3D, FogSettings, SunLight, Transform3D};
        use ribbon_render::gpu_window::ClearColor;

        let mut world = World::new();
        world.insert_resource(StreamingCoordinator::default());
        world.insert_resource(ClearColor([0.003, 0.004, 0.004, 1.0]));
        world.insert_resource(SunLight {
            direction: Vec3::new(0.0, -1.0, -0.15),
            color: Vec3::ZERO,
            ambient: Vec3::splat(0.032),
        });
        world.insert_resource(FogSettings {
            color: Vec3::new(0.018, 0.025, 0.021),
            density: 0.006,
            base_height: 0.0,
            height_falloff: 0.08,
        });
        let camera = world.spawn((
            Transform3D::default(),
            Camera3D {
                fov_y: 68.0_f32.to_radians(),
                near: 0.04,
                far: 100.0,
            },
        ));
        let body = world.spawn((Transform3D::default(),));
        use super::super::VillageTransitionState;
        let mut state = AdventureState::new(
            camera,
            body,
            Vec::new(),
            Vec::new(),
            None,
            None,
            None,
            None,
            1.2,
            Vec3::ZERO,
            Vec3::ZERO,
        );
        state.transition_state = VillageTransitionState::VillageReady;
        state.door_open_progress = VILLAGE_PREVIEW_DOOR_PROGRESS;

        update_village_door_view(&mut world, &mut state);
        assert_eq!(world.resource::<ClearColor>().0, [0.24, 0.29, 0.34, 1.0]);
        assert_eq!(world.resource::<SunLight>().ambient, Vec3::splat(0.032));
        assert!(!state.village_preview_lighting_active);
    }

    #[test]
    fn door_opening_has_a_bounded_nonzero_animation_duration() {
        const {
            const {
                assert!(DOOR_ANIMATION_SECONDS >= 0.5);
            }
        }
        const {
            const {
                assert!(DOOR_ANIMATION_SECONDS <= 1.5);
            }
        }
        assert_eq!(next_door_open_progress(0.0, false, 1.0), 0.0);
        assert!(
            (next_door_open_progress(0.0, true, DOOR_ANIMATION_SECONDS * 0.5) - 0.5).abs() < 1e-6
        );
        assert_eq!(next_door_open_progress(0.9, true, 100.0), 1.0);
        assert_eq!(next_door_open_progress(0.4, true, f32::NAN), 0.4);
        assert_eq!(
            next_door_open_progress(f32::NAN, true, 0.4),
            0.4 / DOOR_ANIMATION_SECONDS
        );
        assert_eq!(next_door_open_progress(0.4, true, -1.0), 0.4);
    }

    #[test]
    fn village_transition_requires_unlocked_open_door_and_the_exit_aperture() {
        let outside = Vec3::new(6.8, 1.65, -2.0);
        assert!(!can_enter_village(GamePhase::Room, false, 0.0, outside));
        assert!(!can_enter_village(GamePhase::Room, true, 0.5, outside));
        assert!(!can_enter_village(GamePhase::Room, false, 1.0, outside));
        assert!(!can_enter_village(GamePhase::Village, true, 1.0, outside));
        assert!(!can_enter_village(
            GamePhase::Room,
            true,
            1.0,
            Vec3::new(6.8, 1.65, -0.8)
        ));
        assert!(!can_enter_village(
            GamePhase::Room,
            true,
            1.0,
            Vec3::new(6.05, 1.65, -2.0)
        ));
        // The player capsule cannot move indefinitely through the room's right-wall collision
        // plane. The transition threshold must be reachable before that collision stops motion.
        assert!(can_enter_village(
            GamePhase::Room,
            true,
            0.9,
            Vec3::new(6.2, 1.65, -2.0)
        ));
        assert!(can_enter_village(GamePhase::Room, true, 0.9, outside));
    }
}
