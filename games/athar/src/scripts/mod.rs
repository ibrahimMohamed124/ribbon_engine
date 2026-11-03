//! Game-specific scripts for the Horror Slice.
//!
//! Reusable rendering, asset loading, collision resolution, and input snapshots remain in the
//! engine. These scripts own this game's player controls, quest state, and village layout. The
//! engine particle emitter is spawned by ATHAR when the village becomes active.

pub mod gameplay;
pub mod player;
pub mod village;

use ribbon_assets::Handle;
use ribbon_ecs::Entity;
use ribbon_plugin_gltf::ModelAsset;
use ribbon_plugin_render3d::glam::Vec3;
use std::collections::HashMap;
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GamePhase {
    Room,
    Village,
}

/// Weak handles for legacy/game-owned room placements. Streamed regions retain their own
/// generation-checked `AssetRef<ModelAsset>` leases through `StreamingCoordinator`.
// Retained for compatibility with the legacy non-streamed village spawn path and its tests.
#[allow(dead_code)]
#[derive(Clone, Debug, Default)]
pub struct GameModelHandles(pub HashMap<String, Handle<ModelAsset>>);

/// Explicit readiness/commit state for ATHAR's one-way room-to-village transition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VillageTransitionState {
    PreparingVillage,
    VillageReady,
    CommittingTransition,
    VillageActive,
    TransitionFailed { asset: String, reason: String },
}

/// State for the key/door quest and the one-way room-to-village transition.
pub struct AdventureState {
    pub phase: GamePhase,
    pub transition_state: VillageTransitionState,
    /// Unique required village model generations confirmed CPU- and GPU-ready.
    pub village_ready_models: usize,
    pub transition_preparation_started: Instant,
    pub transition_preparation_ms: Option<u64>,
    /// Logs the first physics reconciliation after village colliders are activated.
    pub village_first_physics_profile_logged: bool,
    /// Logs the one-shot 3D transform propagation sample after destination activation.
    pub village_transform_profile_logged: bool,
    pub camera_entity: Entity,
    pub rain_emitter: Option<Entity>,
    pub player_body_entity: Entity,
    pub room_entities: Vec<Entity>,
    /// Room prop lights to remove when leaving the room. The player flashlight is deliberately
    /// excluded so it remains attached to the camera outdoors.
    pub room_lights: Vec<Entity>,
    /// Keeps exterior lighting setup one-shot while the open door is visible from the room.
    pub village_preview_lighting_active: bool,
    pub key_entity: Option<Entity>,
    pub key_light: Option<Entity>,
    pub door_entity: Option<Entity>,
    pub door_collider_entity: Option<Entity>,
    pub door_collider_half_height: f32,
    pub key_collected: bool,
    pub door_unlocked: bool,
    pub door_open_progress: f32,
    pub door_collision_removed: bool,
    pub key_position: Vec3,
    pub door_position: Vec3,
    pub save_point_position: Vec3,
    pub elapsed: f32,
    pub message: String,
    pub message_seconds: f32,
}

impl AdventureState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        camera_entity: Entity,
        player_body_entity: Entity,
        room_entities: Vec<Entity>,
        room_lights: Vec<Entity>,
        key_entity: Option<Entity>,
        key_light: Option<Entity>,
        door_entity: Option<Entity>,
        door_collider_entity: Option<Entity>,
        door_collider_half_height: f32,
        key_position: Vec3,
        door_position: Vec3,
        save_point_position: Vec3,
    ) -> Self {
        Self {
            phase: GamePhase::Room,
            transition_state: VillageTransitionState::PreparingVillage,
            village_ready_models: 0,
            transition_preparation_started: Instant::now(),
            transition_preparation_ms: None,
            village_first_physics_profile_logged: false,
            village_transform_profile_logged: false,
            camera_entity,
            rain_emitter: None,
            player_body_entity,
            room_entities,
            room_lights,
            village_preview_lighting_active: false,
            key_entity,
            key_light,
            door_entity,
            door_collider_entity,
            door_collider_half_height,
            key_collected: false,
            door_unlocked: false,
            door_open_progress: 0.0,
            door_collision_removed: false,
            key_position,
            door_position,
            save_point_position,
            elapsed: 0.0,
            message: "Find the brass key.".to_owned(),
            message_seconds: 5.0,
        }
    }

    pub fn set_message(&mut self, message: impl Into<String>, seconds: f32) {
        self.message = message.into();
        self.message_seconds = seconds.max(0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::{AdventureState, GamePhase, VillageTransitionState};
    use ribbon_plugin_render3d::glam::Vec3;

    #[test]
    fn adventure_starts_in_the_room_without_key_or_unlocked_door() {
        let mut world = ribbon_ecs::World::new();
        let camera = world.spawn((Label,));
        let state = AdventureState::new(
            camera,
            camera,
            Vec::new(),
            Vec::new(),
            None,
            None,
            None,
            None,
            1.2,
            Vec3::new(0.85, 0.025, 1.55),
            Vec3::new(5.84, 0.0, -2.0),
            Vec3::new(-0.60, 0.0, 2.35),
        );
        assert_eq!(state.phase, GamePhase::Room);
        assert_eq!(
            state.transition_state,
            VillageTransitionState::PreparingVillage
        );
        assert_eq!(state.village_ready_models, 0);
        assert!(!state.key_collected);
        assert!(!state.door_unlocked);
        assert_eq!(state.door_open_progress, 0.0);
    }

    struct Label;
}
