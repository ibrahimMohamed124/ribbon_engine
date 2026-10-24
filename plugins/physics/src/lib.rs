//! Collision and movement systems for voxel worlds.
//!
//! This crate is scaffolded for the staged engine refactor.
pub mod physics;

pub use physics::*;

use ribbon_app::{App, Stage};
use ribbon_ecs::World;
use ribbon_math::Vect;
use ribbon_plugin_voxel::VoxelWorldRes;
use ribbon_plugins::Plugin;
use ribbon_scene::Transform;

/// Collision footprint used by the horizontal move system.
pub struct Collider(pub BodyShape);

/// Requested horizontal displacement, consumed during `FixedUpdate`.
#[derive(Clone, Copy, Debug, Default)]
pub struct MoveIntent {
    pub delta: Vect,
}

/// Integrate queued horizontal motion against the active voxel grid.
pub fn apply_move_intents(world: &mut World) {
    if world.try_resource::<VoxelWorldRes>().is_none() {
        return;
    }
    world.resource_scope::<VoxelWorldRes, _>(|world, voxels| {
        for (_, (collider, intent, transform)) in
            world.query_mut::<(&Collider, &mut MoveIntent, &mut Transform)>()
        {
            transform.translation = physics::slide_xy(
                &voxels.0,
                voxels.1,
                transform.translation,
                (intent.delta.x, intent.delta.y),
                &collider.0,
            );
            intent.delta = Vect::default();
        }
    });
}

/// Fixed-step voxel collision plugin; vertical motion remains game-side.
pub struct PhysicsPlugin;

impl Plugin for PhysicsPlugin {
    fn name(&self) -> &'static str {
        "PhysicsPlugin"
    }

    fn build(&self, app: &mut App) {
        app.add_system(Stage::FixedUpdate, apply_move_intents);
    }
}

#[cfg(test)]
mod system_tests {
    use super::{apply_move_intents, BodyShape, Collider, MoveIntent};
    use ribbon_ecs::World;
    use ribbon_math::Vect;
    use ribbon_plugin_voxel::VoxelWorldRes;
    use ribbon_scene::Transform;

    #[test]
    fn fixed_step_consumes_horizontal_intent_and_preserves_height() {
        let mut world = World::new();
        world.insert_resource(VoxelWorldRes(
            ribbon_plugin_voxel::VoxelWorld::new(4, 4, 4, vec![0; 64]),
            0,
        ));
        let entity = world.spawn((
            Collider(BodyShape::new(0.1, [0.0])),
            MoveIntent {
                delta: Vect::new(1.0, 0.5, 8.0),
            },
            Transform {
                translation: Vect::new(1.0, 1.0, 2.0),
                ..Transform::default()
            },
        ));
        apply_move_intents(&mut world);
        assert_eq!(
            world.get::<Transform>(entity).unwrap().translation,
            Vect::new(2.0, 1.5, 2.0)
        );
        assert_eq!(
            world.get::<MoveIntent>(entity).unwrap().delta,
            Vect::default()
        );
    }
}
