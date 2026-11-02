//! Live showcase of the graphics features currently integrated into the engine.
//!
//! Run with `cargo run -p graphics_showcase` from the repository root.
//! Optional: `cargo run -p graphics_showcase -- --frames 600` exits after N frames.
//! Optional: `cargo run -p graphics_showcase --release -- --frames 1800 --profile-csv target/profiles/graphics-showcase.csv`.

use ribbon_app::{App, AppExit, Stage, Time, World};
use ribbon_assets::{AssetPlugin, AssetRef, AssetServer, FsSource, LoadState};
use ribbon_events::WorldEventsExt;
use ribbon_platform::input::Key;
use ribbon_platform::{Input, PlatformConfig, PlatformPlugin};
use ribbon_plugin_gltf::{GltfLoader, ModelAsset};
use ribbon_plugin_render3d::glam::{Quat, Vec3};
use ribbon_plugin_render3d::{
    AntiAliasingMode, AntiAliasingSettings, BuiltinMesh, Camera3D, ExtractedScene3D, MeshInstance,
    ModelInstance, PostProcessingSettings, Render3dPlugin, SunLight, Transform3D,
};
use ribbon_plugins::AppPluginExt;
use ribbon_render::gpu_window::{ClearColor, GpuWindowPlugin};
use ribbon_render::profiling::RenderProfilePlugin;
use ribbon_scene::ScenePlugin;
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

fn parse_anti_aliasing_mode(value: &str) -> Result<AntiAliasingMode, String> {
    match value.to_ascii_lowercase().as_str() {
        "none" => Ok(AntiAliasingMode::None),
        "msaa2" | "2x" => Ok(AntiAliasingMode::Msaa2x),
        "msaa4" | "4x" => Ok(AntiAliasingMode::Msaa4x),
        "msaa8" | "8x" => Ok(AntiAliasingMode::Msaa8x),
        "fxaa" => Ok(AntiAliasingMode::Fxaa),
        _ => Err(format!(
            "unknown anti-aliasing mode `{value}`; expected none, msaa2, msaa4, msaa8, or fxaa"
        )),
    }
}

fn parse_post_processing_preset(value: &str) -> Result<PostProcessingSettings, String> {
    match value.to_ascii_lowercase().as_str() {
        "neutral" | "none" => Ok(PostProcessingSettings::default()),
        "cinematic" => Ok(PostProcessingSettings::cinematic()),
        _ => Err(format!(
            "unknown post-processing preset `{value}`; expected neutral or cinematic"
        )),
    }
}

mod scene;
mod setup;
mod stress;
use scene::{animate_showcase, report_render_extraction};
use setup::add_scene;
use stress::{spawn_large_scene_grid, stress_asset_path, update_large_scene_stress};

fn main() -> Result<(), Box<dyn Error>> {
    let mut frame_limit = None;
    let mut anti_aliasing_mode = AntiAliasingMode::None;
    let mut post_processing_settings = PostProcessingSettings::default();
    let mut post_processing_preset = "neutral".to_owned();
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
            "--aa" => {
                anti_aliasing_mode =
                    parse_anti_aliasing_mode(&args.next().ok_or("--aa expects a mode")?)?;
            }
            "--post-preset" => {
                post_processing_preset = args.next().ok_or("--post-preset expects a preset")?;
                post_processing_settings = parse_post_processing_preset(&post_processing_preset)?;
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
                    "graphics_showcase\n\nOptions:\n  --frames N                  Exit after N frames\n  --aa MODE                   none | msaa2 | msaa4 | msaa8 | fxaa (default: none)\n  --post-preset PRESET        neutral | cinematic (default: neutral)\n  --profile-csv PATH          Write a render profile CSV at shutdown\n  --profile-warmup N          Frames excluded before capture (default: 120)\n  --profile-static-scene      Keep stress-scene camera fixed during capture\n  --stress-grid SIDE          Render SIDE x SIDE stress placements (0 disables; max 128)\n  --stress-cycle-frames N     Despawn/reload the active stress scene every N frames\n  --stress-multi-asset-count N Use N distinct generated GLB assets in the active scene (max 128)\n  --stress-cache-prefetch-count N Request N extra low-priority, unleased assets (max 128)\n  --asset-memory-budget-mib N Set the soft decoded-CPU cache budget for unpinned assets\n  -h, --help                  Show this help\n\nStress examples:\n  --stress-grid 32 --frames 1800 --profile-warmup 60 --profile-csv target/profiles/large-scene.csv\n  --stress-grid 48 --stress-multi-asset-count 32 --stress-cache-prefetch-count 32 --asset-memory-budget-mib 1 --frames 1800 --profile-warmup 60 --profile-csv target/profiles/multi-asset.csv\n  --stress-grid 24 --stress-multi-asset-count 16 --stress-cache-prefetch-count 48 --asset-memory-budget-mib 1 --stress-cycle-frames 300 --frames 1950 --profile-warmup 60 --profile-csv target/profiles/multi-asset-lifecycle.csv"
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
    let (checker, checker_center, optional_triangle, alpha_showcase) = {
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
        let alpha_showcase = if stress_grid_side == 0 {
            let handle = server.load::<ModelAsset>("models/material_alpha_showcase.glb")?;
            let imported = server
                .get(handle)
                .ok_or("material_alpha_showcase.glb failed to decode")?;
            let has_alpha_modes = imported
                .materials
                .iter()
                .any(|material| material.alpha_mode == ribbon_plugin_gltf::AlphaMode::Opaque)
                && imported
                    .materials
                    .iter()
                    .any(|material| material.alpha_mode == ribbon_plugin_gltf::AlphaMode::Mask)
                && imported
                    .materials
                    .iter()
                    .filter(|material| material.alpha_mode == ribbon_plugin_gltf::AlphaMode::Blend)
                    .count()
                    >= 2
                && imported
                    .materials
                    .iter()
                    .any(|material| material.double_sided);
            if !has_alpha_modes || imported.nodes.len() != 5 {
                return Err(
                    "material_alpha_showcase.glb is missing the expected OPAQUE/MASK/BLEND/doubleSided fixtures"
                        .into(),
                );
            }
            Some(handle)
        } else {
            None
        };
        (checker, checker_center, triangle, alpha_showcase)
    };

    let mut stress_model_paths = Vec::<String>::new();
    let mut stress_leases = Vec::<AssetRef<ModelAsset>>::new();
    let mut initial_stress_models: Option<Vec<(ribbon_assets::Handle<ModelAsset>, Vec3)>> = None;
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
        title: "Ribbon | Graphics Showcase".to_owned(),
        width: 1280,
        height: 800,
    });
    app.insert_resource(ClearColor([0.018, 0.028, 0.048, 1.0]));
    app.insert_resource(AntiAliasingSettings {
        mode: anti_aliasing_mode,
    });
    app.insert_resource(post_processing_settings);
    println!("[RENDER3D AA] requested mode: {anti_aliasing_mode:?}");
    println!("[RENDER3D POST] preset: {post_processing_preset}");
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
    } else if let (Some((triangle, triangle_center)), Some(alpha_showcase)) =
        (optional_triangle, alpha_showcase)
    {
        add_scene(
            &mut app.world,
            checker,
            checker_center,
            triangle,
            triangle_center,
            alpha_showcase,
        );
    } else {
        return Err("default showcase triangle was not initialized".into());
    }
    app.add_system(Stage::Update, animate_showcase);
    if stress_grid_side > 0 {
        app.add_system(Stage::Update, update_large_scene_stress);
    }
    app.add_system(Stage::Last, report_render_extraction);

    println!("Ribbon Graphics Showcase");
    println!("Live features: native wgpu surface, ECS extraction, indexed meshes, depth buffer,");
    println!("Cook-Torrance metallic-roughness PBR, HDR tone mapping, directional shadows, and frustum culling.");
    if stress_grid_side == 0 {
        println!(
            "Material wall: OPAQUE alpha-ignore, MASK cutout, overlapping BLEND sheets, reversed-winding doubleSided panel."
        );
    }
    println!(
        "Controls: Space pause/resume, Left/Right orbit speed, Up/Down camera height, R reset."
    );
    println!("Remaining: cascaded/soft shadows, multiple local-light shadows, SSAO/GTAO, IBL, probes, lightmaps, and GPU culling.");
    println!(
        "Run `cargo run -p voxel_mesh_showcase` to view greedy voxels on the same mesh pipeline."
    );

    app.build()?.run();
    Ok(())
}

#[cfg(test)]
mod anti_aliasing_cli_tests {
    use super::{parse_anti_aliasing_mode, parse_post_processing_preset};
    use ribbon_plugin_render3d::AntiAliasingMode;

    #[test]
    fn parses_supported_anti_aliasing_modes_and_aliases() {
        assert_eq!(parse_anti_aliasing_mode("none"), Ok(AntiAliasingMode::None));
        assert_eq!(
            parse_anti_aliasing_mode("MSAA2"),
            Ok(AntiAliasingMode::Msaa2x)
        );
        assert_eq!(parse_anti_aliasing_mode("4x"), Ok(AntiAliasingMode::Msaa4x));
        assert_eq!(
            parse_anti_aliasing_mode("msaa8"),
            Ok(AntiAliasingMode::Msaa8x)
        );
        assert_eq!(parse_anti_aliasing_mode("FxAa"), Ok(AntiAliasingMode::Fxaa));
    }

    #[test]
    fn rejects_unknown_anti_aliasing_modes() {
        assert!(parse_anti_aliasing_mode("taa").is_err());
        assert!(parse_anti_aliasing_mode("msaa16").is_err());
    }
    #[test]
    fn parses_supported_post_processing_presets() {
        let neutral = parse_post_processing_preset("neutral").unwrap();
        assert!(!neutral.auto_exposure.enabled);
        let cinematic = parse_post_processing_preset("CINEMATIC").unwrap();
        assert!(cinematic.auto_exposure.enabled);
        assert!(cinematic.vignette.intensity > 0.0);
        assert!(cinematic.film_grain.intensity > 0.0);
        assert!(cinematic.chromatic_aberration.intensity_pixels > 0.0);
    }

    #[test]
    fn rejects_unknown_post_processing_presets() {
        assert!(parse_post_processing_preset("photoreal").is_err());
    }
}
