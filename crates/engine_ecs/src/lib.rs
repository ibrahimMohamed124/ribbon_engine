//! Safe entity/component access and typed resources for Voxelworks systems.
//!
//! hecs is deliberately contained here so downstream crates use one engine ECS API.

mod commands;
mod world;

pub use commands::Commands;
pub use hecs::{Bundle, Component, Entity, Query, QueryBorrow, QueryMut, Ref, RefMut};
pub use world::{Resources, SyncResource, World};

#[cfg(test)]
mod tests {
    use super::{Commands, Entity, World};

    #[derive(Debug, PartialEq)]
    struct Count(i32);

    #[derive(Debug, PartialEq)]
    struct Label(&'static str);

    #[test]
    fn stale_entity_handles_are_rejected_after_despawn_and_reuse() {
        let mut world = World::new();
        let old = world.spawn((Count(1),));
        world.despawn(old).unwrap();
        let new = world.spawn((Count(2),));
        assert_ne!(old, new);
        assert!(!world.contains(old));
        assert!(world.get::<Count>(old).is_none());
        assert!(world.contains(new));
    }

    #[test]
    fn query_mut_changes_matching_components() {
        let mut world = World::new();
        world.spawn((Count(1),));
        world.spawn((Count(5), Label("ignored")));
        for (_, count) in world.query_mut::<&mut Count>() {
            count.0 += 10;
        }
        let sum: i32 = world
            .query::<&Count>()
            .iter()
            .map(|(_, count)| count.0)
            .sum();
        assert_eq!(sum, 26);
    }

    #[test]
    fn resource_scope_restores_the_resource_after_unwind() {
        let mut world = World::new();
        world.insert_resource(Count(3));
        let entity = world.spawn((Label("scope"),));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            world.resource_scope::<Count, _>(|world, count| {
                count.0 = 9;
                assert!(world.contains(entity));
                panic!("test unwind");
            });
        }));
        assert!(result.is_err());
        assert_eq!(world.resource::<Count>().0, 9);
    }

    #[test]
    fn commands_apply_in_submission_order() {
        let mut world = World::new();
        let entity: Entity = world.spawn((Count(0),));
        let mut commands = Commands::new();
        commands.insert(entity, Count(1));
        commands.remove::<Count>(entity);
        commands.insert(entity, Count(2));
        commands.apply(&mut world);
        assert_eq!(world.get::<Count>(entity).unwrap().0, 2);
    }
}
