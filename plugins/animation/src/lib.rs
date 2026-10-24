//! Deterministic entity and sprite animation data and update systems.
//!
//! This plugin advances normalized sprite frames only; game-specific animation states remain game-owned.

use ribbon_app::{App, Stage, Time};
use ribbon_ecs::World;
use ribbon_plugins::Plugin;

/// Sprite animation selection and fractional frame position.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SpriteAnimator {
    pub state: u8,
    pub frame: f32,
    pub fps: f32,
}

/// Advance every animator by the current app delta, preserving fractional frames.
pub fn advance_sprite_animators(world: &mut World) {
    let dt = world.resource::<Time>().delta.as_secs_f32();
    for (_, animator) in world.query_mut::<&mut SpriteAnimator>() {
        animator.frame += dt * animator.fps;
    }
}

/// Registers sprite animation advancement in the regular update stage.
pub struct AnimationPlugin;

impl Plugin for AnimationPlugin {
    fn name(&self) -> &'static str {
        "AnimationPlugin"
    }

    fn build(&self, app: &mut App) {
        app.add_system(Stage::Update, advance_sprite_animators);
    }
}

#[cfg(test)]
mod tests {
    use super::{advance_sprite_animators, SpriteAnimator};
    use ribbon_app::{Time, World};
    use std::time::Duration;

    #[test]
    fn advances_fractional_sprite_frames_from_virtual_time() {
        let mut world = World::new();
        world.insert_resource(Time {
            delta: Duration::from_millis(250),
            ..Time::default()
        });
        let entity = world.spawn((SpriteAnimator {
            state: 3,
            frame: 2.0,
            fps: 8.0,
        },));
        advance_sprite_animators(&mut world);
        assert_eq!(world.get::<SpriteAnimator>(entity).unwrap().frame, 4.0);
        assert_eq!(world.get::<SpriteAnimator>(entity).unwrap().state, 3);
    }
}
