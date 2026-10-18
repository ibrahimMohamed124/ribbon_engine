//! Loads a glTF model and orbits a camera around it.
//!
//! Usage: `cargo run -p model_viewer [-- path/to/model.glb] [--frames N]`.
//! Without a path the bundled textured cube is shown. Only `.glb` files (or
//! `.gltf` files with embedded data) are supported.

use engine_app::{App, AppExit, Stage, Time, World};
use engine_assets::{AssetPlugin, AssetServer, FsSource};
use engine_events::WorldEventsExt;
use engine_platform::{PlatformConfig, PlatformPlugin};
use engine_plugins::AppPluginExt;
use engine_render::gpu_window::{ClearColor, GpuWindowPlugin};
use plugin_gltf::{GltfLoader, ModelAsset};
use plugin_render3d::glam::{Quat, Vec3};
use plugin_render3d::{Camera3D, ModelInstance, Render3dPlugin, Transform3D};
use std::error::Error;
use std::path::{Path, PathBuf};

struct Orbit {
    center: Vec3,
    distance: f32,
    height: f32,
    angle: f32,
}

struct FrameLimit(Option<u64>);

fn orbit_system(world: &mut World) {
    let dt = world.resource::<Time>().delta.as_secs_f32();
    let frame = world.resource::<Time>().frame;
    let (center, distance, height, angle) = {
        let orbit = world.resource_mut::<Orbit>();
        orbit.angle += dt * 0.5;
        (orbit.center, orbit.distance, orbit.height, orbit.angle)
    };
    let eye = center + Vec3::new(angle.sin() * distance, height, angle.cos() * distance);
    for (_, (pose, _)) in world.query_mut::<(&mut Transform3D, &Camera3D)>() {
        *pose = Transform3D::looking_at(eye, center, Vec3::Y);
    }
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
    let mut model_path: Option<PathBuf> = None;
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
            path if !path.starts_with("--") => model_path = Some(PathBuf::from(path)),
            other => return Err(format!("unknown argument `{other}`").into()),
        }
    }
    let (root, file) = match model_path {
        Some(path) => {
            let root = path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."))
                .to_path_buf();
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or("model path has no file name")?
                .to_owned();
            (root, name)
        }
        None => (
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets"),
            "models/checker_cube.glb".to_owned(),
        ),
    };

    let mut app = App::new();
    app.insert_resource(PlatformConfig {
        title: "Voxelworks model viewer".to_owned(),
        width: 960,
        height: 540,
    });
    app.insert_resource(ClearColor([0.03, 0.04, 0.06, 1.0]));
    app.insert_resource(FrameLimit(limit));
    app.add_plugin(PlatformPlugin)?;
    app.add_plugin(AssetPlugin)?;
    app.add_plugin(GpuWindowPlugin)?;
    app.add_plugin(Render3dPlugin)?;

    let (handle, bounds) = {
        let server = app.world.resource_mut::<AssetServer>();
        server.register_loader(GltfLoader);
        server.add_source(0, FsSource::new(root));
        let handle = server
            .load::<ModelAsset>(&file)
            .map_err(|e| format!("could not load `{file}`: {} ({:?})", e.message, e.kind))?;
        let bounds = server.get(handle).and_then(ModelAsset::bounds);
        (handle, bounds)
    };
    let (min, max) = bounds.ok_or("the model contains no triangles")?;
    let center = (min + max) * 0.5;
    let radius = ((max - min).length() * 0.5).max(1e-3);
    let camera = Camera3D {
        near: (radius * 0.02).max(1e-3),
        far: radius * 100.0,
        ..Camera3D::default()
    };
    let distance = radius / (camera.fov_y * 0.5).sin() * 1.1;
    app.insert_resource(Orbit {
        center,
        distance,
        height: distance * 0.35,
        angle: 0.6,
    });
    app.world.spawn((Transform3D::default(), camera));
    app.world.spawn((
        Transform3D {
            rotation: Quat::IDENTITY,
            ..Transform3D::default()
        },
        ModelInstance { model: handle },
    ));
    app.add_system(Stage::Update, orbit_system);
    app.build()?.run();
    Ok(())
}
