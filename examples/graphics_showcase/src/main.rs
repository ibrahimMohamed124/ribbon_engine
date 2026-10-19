//! Live showcase of the graphics features currently integrated into the engine.
//!
//! Run with `cargo run -p graphics_showcase` from the repository root.
//! Optional: `cargo run -p graphics_showcase -- --frames 600` exits after N frames.

use engine_app::{App, AppExit, Stage, Time, World};
use engine_assets::{AssetPlugin, AssetServer, FsSource};
use engine_events::WorldEventsExt;
use engine_platform::input::Key;
use engine_platform::{Input, PlatformConfig, PlatformPlugin};
use engine_plugins::AppPluginExt;
use engine_render::gpu_window::{ClearColor, GpuWindowPlugin};
use engine_scene::ScenePlugin;
use plugin_gltf::{GltfLoader, ModelAsset};
use plugin_render3d::glam::{Quat, Vec3};
use plugin_render3d::{
    BuiltinMesh, Camera3D, ExtractedScene3D, MeshInstance, ModelInstance, Render3dPlugin, SunLight,
    Transform3D,
};
use std::error::Error;
use std::path::Path;

const FLOOR_HALF_EXTENTS: Vec3 = Vec3::new(7.0, 0.075, 5.0);

#[derive(Clone, Copy)]
struct CubeMotion {
    base_position: Vec3,
    axis: Vec3,
    radians_per_second: f32,
    bob_amplitude: f32,
    phase: f32,
}

/// Keeps the model's local bounds center at a stable world-space pivot while it rotates.
#[derive(Clone, Copy)]
struct ModelMotion {
    world_center: Vec3,
    local_center: Vec3,
    radians_per_second: f32,
    phase: f32,
    tilt: f32,
}

struct ShowcaseState {
    orbit_angle: f32,
    orbit_speed: f32,
    camera_height: f32,
    paused: bool,
    frame_limit: Option<u64>,
    extraction_reported: bool,
}

fn spawn_cube(
    world: &mut World,
    position: Vec3,
    scale: Vec3,
    color: [f32; 3],
    motion: Option<CubeMotion>,
) {
    let transform = Transform3D {
        translation: position,
        scale,
        ..Transform3D::default()
    };
    let mesh = MeshInstance {
        mesh: BuiltinMesh::Cube,
        color,
    };
    if let Some(motion) = motion {
        world.spawn((transform, mesh, motion));
    } else {
        world.spawn((transform, mesh));
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_model(
    world: &mut World,
    model: engine_assets::Handle<ModelAsset>,
    local_center: Vec3,
    world_center: Vec3,
    scale: f32,
    radians_per_second: f32,
    phase: f32,
    tilt: f32,
) {
    // Transform3D is the outer transform. Offset it so the GLB's actual bounds
    // center (which may not be at the asset origin) stays on the desired pivot.
    let transform = Transform3D {
        translation: world_center - local_center * scale,
        scale: Vec3::splat(scale),
        ..Transform3D::default()
    };
    world.spawn((
        transform,
        ModelInstance { model },
        ModelMotion {
            world_center,
            local_center,
            radians_per_second,
            phase,
            tilt,
        },
    ));
}

fn animate_showcase(world: &mut World) {
    let delta = world.resource::<Time>().delta.as_secs_f32();
    let elapsed = world.resource::<Time>().elapsed.as_secs_f32();
    let frame = world.resource::<Time>().frame;
    let (space, left, right, up, down, reset) = {
        let input = world.resource::<Input>();
        (
            input.just_pressed(Key::Space),
            input.is_down(Key::Left),
            input.is_down(Key::Right),
            input.is_down(Key::Up),
            input.is_down(Key::Down),
            input.just_pressed(Key::R),
        )
    };

    let (orbit_angle, camera_height, paused, frame_limit) = {
        let state = world.resource_mut::<ShowcaseState>();
        if reset {
            state.orbit_angle = 0.35;
            state.orbit_speed = 0.24;
            state.camera_height = 5.0;
            state.paused = false;
        }
        if space {
            state.paused = !state.paused;
        }
        // Let the user change the orbit rate, including reversing its direction.
        let speed_delta = delta * 0.45;
        if left {
            state.orbit_speed -= speed_delta;
        }
        if right {
            state.orbit_speed += speed_delta;
        }
        state.orbit_speed = state.orbit_speed.clamp(-1.5, 1.5);
        if up {
            state.camera_height = (state.camera_height + delta * 2.0).clamp(1.5, 9.0);
        }
        if down {
            state.camera_height = (state.camera_height - delta * 2.0).clamp(1.5, 9.0);
        }
        if !state.paused {
            state.orbit_angle += delta * state.orbit_speed;
        }
        (
            state.orbit_angle,
            state.camera_height,
            state.paused,
            state.frame_limit,
        )
    };

    if !paused {
        for (_, (transform, motion)) in world.query_mut::<(&mut Transform3D, &CubeMotion)>() {
            if motion.radians_per_second.abs() > f32::EPSILON {
                let axis = motion.axis.normalize_or_zero();
                if axis != Vec3::ZERO {
                    transform.rotation =
                        Quat::from_axis_angle(axis, motion.radians_per_second * delta)
                            * transform.rotation;
                }
            }
            transform.translation = motion.base_position
                + Vec3::Y * (elapsed * 1.55 + motion.phase).sin() * motion.bob_amplitude;
        }
        for (_, (transform, motion)) in world.query_mut::<(&mut Transform3D, &ModelMotion)>() {
            let rotation =
                Quat::from_rotation_y(elapsed * motion.radians_per_second + motion.phase)
                    * Quat::from_rotation_x(motion.tilt);
            transform.rotation = rotation;
            transform.translation =
                motion.world_center - rotation * (transform.scale * motion.local_center);
        }

        // A gently moving directional light makes the mesh normals and depth visibly useful.
        let sun_phase = elapsed * 0.22;
        let sun = world.resource_mut::<SunLight>();
        sun.direction =
            Vec3::new(sun_phase.cos() * 0.55, -0.78, sun_phase.sin() * 0.55).normalize();
    }

    let target = Vec3::new(0.0, 0.75, 0.0);
    let eye = target
        + Vec3::new(
            orbit_angle.sin() * 16.0,
            camera_height,
            orbit_angle.cos() * 16.0,
        );
    for (_, (transform, _)) in world.query_mut::<(&mut Transform3D, &Camera3D)>() {
        *transform = Transform3D::looking_at(eye, target, Vec3::Y);
    }

    if frame_limit.is_some_and(|limit| frame >= limit) {
        world.send_event(AppExit);
    }
}

fn report_render_extraction(world: &mut World) {
    let (frame, has_camera, mesh_count, model_count) = {
        let extracted = world.resource::<ExtractedScene3D>();
        (
            extracted.frame,
            extracted.camera.is_some(),
            extracted.meshes.len(),
            extracted.models.len(),
        )
    };
    if frame == 0 {
        return;
    }
    let state = world.resource_mut::<ShowcaseState>();
    if state.extraction_reported {
        return;
    }
    state.extraction_reported = true;
    if has_camera && mesh_count > 0 && model_count > 0 {
        println!(
            "[RENDER EXTRACT] PASS: frame={frame}, camera=1, built-in meshes={mesh_count}, glTF placements={model_count}"
        );
    } else {
        println!(
            "[RENDER EXTRACT] FAIL: frame={frame}, camera={has_camera}, built-in meshes={mesh_count}, glTF placements={model_count}"
        );
    }
}

fn add_scene(
    world: &mut World,
    checker: engine_assets::Handle<ModelAsset>,
    checker_center: Vec3,
    triangle: engine_assets::Handle<ModelAsset>,
    triangle_center: Vec3,
) {
    // Ground slab and a subtle grid make perspective, scale, and camera movement easy to read.
    spawn_cube(
        world,
        Vec3::new(0.0, -FLOOR_HALF_EXTENTS.y, 0.0),
        FLOOR_HALF_EXTENTS,
        [0.105, 0.13, 0.18],
        None,
    );
    for i in -8..=8 {
        let x = i as f32 * 0.8;
        spawn_cube(
            world,
            Vec3::new(x, 0.006, 0.0),
            Vec3::new(0.008, 0.009, 9.8),
            [0.16, 0.20, 0.26],
            None,
        );
    }
    for i in -6..=6 {
        let z = i as f32 * 0.8;
        spawn_cube(
            world,
            Vec3::new(0.0, 0.006, z),
            Vec3::new(13.4, 0.009, 0.008),
            [0.16, 0.20, 0.26],
            None,
        );
    }

    // Central display plinth and three instances of the same textured GLB asset.
    spawn_cube(
        world,
        Vec3::new(0.0, 0.11, 0.0),
        Vec3::new(3.25, 0.22, 2.25),
        [0.18, 0.23, 0.30],
        None,
    );
    spawn_model(
        world,
        checker,
        checker_center,
        Vec3::new(0.0, 1.05, 0.0),
        1.75,
        0.35,
        0.0,
        0.12,
    );
    spawn_model(
        world,
        checker,
        checker_center,
        Vec3::new(-2.15, 0.82, -0.15),
        1.12,
        -0.52,
        1.3,
        0.20,
    );
    spawn_model(
        world,
        checker,
        checker_center,
        Vec3::new(2.15, 0.82, -0.15),
        1.12,
        0.58,
        2.4,
        -0.16,
    );

    // A separately imported untextured triangle demonstrates a minimal glTF primitive.
    spawn_cube(
        world,
        Vec3::new(5.2, 0.12, -0.9),
        Vec3::new(1.65, 0.24, 1.65),
        [0.20, 0.26, 0.33],
        None,
    );
    spawn_model(
        world,
        triangle,
        triangle_center,
        Vec3::new(5.2, 1.08, -0.9),
        1.65,
        0.42,
        0.8,
        0.16,
    );

    // A row of differently colored instanced cubes, with varied size and motion.
    let palette: [[f32; 3]; 7] = [
        [0.95, 0.26, 0.13],
        [0.98, 0.58, 0.16],
        [0.88, 0.83, 0.30],
        [0.20, 0.78, 0.57],
        [0.18, 0.62, 0.94],
        [0.42, 0.36, 0.95],
        [0.83, 0.30, 0.73],
    ];
    for (index, color) in palette.into_iter().enumerate() {
        let x = (index as f32 - 3.0) * 1.65;
        let size = 0.54 + (index % 3) as f32 * 0.12;
        spawn_cube(
            world,
            Vec3::new(x, 0.12, 3.45),
            Vec3::new(0.84, 0.16, 0.84),
            [0.15, 0.19, 0.25],
            None,
        );
        spawn_cube(
            world,
            Vec3::new(x, 0.48 + size * 0.5, 3.45),
            Vec3::splat(size),
            color,
            Some(CubeMotion {
                base_position: Vec3::new(x, 0.48 + size * 0.5, 3.45),
                axis: Vec3::new(0.25 + index as f32 * 0.11, 1.0, 0.2),
                radians_per_second: if index % 2 == 0 { 0.65 } else { -0.48 },
                bob_amplitude: 0.14 + (index % 3) as f32 * 0.035,
                phase: index as f32 * 0.7,
            }),
        );
    }

    // Staggered block columns give the scene more depth and catch the moving light.
    let column_colors = [
        [0.24, 0.40, 0.54],
        [0.28, 0.55, 0.61],
        [0.47, 0.43, 0.67],
        [0.63, 0.36, 0.42],
    ];
    for (column, x) in [-5.7_f32, -3.9, 3.9, 5.7].into_iter().enumerate() {
        let height = 2 + (column % 3);
        spawn_cube(
            world,
            Vec3::new(x, 0.10, -3.2),
            Vec3::new(1.05, 0.2, 1.05),
            [0.13, 0.17, 0.22],
            None,
        );
        for level in 0..height {
            let y = 0.515 + level as f32 * 0.67;
            spawn_cube(
                world,
                Vec3::new(x, y, -3.2),
                Vec3::splat(0.63),
                column_colors[(column + level) % column_colors.len()],
                Some(CubeMotion {
                    base_position: Vec3::new(x, y, -3.2),
                    axis: if level % 2 == 0 {
                        Vec3::Y
                    } else {
                        Vec3::new(1.0, 0.3, 0.0)
                    },
                    radians_per_second: if level % 2 == 0 { 0.18 } else { -0.24 },
                    bob_amplitude: 0.025,
                    phase: column as f32 + level as f32 * 0.4,
                }),
            );
        }
    }
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
            other => return Err(format!("unknown argument `{other}`").into()),
        }
    }

    let asset_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let mut app = App::new();
    app.add_plugin(PlatformPlugin)?;
    app.add_plugin(ScenePlugin)?;
    app.add_plugin(AssetPlugin)?;
    let (checker, checker_center, triangle, triangle_center) = {
        let server = app.world.resource_mut::<AssetServer>();
        server.register_loader(GltfLoader);
        server.add_source(0, FsSource::new(asset_root));
        let checker = server.load::<ModelAsset>("models/checker_cube.glb")?;
        let triangle = server.load::<ModelAsset>("models/bare_triangle.glb")?;
        let checker_center = server
            .get(checker)
            .and_then(ModelAsset::bounds)
            .map(|(min, max)| (min + max) * 0.5)
            .ok_or("checker_cube.glb contains no triangles")?;
        let triangle_center = server
            .get(triangle)
            .and_then(ModelAsset::bounds)
            .map(|(min, max)| (min + max) * 0.5)
            .ok_or("bare_triangle.glb contains no triangles")?;
        (checker, checker_center, triangle, triangle_center)
    };

    app.insert_resource(PlatformConfig {
        title: "Voxelworks | Graphics Showcase".to_owned(),
        width: 1280,
        height: 800,
    });
    app.insert_resource(ClearColor([0.018, 0.028, 0.048, 1.0]));
    app.insert_resource(ShowcaseState {
        orbit_angle: 0.35,
        orbit_speed: 0.24,
        camera_height: 5.0,
        paused: false,
        frame_limit,
        extraction_reported: false,
    });
    app.add_plugin(GpuWindowPlugin)?;
    app.insert_resource(SunLight {
        direction: Vec3::new(-0.45, -0.78, -0.45).normalize(),
        color: Vec3::new(1.0, 0.90, 0.78),
        ambient: Vec3::splat(0.20),
    });
    app.add_plugin(Render3dPlugin)?;

    app.world.spawn((
        Transform3D::looking_at(
            Vec3::new(16.0, 5.75, 0.0),
            Vec3::new(0.0, 0.75, 0.0),
            Vec3::Y,
        ),
        Camera3D {
            far: 150.0,
            ..Camera3D::default()
        },
    ));
    add_scene(
        &mut app.world,
        checker,
        checker_center,
        triangle,
        triangle_center,
    );
    app.add_system(Stage::Update, animate_showcase);
    app.add_system(Stage::Last, report_render_extraction);

    println!("Voxelworks Graphics Showcase");
    println!("Live features: native wgpu surface, ECS extraction, indexed meshes, depth buffer,");
    println!("Cook-Torrance metallic-roughness PBR, HDR tone mapping, directional shadows, and frustum culling.");
    println!(
        "Controls: Space pause/resume, Left/Right orbit speed, Up/Down camera height, R reset."
    );
    println!("Remaining: normal/occlusion/emissive maps, alpha modes, cascaded/soft shadows, AO, bloom, fog, IBL, and GPU culling.");
    println!(
        "Run `cargo run -p voxel_mesh_showcase` to view greedy voxels on the same mesh pipeline."
    );

    app.build()?.run();
    Ok(())
}
