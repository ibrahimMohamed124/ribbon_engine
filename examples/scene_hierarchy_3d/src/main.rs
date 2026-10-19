//! Interactive visual check for 3D parent/child transform propagation.
//!
//! Run from the workspace root with `cargo run -p scene_hierarchy_3d`.
//! The example prints a PASS/FAIL report after checking the resolved world transforms.

use engine_app::{App, AppExit, Stage, Time, World};
use engine_events::WorldEventsExt;
use engine_platform::input::Key;
use engine_platform::{Input, PlatformConfig, PlatformPlugin};
use engine_plugins::AppPluginExt;
use engine_render::gpu_window::{ClearColor, GpuWindowPlugin};
use engine_scene::{GlobalTransform3D, HierarchyExt, ScenePlugin};
use plugin_render3d::glam::{Quat, Vec3};
use plugin_render3d::{BuiltinMesh, Camera3D, MeshInstance, Render3dPlugin, Transform3D};
use std::error::Error;

#[derive(Clone, Copy)]
struct RigRoot;

#[derive(Clone, Copy)]
struct RigJoint;

#[derive(Clone, Copy)]
struct RigLink;

#[derive(Clone, Copy)]
struct RigTip;

struct DemoState {
    root_angle: f32,
    joint_angle: f32,
    paused: bool,
    hierarchy_reported: bool,
    hierarchy_report_attempts: u8,
    frame_limit: Option<u64>,
}

fn spawn_cube(
    world: &mut World,
    position: Vec3,
    scale: Vec3,
    color: [f32; 3],
) -> engine_ecs::Entity {
    world.spawn((
        Transform3D {
            translation: position,
            scale,
            ..Transform3D::default()
        },
        MeshInstance {
            mesh: BuiltinMesh::Cube,
            color,
        },
    ))
}

fn animate_hierarchy(world: &mut World) {
    let delta = world.resource::<Time>().delta.as_secs_f32().clamp(0.0, 0.1);
    let frame = world.resource::<Time>().frame;
    let (pause_pressed, reset_pressed) = {
        let input = world.resource::<Input>();
        (input.just_pressed(Key::Space), input.just_pressed(Key::R))
    };

    let (root_angle, joint_angle, frame_limit) = {
        let state = world.resource_mut::<DemoState>();
        if pause_pressed {
            state.paused = !state.paused;
        }
        if reset_pressed {
            state.root_angle = 0.0;
            state.joint_angle = 0.0;
            state.paused = false;
        }
        if !state.paused {
            state.root_angle += delta * 0.52;
            state.joint_angle += delta * 0.95;
        }
        (state.root_angle, state.joint_angle, state.frame_limit)
    };

    for (_, (transform, _)) in world.query_mut::<(&mut Transform3D, &RigRoot)>() {
        transform.rotation = Quat::from_rotation_y(root_angle);
    }
    for (_, (transform, _)) in world.query_mut::<(&mut Transform3D, &RigJoint)>() {
        transform.rotation = Quat::from_rotation_z(joint_angle);
    }

    if frame_limit.is_some_and(|limit| frame >= limit) {
        println!("[HIERARCHY] Frame limit reached; requesting application shutdown.");
        world.send_event(AppExit);
    }
}

fn vec_near(a: Vec3, b: Vec3) -> bool {
    a.distance(b) <= 0.015
}

fn quat_near(a: Quat, b: Quat) -> bool {
    1.0 - a.normalize().dot(b.normalize()).abs() <= 0.002
}

/// Verifies the same composition rule used by `engine_scene::propagate_transforms_3d`.
/// This is runtime diagnostic output for the example, not a test target.
fn report_hierarchy(world: &mut World) {
    if world.resource::<DemoState>().hierarchy_reported {
        return;
    }

    let root = world
        .query::<(&RigRoot, &Transform3D, &GlobalTransform3D)>()
        .iter()
        .next()
        .map(|(_, (_, local, global))| (*local, *global));
    let joint = world
        .query::<(&RigJoint, &Transform3D, &GlobalTransform3D)>()
        .iter()
        .next()
        .map(|(_, (_, local, global))| (*local, *global));
    let link = world
        .query::<(&RigLink, &Transform3D, &GlobalTransform3D)>()
        .iter()
        .next()
        .map(|(_, (_, local, global))| (*local, *global));
    let tip = world
        .query::<(&RigTip, &Transform3D, &GlobalTransform3D)>()
        .iter()
        .next()
        .map(|(_, (_, local, global))| (*local, *global));

    let (
        Some((root_local, root_global)),
        Some((joint_local, joint_global)),
        Some((link_local, link_global)),
        Some((tip_local, tip_global)),
    ) = (root, joint, link, tip)
    else {
        let state = world.resource_mut::<DemoState>();
        state.hierarchy_report_attempts = state.hierarchy_report_attempts.saturating_add(1);
        if state.hierarchy_report_attempts >= 5 {
            state.hierarchy_reported = true;
            println!("[HIERARCHY] FAIL: one or more GlobalTransform3D values were not propagated.");
            println!("[HIERARCHY] Check that ScenePlugin is registered before the Render3dPlugin.");
        }
        return;
    };

    let expected_joint_position = root_global.translation
        + root_global.rotation * (root_global.scale * joint_local.translation);
    let expected_joint_rotation = root_global.rotation * joint_local.rotation;
    let expected_joint_scale = root_global.scale * joint_local.scale;
    let expected_link_position = joint_global.translation
        + joint_global.rotation * (joint_global.scale * link_local.translation);
    let expected_link_rotation = joint_global.rotation * link_local.rotation;
    let expected_link_scale = joint_global.scale * link_local.scale;
    let expected_tip_position = link_global.translation
        + link_global.rotation * (link_global.scale * tip_local.translation);
    let expected_tip_rotation = link_global.rotation * tip_local.rotation;
    let expected_tip_scale = link_global.scale * tip_local.scale;

    let valid = vec_near(root_local.translation, root_global.translation)
        && quat_near(root_local.rotation, root_global.rotation)
        && vec_near(root_local.scale, root_global.scale)
        && vec_near(expected_joint_position, joint_global.translation)
        && quat_near(expected_joint_rotation, joint_global.rotation)
        && vec_near(expected_joint_scale, joint_global.scale)
        && vec_near(expected_link_position, link_global.translation)
        && quat_near(expected_link_rotation, link_global.rotation)
        && vec_near(expected_link_scale, link_global.scale)
        && vec_near(expected_tip_position, tip_global.translation)
        && quat_near(expected_tip_rotation, tip_global.rotation)
        && vec_near(expected_tip_scale, tip_global.scale);

    if valid {
        println!("[HIERARCHY] PASS: root -> joint -> link -> tip transforms propagated.");
        println!(
            "[HIERARCHY] PASS: local translation, rotation, and scale compose into world transforms."
        );
    } else {
        println!("[HIERARCHY] FAIL: resolved transforms differ from the expected composition.");
        println!(
            "[HIERARCHY] joint expected {:?}, actual {:?}",
            expected_joint_position, joint_global.translation
        );
        println!(
            "[HIERARCHY] link  expected {:?}, actual {:?}",
            expected_link_position, link_global.translation
        );
        println!(
            "[HIERARCHY] tip   expected {:?}, actual {:?}",
            expected_tip_position, tip_global.translation
        );
    }
    println!(
        "[HIERARCHY] root world position: {:?}",
        root_global.translation
    );
    println!(
        "[HIERARCHY] tip world position:  {:?}",
        tip_global.translation
    );
    world.resource_mut::<DemoState>().hierarchy_reported = true;
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
                    "Scene Hierarchy 3D\n\n",
                    "Options:\n",
                    "  --frames N  Exit after N frames\n",
                    "  -h, --help  Show this help\n\n",
                    "Controls: Space pause/resume; R reset."
                ));
                return Ok(());
            }
            other => return Err(format!("unknown argument `{other}`").into()),
        }
    }

    let mut app = App::new();
    app.insert_resource(PlatformConfig {
        title: "Voxelworks | Scene Hierarchy 3D".to_owned(),
        width: 1152,
        height: 720,
    });
    app.insert_resource(ClearColor([0.018, 0.027, 0.043, 1.0]));
    app.insert_resource(DemoState {
        root_angle: 0.0,
        joint_angle: 0.0,
        paused: false,
        hierarchy_reported: false,
        hierarchy_report_attempts: 0,
        frame_limit,
    });

    app.add_plugin(PlatformPlugin)?;
    app.add_plugin(ScenePlugin)?;
    app.add_plugin(GpuWindowPlugin)?;
    app.add_plugin(Render3dPlugin)?;

    app.world.spawn((
        Transform3D::looking_at(
            Vec3::new(0.0, 5.1, 11.5),
            Vec3::new(0.0, 1.05, 0.0),
            Vec3::Y,
        ),
        Camera3D {
            far: 100.0,
            ..Camera3D::default()
        },
    ));

    // A simple floor and grid establish a stable frame of reference.
    spawn_cube(
        &mut app.world,
        Vec3::new(0.0, -0.15, 0.0),
        Vec3::new(11.5, 0.15, 8.5),
        [0.10, 0.13, 0.18],
    );
    for index in -5..=5 {
        let offset = index as f32;
        spawn_cube(
            &mut app.world,
            Vec3::new(offset, 0.008, 0.0),
            Vec3::new(0.009, 0.008, 8.2),
            [0.17, 0.21, 0.27],
        );
    }
    for index in -4..=4 {
        let offset = index as f32;
        spawn_cube(
            &mut app.world,
            Vec3::new(0.0, 0.009, offset),
            Vec3::new(10.8, 0.008, 0.009),
            [0.17, 0.21, 0.27],
        );
    }
    // Static pedestal underneath the animated parent pivot.
    spawn_cube(
        &mut app.world,
        Vec3::new(0.0, 0.68, 0.0),
        Vec3::new(0.42, 0.68, 0.42),
        [0.19, 0.25, 0.33],
    );

    // Parent root: its rotation carries every descendant around the root pivot.
    let root = app.world.spawn((
        RigRoot,
        Transform3D {
            translation: Vec3::new(0.0, 1.25, 0.0),
            ..Transform3D::default()
        },
    ));
    let root_visual = spawn_cube(
        &mut app.world,
        Vec3::ZERO,
        Vec3::splat(0.58),
        [0.15, 0.76, 0.91],
    );
    app.world.set_parent(root_visual, root)?;

    // Child joint rotates locally around Z; its link and tip inherit that movement.
    let joint = app.world.spawn((
        RigJoint,
        Transform3D {
            translation: Vec3::new(1.35, 0.0, 0.0),
            ..Transform3D::default()
        },
    ));
    app.world.set_parent(joint, root)?;

    // A transform-only link anchor prevents the visible beam's thin scale from
    // squashing the child tip. The beam and tip are siblings under this anchor.
    let link = app.world.spawn((
        RigLink,
        Transform3D {
            translation: Vec3::new(0.65, 0.0, 0.0),
            ..Transform3D::default()
        },
    ));
    app.world.set_parent(link, joint)?;

    let link_visual = spawn_cube(
        &mut app.world,
        Vec3::ZERO,
        Vec3::new(1.2, 0.16, 0.16),
        [0.98, 0.43, 0.16],
    );
    app.world.set_parent(link_visual, link)?;

    let tip = spawn_cube(
        &mut app.world,
        Vec3::new(0.75, 0.0, 0.0),
        Vec3::splat(0.34),
        [0.98, 0.82, 0.25],
    );
    app.world.set_parent(tip, link)?;
    app.world.insert(tip, RigTip)?;

    // A counterweight and a green satellite make the parent's rotation easy to see.
    let counterweight = spawn_cube(
        &mut app.world,
        Vec3::new(-1.55, 0.0, 0.0),
        Vec3::splat(0.38),
        [0.78, 0.27, 0.67],
    );
    app.world.set_parent(counterweight, root)?;
    let satellite = spawn_cube(
        &mut app.world,
        Vec3::new(0.0, 0.0, 1.45),
        Vec3::splat(0.3),
        [0.25, 0.86, 0.48],
    );
    app.world.set_parent(satellite, root)?;

    app.add_system(Stage::Update, animate_hierarchy);
    // ScenePlugin registers transform propagation first; this diagnostic observes the results.
    app.add_system(Stage::PostUpdate, report_hierarchy);

    println!("Voxelworks Scene Hierarchy 3D");
    println!(
        "Visual: cyan root pivot, orange arm, yellow tip, magenta counterweight, green satellite."
    );
    println!(
        "Expected: the parent rotates all children; the joint independently swings the arm and tip."
    );
    println!("Controls: Space pause/resume; R reset; --frames N exits after N frames.");
    println!("A [HIERARCHY] PASS/FAIL report should appear once transform propagation has run.");

    app.build()?.run();
    Ok(())
}
