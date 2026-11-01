//! A lit cube spinning above a floor slab, drawn by the GPU mesh pipeline.
//!
//! Usage: `cargo run -p cube_spin [-- --frames N]`.

use ribbon_app::{App, AppExit, Stage, Time, World};
use ribbon_events::WorldEventsExt;
use ribbon_platform::{PlatformConfig, PlatformPlugin};
use ribbon_plugin_render3d::glam::{Quat, Vec3};
use ribbon_plugin_render3d::{BuiltinMesh, Camera3D, MeshInstance, Render3dPlugin, Transform3D};
use ribbon_plugins::AppPluginExt;
use ribbon_render::gpu_window::{ClearColor, GpuWindowPlugin};
use ribbon_scene::ScenePlugin;
use std::error::Error;

/// Radians per second around each axis.
struct Spin(Vec3);

struct FrameLimit(Option<u64>);

fn spin_system(world: &mut World) {
    let dt = world.resource::<Time>().delta.as_secs_f32();
    for (_, (transform, spin)) in world.query_mut::<(&mut Transform3D, &Spin)>() {
        transform.rotation = Quat::from_rotation_y(spin.0.y * dt)
            * Quat::from_rotation_x(spin.0.x * dt)
            * transform.rotation;
    }
    let frame = world.resource::<Time>().frame;
    if world
        .resource::<FrameLimit>()
        .0
        .is_some_and(|limit| frame >= limit)
    {
        world.send_event(AppExit);
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut limit = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--frames" => {
                limit = Some(
                    args.next()
                        .ok_or("--frames expects a number")?
                        .parse::<u64>()?,
                );
            }
            other => return Err(format!("unknown argument `{other}`").into()),
        }
    }
    let mut app = App::new();
    app.insert_resource(PlatformConfig {
        title: "Ribbon cube".to_owned(),
        width: 960,
        height: 540,
    });
    app.insert_resource(ClearColor([0.03, 0.04, 0.06, 1.0]));
    app.insert_resource(FrameLimit(limit));
    app.add_plugin(PlatformPlugin)?;
    app.add_plugin(ScenePlugin)?;
    app.add_plugin(GpuWindowPlugin)?;
    app.add_plugin(Render3dPlugin)?;

    app.world.spawn((
        Transform3D::looking_at(Vec3::new(3.5, 2.5, 5.0), Vec3::new(0.0, 0.2, 0.0), Vec3::Y),
        Camera3D::default(),
    ));
    app.world.spawn((
        Transform3D::default(),
        MeshInstance {
            mesh: BuiltinMesh::Cube,
            color: [0.9, 0.45, 0.15],
        },
        Spin(Vec3::new(0.6, 0.9, 0.0)),
    ));
    app.world.spawn((
        Transform3D {
            translation: Vec3::new(0.0, -1.0, 0.0),
            scale: Vec3::new(6.0, 0.1, 6.0),
            ..Transform3D::default()
        },
        MeshInstance {
            mesh: BuiltinMesh::Cube,
            color: [0.35, 0.37, 0.4],
        },
    ));
    app.add_system(Stage::Update, spin_system);
    app.build()?.run();
    Ok(())
}
