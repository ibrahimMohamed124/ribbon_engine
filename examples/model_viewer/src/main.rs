//! Loads a glTF model and orbits a camera around it.
//!
//! Usage: `cargo run -p model_viewer [-- path/to/model.glb] [--frames N]`.
//! The bundled textured cube is shown when no path is supplied. Source IO and glTF CPU decoding
//! run asynchronously; GPU resources are uploaded later by the render pass. Only `.glb` files (or
//! `.gltf` files with embedded data) are supported.
//! Optional: `cargo run -p model_viewer --release -- --frames 1800 --profile-csv target/profiles/model-viewer.csv`.

use engine_app::{App, AppExit, Stage, Time, World};
use engine_assets::{AssetPlugin, AssetRef, AssetServer, FsSource, LoadState};
use engine_events::WorldEventsExt;
use engine_platform::{PlatformConfig, PlatformPlugin};
use engine_plugins::AppPluginExt;
use engine_render::gpu_window::{ClearColor, GpuWindowPlugin};
use engine_render::profiling::RenderProfilePlugin;
use engine_scene::ScenePlugin;
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

/// Keeps the requested model pinned while its asynchronous load is pending and while it is shown.
struct PendingModel {
    lease: AssetRef<ModelAsset>,
    path: String,
    fitted: bool,
    error_reported: bool,
}

fn fit_camera_when_ready(world: &mut World) {
    let (handle, path, fitted, error_reported) = {
        let pending = world.resource::<PendingModel>();
        (
            pending.lease.handle(),
            pending.path.clone(),
            pending.fitted,
            pending.error_reported,
        )
    };
    if fitted {
        return;
    }

    let bounds = world
        .resource::<AssetServer>()
        .get(handle)
        .and_then(ModelAsset::bounds);
    if let Some((min, max)) = bounds {
        let center = (min + max) * 0.5;
        let radius = ((max - min).length() * 0.5).max(1e-3);
        let camera = Camera3D {
            near: (radius * 0.02).max(1e-3),
            far: radius * 100.0,
            ..Camera3D::default()
        };
        let distance = radius / (camera.fov_y * 0.5).sin() * 1.1;
        {
            let orbit = world.resource_mut::<Orbit>();
            orbit.center = center;
            orbit.distance = distance;
            orbit.height = distance * 0.35;
        }
        {
            let eye = center
                + Vec3::new(
                    0.6_f32.sin() * distance,
                    distance * 0.35,
                    0.6_f32.cos() * distance,
                );
            for (_, (pose, camera_component)) in
                world.query_mut::<(&mut Transform3D, &mut Camera3D)>()
            {
                *pose = Transform3D::looking_at(eye, center, Vec3::Y);
                *camera_component = camera;
            }
        }
        world.resource_mut::<PendingModel>().fitted = true;
        println!("loaded `{path}` asynchronously; camera fitted to model bounds");
        return;
    }

    let state = world
        .resource::<AssetServer>()
        .state::<ModelAsset>(&path)
        .cloned();
    let message = match state {
        Some(LoadState::Failed(error)) => Some(format!("could not load `{path}`: {error}")),
        Some(LoadState::Loaded) => Some(format!("model `{path}` contains no triangles")),
        _ => None,
    };
    if !error_reported {
        if let Some(message) = message {
            eprintln!("{message}");
            world.resource_mut::<PendingModel>().error_reported = true;
        }
    }
}

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
    let mut profile_csv: Option<String> = None;
    let mut profile_warmup = 120_u64;
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
            "--profile-csv" => {
                profile_csv = Some(args.next().ok_or("--profile-csv expects a path")?);
            }
            "--profile-warmup" => {
                profile_warmup = args
                    .next()
                    .ok_or("--profile-warmup expects a number")?
                    .parse::<u64>()?;
            }
            "--help" | "-h" => {
                println!(
                    "model_viewer\n\nOptions:\n  [MODEL.glb]            Optional model path\n  --frames N             Exit after N frames\n  --profile-csv PATH     Write a render profile CSV at shutdown\n  --profile-warmup N     Frames excluded before capture (default: 120)\n  -h, --help             Show this help"
                );
                return Ok(());
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
    app.add_plugin(ScenePlugin)?;
    app.add_plugin(AssetPlugin)?;
    app.add_plugin(GpuWindowPlugin)?;
    app.add_plugin(Render3dPlugin)?;
    if let Some(path) = profile_csv {
        app.add_plugin(RenderProfilePlugin::new(path).warmup_frames(profile_warmup))?;
    }

    let lease = {
        let server = app.world.resource_mut::<AssetServer>();
        server.register_loader(GltfLoader);
        server.add_source(0, FsSource::new(root));
        server
            .request_ref::<ModelAsset>(&file)
            .map_err(|e| format!("could not queue `{file}`: {} ({:?})", e.message, e.kind))?
    };
    let handle = lease.handle();
    app.insert_resource(PendingModel {
        lease,
        path: file.clone(),
        fitted: false,
        error_reported: false,
    });
    app.insert_resource(Orbit {
        center: Vec3::ZERO,
        distance: 4.0,
        height: 1.4,
        angle: 0.6,
    });
    app.world.spawn((
        Transform3D::looking_at(Vec3::new(0.0, 1.2, 4.0), Vec3::ZERO, Vec3::Y),
        Camera3D {
            near: 0.05,
            far: 1_000.0,
            ..Camera3D::default()
        },
    ));
    app.world.spawn((
        Transform3D {
            rotation: Quat::IDENTITY,
            ..Transform3D::default()
        },
        ModelInstance { model: handle },
    ));
    app.add_system(Stage::First, fit_camera_when_ready);
    app.add_system(Stage::Update, orbit_system);
    app.build()?.run();
    Ok(())
}
