//! ATHAR's intentionally small, game-owned progress save section.
use crate::{
    scripts::player::FirstPersonController,
    scripts::{AdventureState, GamePhase},
};
use ribbon_app::{LogLevel, World, WorldEventsExt};
use ribbon_save::{LoadRequest, SaveError, SaveSection};
use ribbon_scene::Transform3D as SceneTransform3D;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy)]
pub(crate) struct RoomStartPosition(pub ribbon_plugin_render3d::glam::Vec3);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct AtharProgress {
    pub phase_is_village: bool,
    pub key_collected: bool,
    pub door_open_progress: f32,
    pub player_position: [f32; 3],
    pub yaw: f32,
}
impl SaveSection for AtharProgress {
    const KEY: &'static str = "athar.progress";
    const VERSION: u32 = 1;
    fn capture(world: &ribbon_ecs::World) -> Self {
        let state = world.resource::<AdventureState>();
        let position = world
            .get::<&SceneTransform3D>(state.camera_entity)
            .map_or([0.0, 0.0, 0.0], |t| t.translation.to_array());
        Self {
            phase_is_village: state.phase == GamePhase::Village,
            key_collected: state.key_collected,
            door_open_progress: state.door_open_progress,
            player_position: position,
            yaw: world.resource::<FirstPersonController>().yaw,
        }
    }
    fn apply(self, world: &mut ribbon_ecs::World) -> Result<(), SaveError> {
        let village = self.phase_is_village;
        if village {
            world
                .resource_mut::<ribbon_app::LogBuffer>()
                .push(LogLevel::Warn, "village saves are not restored yet");
        }
        let (camera, start) = {
            (
                world.resource::<AdventureState>().camera_entity,
                world.resource::<RoomStartPosition>().0,
            )
        };
        let state = world.resource_mut::<AdventureState>();
        state.phase = GamePhase::Room;
        state.key_collected = self.key_collected;
        state.door_unlocked = self.key_collected;
        state.door_open_progress = self.door_open_progress.clamp(0.0, 1.0);
        if world.try_resource::<FirstPersonController>().is_some() {
            world.resource_mut::<FirstPersonController>().yaw =
                if self.yaw.is_finite() { self.yaw } else { 0.0 };
        }
        if let Some(mut transform) = world.get_mut::<SceneTransform3D>(camera) {
            transform.translation = if village {
                start
            } else {
                ribbon_plugin_render3d::glam::Vec3::from_array(self.player_position)
            };
        }
        Ok(())
    }
}

pub(crate) fn request_continue(world: &mut World) {
    if ribbon_save::slot_exists(world, "autosave") {
        world.send_event(LoadRequest {
            slot: "autosave".into(),
        });
    } else {
        world
            .resource_mut::<ribbon_app::LogBuffer>()
            .push(LogLevel::Info, "no ATHAR autosave slot exists");
    }
}

#[cfg(test)]
mod tests {
    use super::AtharProgress;
    #[test]
    fn athar_progress_round_trips_through_ron() {
        let value = AtharProgress {
            phase_is_village: true,
            key_collected: true,
            door_open_progress: 0.75,
            player_position: [1.0, 2.0, 3.0],
            yaw: 0.4,
        };
        let encoded = ron::to_string(&value).expect("encode progress");
        assert_eq!(
            ron::from_str::<AtharProgress>(&encoded).expect("decode progress"),
            value
        );
    }
}
