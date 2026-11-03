use super::*;
use crate::agent::{nav_agent_system, PathCursor};
use crate::perception::{perception_system, SightCursor};
use glam::{Vec2, Vec3};
use ribbon_app::{App, Time};
use std::time::Duration;

#[test]
fn astar_routes_around_a_wall_and_smooths_open_field() {
    let mut grid = NavGrid::new(Vec2::ZERO, 1.0, 8, 6);
    let direct = grid
        .find_path(Vec3::new(0.5, 0.0, 0.5), Vec3::new(7.5, 0.0, 5.5))
        .expect("open grid path");
    assert_eq!(direct.waypoints.len(), 2);
    for y in 0..5 {
        grid.set_walkable(3, y, false);
    }
    let around = grid
        .find_path(Vec3::new(0.5, 0.0, 0.5), Vec3::new(6.5, 0.0, 0.5))
        .expect("gap path");
    assert!(around.waypoints.len() > 2);
    assert!(around.waypoints.iter().all(|p| grid
        .world_to_cell(*p)
        .is_some_and(|(x, y)| grid.cell(x, y).is_some_and(|c| c.walkable))));
}

#[test]
fn astar_does_not_cut_diagonal_corners_and_snaps_nearby_goal() {
    let mut grid = NavGrid::new(Vec2::ZERO, 1.0, 2, 2);
    grid.set_walkable(1, 0, false);
    grid.set_walkable(0, 1, false);
    assert_eq!(
        grid.find_path(Vec3::new(0.5, 0.0, 0.5), Vec3::new(1.5, 0.0, 1.5)),
        Err(NavError::NoPath)
    );
    grid.set_walkable(1, 0, true);
    let goal = Vec3::new(0.5, 0.0, 1.5);
    let path = grid
        .find_path(Vec3::new(0.5, 0.0, 0.5), goal)
        .expect("snapped goal");
    let snapped = grid
        .nearest_walkable(goal, 3)
        .expect("nearby walkable goal");
    assert_eq!(
        path.waypoints.last().copied(),
        Some(grid.cell_center(snapped.0, snapped.1))
    );
}

#[test]
fn behavior_tree_resumes_running_sequence_and_wait() {
    let mut app = App::new();
    app.insert_resource(0u32);
    let mut registry = BtRegistry::default();
    registry.register("tick", |ctx| {
        *ctx.world.resource_mut::<u32>() += 1;
        BtStatus::Success
    });
    let mut tree = BehaviorTree::new(BtNode::Sequence(vec![
        BtNode::Leaf("tick".into()),
        BtNode::Wait { seconds: 0.2 },
        BtNode::Leaf("tick".into()),
    ]));
    let entity = app.world.spawn(());
    assert_eq!(
        tree.tick(&mut app.world, &registry, entity, 0.1),
        BtStatus::Running
    );
    assert_eq!(
        tree.tick(&mut app.world, &registry, entity, 0.1),
        BtStatus::Success
    );
    assert_eq!(*app.world.resource::<u32>(), 2);
}

#[test]
fn behavior_tree_loads_ron_and_validates_unknown_leaf() {
    let node = BtNode::from_ron("Leaf(\"missing\")").expect("RON node");
    let tree = BehaviorTree::new(node);
    let registry = BtRegistry::default();
    assert_eq!(
        tree.validate(&registry),
        Err(BtError::UnknownLeaf("missing".into()))
    );
}

#[test]
fn behavior_tree_selector_inverter_and_repeat_are_supported() {
    let mut app = App::new();
    app.insert_resource(0u32);
    let mut registry = BtRegistry::default();
    registry.register("fail", |_| BtStatus::Failure);
    registry.register("count", |ctx| {
        *ctx.world.resource_mut::<u32>() += 1;
        BtStatus::Success
    });
    let root = BtNode::Sequence(vec![
        BtNode::Inverter(Box::new(BtNode::Leaf("fail".into()))),
        BtNode::Selector(vec![
            BtNode::Leaf("fail".into()),
            BtNode::Repeat {
                child: Box::new(BtNode::Leaf("count".into())),
                times: 3,
            },
        ]),
    ]);
    let mut tree = BehaviorTree::new(root);
    let e = app.world.spawn(());
    assert_eq!(
        tree.tick(&mut app.world, &registry, e, 0.016),
        BtStatus::Success
    );
    assert_eq!(*app.world.resource::<u32>(), 3);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Mode {
    Idle,
    Run,
}
#[test]
fn state_machine_applies_requested_state_and_emits_event() {
    let mut app = App::new();
    register_state_machine::<Mode>(&mut app);
    let entity = app.world.spawn((StateMachine::new(Mode::Idle),));
    app.world
        .get_mut::<StateMachine<Mode>>(entity)
        .expect("machine")
        .request(Mode::Run);
    app.world.resource_mut::<Time>().delta = Duration::from_millis(16);
    app.update();
    assert_eq!(
        app.world
            .get::<StateMachine<Mode>>(entity)
            .expect("machine")
            .current(),
        Mode::Run
    );
}

#[test]
fn physics_bake_marks_ground_walkable() {
    use ribbon_app::Time;
    use ribbon_plugin_physics::{Collider, PhysicsPlugin};
    use ribbon_plugins::AppPluginExt;
    use ribbon_scene::{ScenePlugin, Transform3D};
    use std::time::Duration;
    let mut app = App::new();
    app.add_plugin(ScenePlugin).expect("scene");
    app.add_plugin(PhysicsPlugin).expect("physics");
    app.world.spawn((
        Transform3D::from_translation(Vec3::new(0.0, -0.5, 0.0)),
        Collider::cuboid(Vec3::new(10.0, 0.5, 10.0)),
    ));
    app.world.resource_mut::<Time>().delta = Duration::from_millis(16);
    app.update();
    let cfg = NavBakeConfig {
        min: Vec3::new(-1.0, -1.0, -1.0),
        max: Vec3::new(1.0, 1.0, 1.0),
        cell_size: 1.0,
        agent_radius: 0.0,
        agent_height: 0.5,
        max_step_height: 0.3,
        max_slope_deg: 45.0,
        probe_height_above_ground: 2.0,
    };
    let grid = bake_nav_grid(&app.world, &cfg);
    assert!(grid.cell(0, 0).is_some_and(|c| c.walkable));
}

#[test]
fn sight_does_not_see_target_through_a_wall() {
    use ribbon_app::Time;
    use ribbon_plugin_physics::{Collider, PhysicsPlugin};
    use ribbon_plugins::AppPluginExt;
    use ribbon_scene::{ScenePlugin, Transform3D};
    use std::time::Duration;
    let mut app = App::new();
    app.add_plugin(ScenePlugin).expect("scene");
    app.add_plugin(PhysicsPlugin).expect("physics");
    let observer = app.world.spawn((
        Sight {
            range: 10.0,
            fov_deg: 100.0,
            eye_offset: Vec3::ZERO,
            target_layers: Default::default(),
        },
        Transform3D::from_translation(Vec3::new(0.0, 1.0, 0.0)),
    ));
    let target = app.world.spawn((
        SightTarget,
        Transform3D::from_translation(Vec3::new(0.0, 1.0, -4.0)),
        Collider::ball(0.3),
    ));
    assert!(app.world.get::<Sight>(observer).is_some());
    assert!(app.world.get::<Transform3D>(observer).is_some());
    assert_eq!(app.world.query::<&Sight>().iter().count(), 1);
    let wall = app.world.spawn((
        Transform3D::from_translation(Vec3::new(0.0, 1.0, -2.0)),
        Collider::cuboid(Vec3::new(1.0, 1.0, 0.15)),
    ));
    app.world.insert_resource(AiSettings {
        max_sight_checks_per_update: 32,
    });
    app.world.insert_resource(SightCursor::default());
    app.world.resource_mut::<Time>().delta = Duration::from_millis(16);
    app.update();
    let hit = ribbon_plugin_physics::raycast(
        &app.world,
        ribbon_plugin_physics::RaycastQuery::new(Vec3::new(0.0, 1.0, 0.0), Vec3::NEG_Z, 4.0),
    );
    assert!(
        hit.is_some_and(|h| h.entity == wall),
        "expected wall hit, got {hit:?}"
    );
    perception_system(&mut app.world);
    let perceived = app
        .world
        .get::<Perceived>(observer)
        .map(|p| p.visible.clone());
    assert!(
        perceived.as_ref().is_some_and(|p| !p.contains(&target)),
        "visible={perceived:?}; observer={observer:?}; target={target:?}"
    );
}

#[test]
fn nav_agent_applies_velocity_and_path_budget_rotates_fairly() {
    use ribbon_scene::Transform3D;
    let mut app = App::new();
    app.world.insert_resource(NavWorld {
        grid: NavGrid::new(Vec2::ZERO, 1.0, 8, 2),
    });
    app.world.insert_resource(NavSettings {
        max_paths_per_update: 1,
        max_expansions: 100,
    });
    app.world.insert_resource(PathCursor::default());
    let a = app.world.spawn((
        NavAgent {
            speed: 3.0,
            arrive_radius: 0.1,
            repath_interval: 10.0,
            max_path_age: 20.0,
        },
        NavDestination(Vec3::new(5.5, 0.0, 0.5)),
        Transform3D::from_translation(Vec3::new(0.5, 0.0, 0.5)),
    ));
    let b = app.world.spawn((
        NavAgent::default(),
        NavDestination(Vec3::new(5.5, 0.0, 1.5)),
        Transform3D::from_translation(Vec3::new(0.5, 0.0, 1.5)),
    ));
    nav_agent_system(&mut app.world);
    assert_eq!(
        app.world.get::<NavSteering>(a).map(|s| s.status),
        Some(NavStatus::Moving)
    );
    assert_eq!(
        app.world.get::<NavSteering>(b).map(|s| s.status),
        Some(NavStatus::NoPath)
    );
    nav_agent_system(&mut app.world);
    assert_eq!(
        app.world.get::<NavSteering>(b).map(|s| s.status),
        Some(NavStatus::Moving)
    );
    app.world.resource_mut::<Time>().delta = Duration::from_secs_f32(0.1);
    for _ in 0..18 {
        nav_agent_system(&mut app.world);
        let velocity = app
            .world
            .get::<NavSteering>(a)
            .map_or(Vec3::ZERO, |s| s.desired_velocity);
        if let Some(mut transform) = app.world.get_mut::<Transform3D>(a) {
            transform.translation += velocity * 0.1;
        }
    }
    nav_agent_system(&mut app.world);
    assert_eq!(
        app.world.get::<NavSteering>(a).map(|s| s.status),
        Some(NavStatus::Arrived)
    );
}
