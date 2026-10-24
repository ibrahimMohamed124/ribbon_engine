//! Live showcase of the graphics features currently integrated into the engine.
//!
//! Run with `cargo run -p graphics_showcase` from the repository root.
//! Optional: `cargo run -p graphics_showcase -- --frames 600` exits after N frames.
//! Optional: `cargo run -p graphics_showcase --release -- --frames 1800 --profile-csv target/profiles/graphics-showcase.csv`.

use engine_app::{App, AppExit, Stage, Time, World};
use engine_assets::{AssetPlugin, AssetRef, AssetServer, FsSource, LoadState};
use engine_events::WorldEventsExt;
use engine_platform::input::Key;
use engine_platform::{Input, PlatformConfig, PlatformPlugin};
use engine_plugins::AppPluginExt;
use engine_render::gpu_window::{ClearColor, GpuWindowPlugin};
use engine_render::profiling::RenderProfilePlugin;
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
    camera_height_max: f32,
    camera_distance: f32,
    paused: bool,
    frame_limit: Option<u64>,
    extraction_reported: bool,
    extraction_empty_frames: u32,
}

/// Owns strong leases for every distinct model used by the active stress scene. Optional low-
/// priority prefetch assets are deliberately left unleased so cache-budget eviction can reclaim them.
struct LargeSceneStress {
    model_paths: Vec<String>,
    grid_side: usize,
    cycle_frames: Option<u64>,
    leases: Vec<AssetRef<ModelAsset>>,
    active: bool,
    pending_activation: bool,
    next_cycle_frame: u64,
    generation: u64,
}

#[derive(Clone, Copy)]
struct StressPlacement {
    generation: u64,
}

const MAX_STRESS_GRID_SIDE: usize = 128;
const MAX_STRESS_MULTI_ASSET_COUNT: usize = 128;
const MAX_STRESS_CACHE_PREFETCH_COUNT: usize = 128;
const MAX_STRESS_CORPUS_COUNT: usize = 256;
const STRESS_GRID_SPACING: f32 = 2.6;
const ACTIVE_ASSET_PRIORITY: i32 = 100;
const BACKGROUND_ASSET_PRIORITY: i32 = -100;
const MAX_STRESS_EXTRACT_EMPTY_FRAMES: u32 = 8;

fn stress_asset_path(index: usize) -> String {
    format!("models/stress_{index:04}.glb")
}

fn spawn_large_scene_grid(
    world: &mut World,
    models: &[(engine_assets::Handle<ModelAsset>, Vec3)],
    side: usize,
    generation: u64,
) -> usize {
    if models.is_empty() {
        return 0;
    }
    let offset = (side.saturating_sub(1) as f32) * STRESS_GRID_SPACING * 0.5;
    for z_index in 0..side {
        for x_index in 0..side {
            let x = x_index as f32 * STRESS_GRID_SPACING - offset;
            let z = z_index as f32 * STRESS_GRID_SPACING - offset;
            let cell_index = z_index.saturating_mul(side).saturating_add(x_index);
            let (model, local_center) = models[cell_index % models.len()];
            let scale = 0.72 + ((x_index + z_index * 3) % 4) as f32 * 0.045;
            let rotation = Quat::from_rotation_y(
                ((x_index + z_index) % 8) as f32 * std::f32::consts::FRAC_PI_8,
            );
            let center = Vec3::new(x, 0.55, z);
            world.spawn((
                Transform3D {
                    translation: center - rotation * (local_center * scale),
                    rotation,
                    scale: Vec3::splat(scale),
                },
                ModelInstance { model },
                StressPlacement { generation },
            ));
        }
    }
    side.saturating_mul(side)
}

fn despawn_large_scene_grid(world: &mut World, generation: u64) -> usize {
    let entities: Vec<_> = world
        .query::<&StressPlacement>()
        .iter()
        .filter(|(_, placement)| placement.generation == generation)
        .map(|(entity, _)| entity)
        .collect();
    let count = entities.len();
    for entity in entities {
        let _ = world.despawn(entity);
    }
    count
}

fn request_stress_leases(
    world: &mut World,
    paths: &[String],
) -> Result<Vec<AssetRef<ModelAsset>>, String> {
    let server = world.resource_mut::<AssetServer>();
    let mut leases = Vec::with_capacity(paths.len());
    for path in paths {
        let lease = if matches!(server.state::<ModelAsset>(path), Some(LoadState::Failed(_))) {
            let handle = server
                .retry_with_priority::<ModelAsset>(path, ACTIVE_ASSET_PRIORITY)
                .map_err(|error| format!("could not retry stress model `{path}`: {error}"))?;
            server.strong_ref(handle).ok_or_else(|| {
                format!("could not pin the retry handle for stress model `{path}`")
            })?
        } else {
            server
                .request_ref_with_priority::<ModelAsset>(path, ACTIVE_ASSET_PRIORITY)
                .map_err(|error| format!("could not request stress model `{path}`: {error}"))?
        };
        leases.push(lease);
    }
    Ok(leases)
}

fn update_large_scene_stress(world: &mut World) {
    let frame = world.resource::<Time>().frame;
    let pending = world.try_resource::<LargeSceneStress>().and_then(|stress| {
        stress.pending_activation.then(|| {
            (
                stress
                    .leases
                    .iter()
                    .map(AssetRef::handle)
                    .collect::<Vec<_>>(),
                stress.model_paths.clone(),
                stress.grid_side,
                stress.generation,
                stress.cycle_frames,
            )
        })
    });

    if let Some((handles, paths, side, generation, cycle_frames)) = pending {
        if handles.is_empty() || handles.len() != paths.len() {
            let leases = {
                let stress = world.resource_mut::<LargeSceneStress>();
                stress.pending_activation = false;
                stress.next_cycle_frame = cycle_frames
                    .map(|interval| frame.saturating_add(interval))
                    .unwrap_or(u64::MAX);
                std::mem::take(&mut stress.leases)
            };
            drop(leases);
            eprintln!(
                "[ASSET STRESS] invalid pending activation: active asset paths and leases differ"
            );
            return;
        }

        enum ActivationState {
            Pending,
            Failed(String),
            Ready(Vec<(engine_assets::Handle<ModelAsset>, Vec3)>),
        }
        let activation = {
            let server = world.resource::<AssetServer>();
            let mut loaded_models = Vec::with_capacity(handles.len());
            let mut waiting_for_assets = false;
            let mut failure = None;
            for (handle, path) in handles.iter().copied().zip(&paths) {
                if let Some(model) = server.get(handle) {
                    match model.bounds() {
                        Some((min, max)) => loaded_models.push((handle, (min + max) * 0.5)),
                        None => {
                            failure = Some(format!("stress model `{path}` contains no triangles"))
                        }
                    }
                } else if let Some(LoadState::Failed(error)) = server.state::<ModelAsset>(path) {
                    failure = Some(format!("stress model `{path}` failed to load: {error}"));
                } else {
                    waiting_for_assets = true;
                }
            }
            if let Some(error) = failure {
                ActivationState::Failed(error)
            } else if waiting_for_assets || loaded_models.len() != handles.len() {
                ActivationState::Pending
            } else {
                ActivationState::Ready(loaded_models)
            }
        };

        match activation {
            ActivationState::Ready(models) => {
                let count = spawn_large_scene_grid(world, &models, side, generation);
                {
                    let stress = world.resource_mut::<LargeSceneStress>();
                    stress.active = true;
                    stress.pending_activation = false;
                    stress.next_cycle_frame = cycle_frames
                        .map(|interval| frame.saturating_add(interval))
                        .unwrap_or(u64::MAX);
                }
                let identities: Vec<_> = models
                    .iter()
                    .map(|(handle, _)| (handle.id(), handle.generation()))
                    .collect();
                println!(
                    "[ASSET STRESS] activated grid_generation={generation}; active_assets={}; model_asset_id/generations={identities:?}; placements={count}; CPU model leases restored",
                    models.len()
                );
                let cache_stats = world.resource::<AssetServer>().cpu_memory_stats();
                println!(
                    "[ASSET STRESS] CPU cache at activation: resident={} bytes; pinned={} bytes; unpinned={} bytes; budget={:?}; unpinned_over_budget={} bytes; budget_evictions={}",
                    cache_stats.resident_bytes_estimate,
                    cache_stats.pinned_bytes_estimate,
                    cache_stats.unpinned_bytes_estimate,
                    cache_stats.budget_bytes,
                    cache_stats.over_budget_bytes,
                    cache_stats.budget_evictions_cumulative
                );
                return;
            }
            ActivationState::Pending => return,
            ActivationState::Failed(error) => {
                let leases = {
                    let stress = world.resource_mut::<LargeSceneStress>();
                    stress.pending_activation = false;
                    stress.next_cycle_frame = cycle_frames
                        .map(|interval| frame.saturating_add(interval))
                        .unwrap_or(u64::MAX);
                    std::mem::take(&mut stress.leases)
                };
                drop(leases);
                let memory_budget_enabled = world
                    .resource::<AssetServer>()
                    .cpu_memory_stats()
                    .budget_bytes
                    .is_some();
                let (evicted, stats) = {
                    let server = world.resource_mut::<AssetServer>();
                    let evicted = if memory_budget_enabled {
                        server.enforce_cpu_memory_budget()
                    } else {
                        server.evict_unused()
                    };
                    (evicted, server.cpu_memory_stats())
                };
                eprintln!(
                    "[ASSET STRESS] activation failed: {error}; evicted={evicted}; resident_cpu_bytes={}; unpinned_cpu_bytes={}; budget_bytes={:?}",
                    stats.resident_bytes_estimate,
                    stats.unpinned_bytes_estimate,
                    stats.budget_bytes
                );
                return;
            }
        }
    }

    let should_toggle = world
        .try_resource::<LargeSceneStress>()
        .is_some_and(|stress| {
            stress.cycle_frames.is_some()
                && !stress.pending_activation
                && frame >= stress.next_cycle_frame
        });
    if !should_toggle {
        return;
    }

    let active = world.resource::<LargeSceneStress>().active;
    if active {
        let generation = world.resource::<LargeSceneStress>().generation;
        let despawned = despawn_large_scene_grid(world, generation);
        let leases = {
            let stress = world.resource_mut::<LargeSceneStress>();
            stress.active = false;
            stress.pending_activation = false;
            stress.generation = stress.generation.saturating_add(1);
            stress.next_cycle_frame = stress
                .cycle_frames
                .map(|interval| frame.saturating_add(interval))
                .unwrap_or(u64::MAX);
            std::mem::take(&mut stress.leases)
        };
        let asset_identities: Vec<_> = leases
            .iter()
            .map(|lease| (lease.asset_id().get(), lease.generation()))
            .collect();
        drop(leases);
        let (evicted, stats) = {
            let server = world.resource_mut::<AssetServer>();
            let evicted = if server.cpu_memory_stats().budget_bytes.is_some() {
                server.enforce_cpu_memory_budget()
            } else {
                server.evict_unused()
            };
            (evicted, server.cpu_memory_stats())
        };
        println!(
            "[ASSET STRESS] deactivated grid_generation={generation}; assets={asset_identities:?}; despawned={despawned}; CPU cache evictions={evicted}; resident_cpu_bytes={}; pinned_cpu_bytes={}; unpinned_cpu_bytes={}; budget_bytes={:?}; budget_evictions_cumulative={}",
            stats.resident_bytes_estimate,
            stats.pinned_bytes_estimate,
            stats.unpinned_bytes_estimate,
            stats.budget_bytes,
            stats.budget_evictions_cumulative
        );
        return;
    }

    let paths = world.resource::<LargeSceneStress>().model_paths.clone();
    match request_stress_leases(world, &paths) {
        Ok(leases) => {
            let generation = {
                let stress = world.resource_mut::<LargeSceneStress>();
                stress.leases = leases;
                stress.pending_activation = true;
                stress.next_cycle_frame = u64::MAX;
                stress.generation
            };
            println!(
                "[ASSET STRESS] reload requested for generation={generation}; active_assets={}; waiting for worker decode",
                paths.len()
            );
        }
        Err(error) => {
            eprintln!("[ASSET STRESS] {error}");
            let stress = world.resource_mut::<LargeSceneStress>();
            stress.next_cycle_frame = stress
                .cycle_frames
                .map(|interval| frame.saturating_add(interval))
                .unwrap_or(u64::MAX);
        }
    }
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

    let (orbit_angle, camera_height, camera_distance, paused, frame_limit) = {
        let state = world.resource_mut::<ShowcaseState>();
        if reset {
            state.orbit_angle = 0.35;
            state.orbit_speed = 0.24;
            state.camera_height =
                (state.camera_distance * 0.31).clamp(5.0, state.camera_height_max);
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
            state.camera_height =
                (state.camera_height + delta * 2.0).clamp(1.5, state.camera_height_max);
        }
        if down {
            state.camera_height =
                (state.camera_height - delta * 2.0).clamp(1.5, state.camera_height_max);
        }
        if !state.paused {
            state.orbit_angle += delta * state.orbit_speed;
        }
        (
            state.orbit_angle,
            state.camera_height,
            state.camera_distance,
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
            orbit_angle.sin() * camera_distance,
            camera_height,
            orbit_angle.cos() * camera_distance,
        );
    for (_, (transform, _)) in world.query_mut::<(&mut Transform3D, &Camera3D)>() {
        *transform = Transform3D::looking_at(eye, target, Vec3::Y);
    }

    if frame_limit.is_some_and(|limit| frame >= limit) {
        world.send_event(AppExit);
    }
}

fn report_render_extraction(world: &mut World) {
    let (frame, has_camera, mesh_count, model_count, voxel_count) = {
        let extracted = world.resource::<ExtractedScene3D>();
        (
            extracted.frame,
            extracted.camera.is_some(),
            extracted.meshes.len(),
            extracted.models.len(),
            extracted.voxels.len(),
        )
    };
    if frame == 0 {
        return;
    }

    let renderable_count = mesh_count + model_count + voxel_count;
    let (is_stress_scene, activation_pending, stress_active) = world
        .try_resource::<LargeSceneStress>()
        .map(|stress| {
            (
                stress.grid_side > 0,
                stress.pending_activation,
                stress.active,
            )
        })
        .unwrap_or((false, false, false));

    // Multi-asset stress startup is intentionally asynchronous. Do not report the expected empty
    // extraction before model decode as a renderer failure; wait for activation and a few frames
    // for the extraction stage to observe newly spawned placements. Activation failures are
    // reported by update_large_scene_stress and still result in a final extraction failure.
    if renderable_count == 0 && is_stress_scene && (activation_pending || stress_active) {
        if activation_pending {
            return;
        }
        let state = world.resource_mut::<ShowcaseState>();
        if state.extraction_reported {
            return;
        }
        state.extraction_empty_frames = state.extraction_empty_frames.saturating_add(1);
        if state.extraction_empty_frames < MAX_STRESS_EXTRACT_EMPTY_FRAMES {
            return;
        }
    }

    let state = world.resource_mut::<ShowcaseState>();
    if state.extraction_reported {
        return;
    }
    state.extraction_reported = true;
    if has_camera && renderable_count > 0 {
        println!(
            "[RENDER EXTRACT] PASS: frame={frame}, camera=1, built-in meshes={mesh_count}, glTF placements={model_count}, voxel placements={voxel_count}"
        );
    } else {
        println!(
            "[RENDER EXTRACT] FAIL: frame={frame}, camera={has_camera}, built-in meshes={mesh_count}, glTF placements={model_count}, voxel placements={voxel_count}"
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
    let mut profile_csv: Option<String> = None;
    let mut profile_warmup = 120_u64;
    let mut stress_grid_side = 0_usize;
    let mut stress_cycle_frames = None;
    let mut stress_multi_asset_count = 0_usize;
    let mut stress_cache_prefetch_count = 0_usize;
    let mut asset_memory_budget_mib: Option<u64> = None;
    let mut profile_static_scene = false;
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
            "--profile-csv" => {
                profile_csv = Some(args.next().ok_or("--profile-csv expects a path")?);
            }
            "--profile-warmup" => {
                profile_warmup = args
                    .next()
                    .ok_or("--profile-warmup expects a number")?
                    .parse::<u64>()?;
            }
            "--profile-static-scene" => profile_static_scene = true,
            "--stress-grid" => {
                stress_grid_side = args
                    .next()
                    .ok_or("--stress-grid expects a grid side integer")?
                    .parse::<usize>()?;
            }
            "--stress-cycle-frames" => {
                stress_cycle_frames = Some(
                    args.next()
                        .ok_or("--stress-cycle-frames expects a positive frame count")?
                        .parse::<u64>()?,
                );
            }
            "--stress-multi-asset-count" => {
                stress_multi_asset_count = args
                    .next()
                    .ok_or("--stress-multi-asset-count expects an integer")?
                    .parse::<usize>()?;
            }
            "--stress-cache-prefetch-count" => {
                stress_cache_prefetch_count = args
                    .next()
                    .ok_or("--stress-cache-prefetch-count expects an integer")?
                    .parse::<usize>()?;
            }
            "--asset-memory-budget-mib" => {
                asset_memory_budget_mib = Some(
                    args.next()
                        .ok_or("--asset-memory-budget-mib expects a positive MiB value")?
                        .parse::<u64>()?,
                );
            }
            "--help" | "-h" => {
                println!(
                    "graphics_showcase\n\nOptions:\n  --frames N                  Exit after N frames\n  --profile-csv PATH          Write a render profile CSV at shutdown\n  --profile-warmup N          Frames excluded before capture (default: 120)\n  --profile-static-scene      Keep stress-scene camera fixed during capture\n  --stress-grid SIDE          Render SIDE x SIDE stress placements (0 disables; max 128)\n  --stress-cycle-frames N     Despawn/reload the active stress scene every N frames\n  --stress-multi-asset-count N Use N distinct generated GLB assets in the active scene (max 128)\n  --stress-cache-prefetch-count N Request N extra low-priority, unleased assets (max 128)\n  --asset-memory-budget-mib N Set the soft decoded-CPU cache budget for unpinned assets\n  -h, --help                  Show this help\n\nStress examples:\n  --stress-grid 32 --frames 1800 --profile-warmup 60 --profile-csv target/profiles/large-scene.csv\n  --stress-grid 48 --stress-multi-asset-count 32 --stress-cache-prefetch-count 32 --asset-memory-budget-mib 1 --frames 1800 --profile-warmup 60 --profile-csv target/profiles/multi-asset.csv\n  --stress-grid 24 --stress-multi-asset-count 16 --stress-cache-prefetch-count 48 --asset-memory-budget-mib 1 --stress-cycle-frames 300 --frames 1950 --profile-warmup 60 --profile-csv target/profiles/multi-asset-lifecycle.csv"
                );
                return Ok(());
            }
            other => return Err(format!("unknown argument `{other}`").into()),
        }
    }
    if stress_grid_side > MAX_STRESS_GRID_SIDE {
        return Err(format!(
            "--stress-grid side must be between 0 (disabled) and {MAX_STRESS_GRID_SIDE}"
        )
        .into());
    }
    if stress_multi_asset_count > MAX_STRESS_MULTI_ASSET_COUNT {
        return Err(format!(
            "--stress-multi-asset-count must be between 0 and {MAX_STRESS_MULTI_ASSET_COUNT}"
        )
        .into());
    }
    if stress_cache_prefetch_count > MAX_STRESS_CACHE_PREFETCH_COUNT {
        return Err(format!(
            "--stress-cache-prefetch-count must be between 0 and {MAX_STRESS_CACHE_PREFETCH_COUNT}"
        )
        .into());
    }
    if stress_multi_asset_count.saturating_add(stress_cache_prefetch_count)
        > MAX_STRESS_CORPUS_COUNT
    {
        return Err(format!(
            "active plus prefetch assets must not exceed {MAX_STRESS_CORPUS_COUNT}"
        )
        .into());
    }
    if stress_multi_asset_count > 0 && stress_grid_side == 0 {
        return Err("--stress-multi-asset-count requires --stress-grid SIDE".into());
    }
    if stress_cache_prefetch_count > 0 && stress_multi_asset_count == 0 {
        return Err("--stress-cache-prefetch-count requires --stress-multi-asset-count N".into());
    }
    if asset_memory_budget_mib == Some(0) {
        return Err("--asset-memory-budget-mib must be greater than zero".into());
    }
    if asset_memory_budget_mib.is_some() && stress_multi_asset_count == 0 {
        return Err("--asset-memory-budget-mib requires --stress-multi-asset-count N".into());
    }
    if stress_cycle_frames == Some(0) {
        return Err("--stress-cycle-frames must be greater than zero".into());
    }
    if stress_cycle_frames.is_some() && stress_grid_side == 0 {
        return Err("--stress-cycle-frames requires --stress-grid SIDE".into());
    }

    let asset_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let stress_asset_root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/asset-stress-corpus");
    let mut app = App::new();
    app.add_plugin(PlatformPlugin)?;
    app.add_plugin(ScenePlugin)?;
    app.add_plugin(AssetPlugin)?;
    let (checker, checker_center, optional_triangle) = {
        let server = app.world.resource_mut::<AssetServer>();
        server.register_loader(GltfLoader);
        if stress_multi_asset_count > 0 {
            // Higher source priority makes generated stress assets override any accidental same-name
            // path in the regular content source without modifying checked-in game assets.
            server.add_source(10, FsSource::new(stress_asset_root));
        }
        server.add_source(0, FsSource::new(asset_root));
        let checker = server.load::<ModelAsset>("models/checker_cube.glb")?;
        let checker_center = server
            .get(checker)
            .and_then(ModelAsset::bounds)
            .map(|(min, max)| (min + max) * 0.5)
            .ok_or("checker_cube.glb contains no triangles")?;
        let triangle = if stress_grid_side == 0 {
            let handle = server.load::<ModelAsset>("models/bare_triangle.glb")?;
            let center = server
                .get(handle)
                .and_then(ModelAsset::bounds)
                .map(|(min, max)| (min + max) * 0.5)
                .ok_or("bare_triangle.glb contains no triangles")?;
            Some((handle, center))
        } else {
            None
        };
        (checker, checker_center, triangle)
    };

    let mut stress_model_paths = Vec::<String>::new();
    let mut stress_leases = Vec::<AssetRef<ModelAsset>>::new();
    let mut initial_stress_models: Option<Vec<(engine_assets::Handle<ModelAsset>, Vec3)>> = None;
    let mut stress_active = false;
    let mut stress_pending_activation = false;
    if stress_grid_side > 0 {
        let server = app.world.resource_mut::<AssetServer>();
        if stress_multi_asset_count > 0 {
            let budget_bytes = asset_memory_budget_mib.map(|mib| mib.saturating_mul(1024 * 1024));
            server.set_auto_evict_unused(false);
            server.set_cpu_memory_budget_bytes(budget_bytes);
            for index in 0..stress_multi_asset_count {
                stress_model_paths.push(stress_asset_path(index));
            }
            for path in &stress_model_paths {
                stress_leases.push(
                    server.request_ref_with_priority::<ModelAsset>(path, ACTIVE_ASSET_PRIORITY)?,
                );
            }
            let prefetch_start = stress_multi_asset_count;
            let prefetch_end = prefetch_start.saturating_add(stress_cache_prefetch_count);
            for index in prefetch_start..prefetch_end {
                let path = stress_asset_path(index);
                server.request_with_priority::<ModelAsset>(&path, BACKGROUND_ASSET_PRIORITY)?;
            }
            stress_pending_activation = true;
            server.evict_unused();
        } else {
            let lease = server
                .strong_ref(checker)
                .ok_or("could not pin the bundled stress model")?;
            server.set_auto_evict_unused(true);
            stress_model_paths.push("models/checker_cube.glb".to_owned());
            stress_leases.push(lease);
            initial_stress_models = Some(vec![(checker, checker_center)]);
            stress_active = true;
        }
    }

    let camera_distance = if stress_grid_side > 0 {
        (stress_grid_side as f32 * STRESS_GRID_SPACING * 1.15).max(16.0)
    } else {
        16.0
    };
    let camera_height = if stress_grid_side > 0 {
        (camera_distance * 0.42).max(5.0)
    } else {
        5.0
    };
    let camera_height_max = if stress_grid_side > 0 {
        (camera_distance * 0.85).max(9.0)
    } else {
        9.0
    };

    app.insert_resource(PlatformConfig {
        title: "Voxelworks | Graphics Showcase".to_owned(),
        width: 1280,
        height: 800,
    });
    app.insert_resource(ClearColor([0.018, 0.028, 0.048, 1.0]));
    app.insert_resource(ShowcaseState {
        orbit_angle: 0.35,
        orbit_speed: if profile_static_scene { 0.0 } else { 0.24 },
        camera_height,
        camera_height_max,
        camera_distance,
        paused: profile_static_scene,
        frame_limit,
        extraction_reported: false,
        extraction_empty_frames: 0,
    });
    if profile_static_scene {
        println!(
            "[PROFILE] static scene enabled; camera, model transforms, and lighting are paused"
        );
    }
    app.add_plugin(GpuWindowPlugin)?;
    app.insert_resource(SunLight {
        direction: Vec3::new(-0.45, -0.78, -0.45).normalize(),
        color: Vec3::new(1.0, 0.90, 0.78),
        ambient: Vec3::splat(0.20),
    });
    app.add_plugin(Render3dPlugin)?;
    if let Some(path) = profile_csv {
        app.add_plugin(RenderProfilePlugin::new(path).warmup_frames(profile_warmup))?;
    }

    app.world.spawn((
        Transform3D::looking_at(
            Vec3::new(camera_distance, camera_height + 0.75, 0.0),
            Vec3::new(0.0, 0.75, 0.0),
            Vec3::Y,
        ),
        Camera3D {
            far: (camera_distance * 6.0).max(150.0),
            ..Camera3D::default()
        },
    ));
    if stress_grid_side > 0 {
        let generation = 1;
        let count = initial_stress_models
            .as_deref()
            .map(|models| {
                spawn_large_scene_grid(&mut app.world, models, stress_grid_side, generation)
            })
            .unwrap_or(0);
        let asset_identities: Vec<_> = stress_leases
            .iter()
            .map(|lease| (lease.asset_id().get(), lease.generation()))
            .collect();
        let initial_active_assets = stress_leases.len();
        let cache_stats = app.world.resource::<AssetServer>().cpu_memory_stats();
        app.insert_resource(LargeSceneStress {
            model_paths: stress_model_paths,
            grid_side: stress_grid_side,
            cycle_frames: stress_cycle_frames,
            leases: stress_leases,
            active: stress_active,
            pending_activation: stress_pending_activation,
            next_cycle_frame: if stress_active {
                stress_cycle_frames.unwrap_or(u64::MAX)
            } else {
                u64::MAX
            },
            generation,
        });
        if stress_multi_asset_count > 0 {
            println!(
                "[ASSET STRESS] started mode=multi-asset grid={stress_grid_side}x{stress_grid_side}; placements={}; active_assets={stress_multi_asset_count}; low_priority_prefetch={stress_cache_prefetch_count}; activation=waiting_for_decode; cpu_cache_budget_bytes={:?}; initial_resident_bytes={};",
                stress_grid_side.saturating_mul(stress_grid_side),
                cache_stats.budget_bytes,
                cache_stats.resident_bytes_estimate,
            );
        } else {
            println!(
                "[ASSET STRESS] started grid={stress_grid_side}x{stress_grid_side}; placements={count}; model_asset_id/generations={asset_identities:?}; CPU auto-eviction=enabled; cycle_frames={stress_cycle_frames:?}; initial_active_assets={initial_active_assets}"
            );
        }
    } else if let Some((triangle, triangle_center)) = optional_triangle {
        add_scene(
            &mut app.world,
            checker,
            checker_center,
            triangle,
            triangle_center,
        );
    } else {
        return Err("default showcase triangle was not initialized".into());
    }
    app.add_system(Stage::Update, animate_showcase);
    if stress_grid_side > 0 {
        app.add_system(Stage::Update, update_large_scene_stress);
    }
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
