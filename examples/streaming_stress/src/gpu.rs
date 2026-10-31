//! Native-window GPU stress phase.
//!
//! This phase uses the real AssetServer GLTF loader, StreamingCoordinator, renderer-owned model
//! warmup, generic GPU texture warmup, and the shared per-frame upload budget. It deliberately
//! does not create a second renderer or fake GPU completions.

use crate::report::{percentile, write_frame_csv, write_markdown_report, FrameSample};
use crate::Options;
use glam::Vec3;
use ribbon_app::{App, AppExit, Stage, Time};
use ribbon_assets::{AssetServer, FsSource};
use ribbon_events::WorldEventsExt;
use ribbon_platform::{PlatformConfig, PlatformPlugin};
use ribbon_plugin_gltf::GltfLoader;
use ribbon_plugin_physics::{Collider, PhysicsPlugin};
use ribbon_plugin_render3d::{
    Camera3D, GpuResidencyBudget, GpuResourceData, GpuResourceKey, GpuResourceLease,
    GpuResourceRequestError, GpuResourceWarmup, GpuSamplerDesc, GpuUploadBudget, OwnedTextureData,
    Render3dPlugin, SunLight, Transform3D,
};
use ribbon_plugin_streaming::{
    RegionBounds, RegionCollider, RegionDefinition, RegionId, RegionModel, RegionPolicy,
    StreamObserver, StreamingConfig, StreamingCoordinator, StreamingMemoryBudget,
};
use ribbon_plugins::AppPluginExt;
use ribbon_render::gpu_window::{ClearColor, GpuWindowPlugin};
use ribbon_render::profiling::{RenderDiagnostics, RenderProfilePlugin};
use ribbon_scene::ScenePlugin;
use std::error::Error;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;

const REGION_SPACING: f32 = 220.0;
const MAX_ACTIVE_REGIONS: usize = 4;
const CONCURRENT_REGION_COUNT: usize = 4;
const CONCURRENT_GPU_HOLD_FRAMES: usize = 24;
const CONCURRENT_CLUSTER_CENTER: Vec3 = Vec3::new(-1_000.0, 0.0, 0.0);
const OBSERVER_ID: u64 = 0x5354_5245_5353;
const GPU_TEXTURE_NAMESPACE: u64 = 0x5249_4242_4f4e_4750;

struct GpuStressLeases {
    _leases: Vec<GpuResourceLease>,
}

#[derive(Clone, Copy)]
struct StressOptionsSnapshot {
    generic_resource_count: usize,
    initial_resource_count: usize,
    expect_gpu_pressure: bool,
    pressure_stage: u8,
    texture_size: u32,
}

#[derive(Clone)]
struct SharedCapture(Arc<Mutex<CaptureState>>);

struct CaptureState {
    regions: usize,
    frames_per_region: usize,
    route_frames: u64,
    concurrent_frames: u64,
    total_frames: u64,
    last_target: Option<usize>,
    target_started: Option<Instant>,
    target_latency_recorded: bool,
    previous_position: Option<Vec3>,
    frame_started: Option<Instant>,
    samples: Vec<FrameSample>,
    activation_latencies_ns: Vec<u64>,
    failure: Option<String>,
    exit_sent: bool,
    final_unloaded_regions: usize,
    final_failed_regions: usize,
    render_frame_count: u64,
    model_uploads_cumulative: u64,
    generic_uploads_cumulative: u64,
    generic_upload_failures_cumulative: u64,
    model_warmup_failed: usize,
    generic_warmup_failed: usize,
    generic_warmup_ready: usize,
    gpu_timestamp_queries_supported: bool,
    gpu_timestamp_samples: u64,
    max_uploads_observed: usize,
    max_upload_bytes_observed: u64,
    peak_active_regions: usize,
    upload_budget_violations: usize,
    shared_generic_dedup_passed: bool,
    concurrent_cluster_started: Option<Instant>,
    concurrent_cluster_latency_ns: Option<u64>,
    concurrent_cluster_hold_frames: usize,
    concurrent_cluster_peak_active: usize,
    generic_leases_released: bool,
    generic_resources_released: u64,
    generic_live_textures: usize,
    generic_live_samplers: usize,
    generic_texture_bytes_estimate: u64,
    gpu_pressure_rejection_passed: bool,
    generic_gpu_evictions: u64,
    generic_gpu_pressure_events: u64,
    generic_gpu_budget_rejections: u64,
}

pub(crate) fn run(options: &Options) -> Result<(), Box<dyn Error>> {
    let output_dir = options.output_dir.join("gpu");
    std::fs::create_dir_all(&output_dir)?;
    validate_corpus(&options.asset_root, options.asset_count)?;

    let mut app = build_gpu_app(options, &output_dir)?;
    register_regions(&mut app, options)?;
    let (shared_dedup_passed, gpu_pressure_rejection_passed) =
        queue_generic_gpu_stress(&mut app, options)?;

    let route_frames = options
        .regions
        .saturating_mul(options.cycles)
        .saturating_mul(options.frames_per_region) as u64;
    let concurrent_frames = options.frames_per_region.max(48) as u64;
    let total_frames = route_frames
        .saturating_add(concurrent_frames)
        .saturating_add(options.tail_frames as u64)
        .max(
            route_frames
                .saturating_add(concurrent_frames)
                .saturating_add(1),
        );
    let capture = SharedCapture(Arc::new(Mutex::new(CaptureState {
        regions: options.regions,
        frames_per_region: options.frames_per_region,
        route_frames,
        concurrent_frames,
        total_frames,
        last_target: None,
        target_started: None,
        target_latency_recorded: false,
        previous_position: None,
        frame_started: None,
        samples: Vec::with_capacity(total_frames.min(100_000) as usize),
        activation_latencies_ns: Vec::with_capacity(options.regions.saturating_mul(options.cycles)),
        failure: None,
        exit_sent: false,
        final_unloaded_regions: 0,
        final_failed_regions: 0,
        render_frame_count: 0,
        model_uploads_cumulative: 0,
        generic_uploads_cumulative: 0,
        generic_upload_failures_cumulative: 0,
        model_warmup_failed: 0,
        generic_warmup_failed: 0,
        generic_warmup_ready: 0,
        gpu_timestamp_queries_supported: false,
        gpu_timestamp_samples: 0,
        max_uploads_observed: 0,
        max_upload_bytes_observed: 0,
        peak_active_regions: 0,
        upload_budget_violations: 0,
        shared_generic_dedup_passed: shared_dedup_passed,
        concurrent_cluster_started: None,
        concurrent_cluster_latency_ns: None,
        concurrent_cluster_hold_frames: 0,
        concurrent_cluster_peak_active: 0,
        generic_leases_released: false,
        generic_resources_released: 0,
        generic_live_textures: 0,
        generic_live_samplers: 0,
        generic_texture_bytes_estimate: 0,
        gpu_pressure_rejection_passed,
        generic_gpu_evictions: 0,
        generic_gpu_pressure_events: 0,
        generic_gpu_budget_rejections: 0,
    })));
    app.insert_resource(capture.clone());
    app.add_system(Stage::First, route_observer_and_camera);
    let capture_for_last = capture.clone();
    let gpu_options = options.clone();
    app.add_system(Stage::Last, move |world| {
        capture_gpu_frame(world, &capture_for_last, &gpu_options)
    });

    println!(
        "[GPU] opening native window; regions={} cycles={} frames={} upload_budget={} ops/{} MiB",
        options.regions,
        options.cycles,
        total_frames,
        options.uploads_per_frame,
        options.upload_budget_mib,
    );
    app.run();

    let mut state = capture
        .0
        .lock()
        .map_err(|_| "GPU stress capture lock was poisoned")?;
    let samples = std::mem::take(&mut state.samples);
    let activation_latencies = std::mem::take(&mut state.activation_latencies_ns);
    let upload_budget_violations = state.upload_budget_violations;
    let final_unloaded_regions = state.final_unloaded_regions;
    let final_failed_regions = state.final_failed_regions;
    let render_frame_count = state.render_frame_count;
    let model_uploads = state.model_uploads_cumulative;
    let generic_uploads = state.generic_uploads_cumulative;
    let generic_upload_failures = state.generic_upload_failures_cumulative;
    let model_failed = state.model_warmup_failed;
    let generic_failed = state.generic_warmup_failed;
    let generic_ready = state.generic_warmup_ready;
    let gpu_timestamp_supported = state.gpu_timestamp_queries_supported;
    let gpu_timestamp_samples = state.gpu_timestamp_samples;
    let max_uploads_observed = state.max_uploads_observed;
    let max_upload_bytes_observed = state.max_upload_bytes_observed;
    let peak_active_regions = state.peak_active_regions;
    let route_failure = state.failure.take();
    let shared_dedup_passed = state.shared_generic_dedup_passed;
    let concurrent_cluster_latency_ns = state.concurrent_cluster_latency_ns.unwrap_or(0);
    let concurrent_cluster_hold_frames = state.concurrent_cluster_hold_frames;
    let concurrent_cluster_peak_active = state.concurrent_cluster_peak_active;
    let generic_leases_released = state.generic_leases_released;
    let generic_release_count = state.generic_resources_released;
    let generic_live_textures = state.generic_live_textures;
    let generic_live_samplers = state.generic_live_samplers;
    let generic_texture_bytes_estimate = state.generic_texture_bytes_estimate;
    let gpu_pressure_rejection_passed = state.gpu_pressure_rejection_passed;
    let generic_evictions = state.generic_gpu_evictions;
    let generic_pressure_events = state.generic_gpu_pressure_events;
    let generic_budget_rejections = state.generic_gpu_budget_rejections;
    drop(state);

    write_frame_csv(&output_dir.join("gpu-frames.csv"), &samples)?;
    let gpu_hardware_observed = render_frame_count > 0;
    let upload_path_observed = model_uploads > 0 && generic_uploads > 0;
    let budget_passed = upload_budget_violations == 0;
    let active_region_cap_passed = peak_active_regions <= MAX_ACTIVE_REGIONS;
    let concurrent_cluster_passed = concurrent_cluster_peak_active == CONCURRENT_REGION_COUNT
        && concurrent_cluster_hold_frames >= CONCURRENT_GPU_HOLD_FRAMES
        && concurrent_cluster_latency_ns > 0;
    let churn_passed = final_unloaded_regions >= options.regions + CONCURRENT_REGION_COUNT
        && final_failed_regions == 0
        && active_region_cap_passed;
    let activation_count = activation_latencies.len();
    // The turn-around region is visited by both adjacent cycles without leaving its bounds,
    // so it is one continuous activation rather than two independent activations.
    let activation_expected = expected_activation_count(options.regions, options.cycles);
    let activation_passed = activation_count == activation_expected;
    let expected_generic_resources =
        options.asset_count.min(8) + 1 + usize::from(options.expect_gpu_pressure); // initial textures + sampler + optional replacement
    let generic_resources_complete =
        generic_uploads >= expected_generic_resources as u64 && generic_failed == 0;
    let generic_resources_released_passed = generic_leases_released
        && generic_release_count >= expected_generic_resources as u64
        && generic_live_textures == 0
        && generic_live_samplers == 0
        && generic_texture_bytes_estimate == 0;
    let skip_gpu_checks = !gpu_hardware_observed && !options.require_gpu;
    let checks = vec![
        (
            "Actual GPU window frames rendered".to_owned(),
            status(gpu_hardware_observed, skip_gpu_checks),
        ),
        (
            "Model GPU warmup/upload path exercised".to_owned(),
            status(model_uploads > 0, skip_gpu_checks),
        ),
        (
            "Generic standalone texture upload path exercised".to_owned(),
            status(generic_resources_complete, skip_gpu_checks),
        ),
        (
            "GPU upload count/byte budget respected".to_owned(),
            status(budget_passed, skip_gpu_checks),
        ),
        (
            "Region traversal and unload completed".to_owned(),
            status(churn_passed, skip_gpu_checks),
        ),
        (
            "Every target region reached Active".to_owned(),
            status(activation_passed, skip_gpu_checks),
        ),
        (
            "Shared generic GPU request deduplicated".to_owned(),
            status(shared_dedup_passed, skip_gpu_checks),
        ),
        (
            "GPU residency pressure rejects pinned overcommit".to_owned(),
            status(
                gpu_pressure_rejection_passed || !options.expect_gpu_pressure,
                skip_gpu_checks,
            ),
        ),
        (
            "GPU residency pressure evicts released GPU resources".to_owned(),
            status(
                !options.expect_gpu_pressure
                    || (generic_evictions > 0
                        && generic_pressure_events > 0
                        && generic_budget_rejections > 0),
                skip_gpu_checks,
            ),
        ),
        (
            "Active-region concurrency cap respected".to_owned(),
            status(active_region_cap_passed, skip_gpu_checks),
        ),
        (
            "Four regions active concurrently".to_owned(),
            status(concurrent_cluster_passed, skip_gpu_checks),
        ),
        (
            "Generic GPU leases released and renderer cache drained".to_owned(),
            status(generic_resources_released_passed, skip_gpu_checks),
        ),
    ];
    let update_wall: Vec<_> = samples.iter().map(|sample| sample.update_wall_ns).collect();
    let render_wall: Vec<_> = samples
        .iter()
        .filter_map(|sample| sample.render_frame_cpu_ns)
        .collect();
    let gpu_mesh_time: Vec<_> = samples
        .iter()
        .filter_map(|sample| sample.gpu_mesh_pass_ns)
        .collect();
    let metadata = vec![
        (
            "Mode".to_owned(),
            "GPU: native winit window and actual renderer-owned wgpu uploads".to_owned(),
        ),
        (
            "Regions / cycles".to_owned(),
            format!("{} / {}", options.regions, options.cycles),
        ),
        ("Assets".to_owned(), options.asset_count.to_string()),
        (
            "Shared model".to_owned(),
            "models/stress_0000.glb is referenced by every region".to_owned(),
        ),
        (
            "Model placements".to_owned(),
            options
                .regions
                .saturating_mul(3)
                .saturating_add(CONCURRENT_REGION_COUNT.saturating_mul(2))
                .to_string(),
        ),
        (
            "Generated GLB assets available".to_owned(),
            options.asset_count.to_string(),
        ),
        (
            "Standalone GPU textures".to_owned(),
            options.asset_count.min(8).to_string(),
        ),
        (
            "Configured upload budget".to_owned(),
            format!(
                "{} operations / {} MiB per rendered frame",
                options.uploads_per_frame, options.upload_budget_mib
            ),
        ),
        (
            "Maximum observed uploads/frame".to_owned(),
            max_uploads_observed.to_string(),
        ),
        (
            "Maximum observed upload bytes/frame".to_owned(),
            max_upload_bytes_observed.to_string(),
        ),
        (
            "Peak active regions".to_owned(),
            format!("{peak_active_regions}/{MAX_ACTIVE_REGIONS}"),
        ),
        (
            "Concurrent cluster active peak".to_owned(),
            format!("{concurrent_cluster_peak_active}/{CONCURRENT_REGION_COUNT}"),
        ),
        (
            "Concurrent cluster held frames".to_owned(),
            format!("{concurrent_cluster_hold_frames}/{CONCURRENT_GPU_HOLD_FRAMES}"),
        ),
        (
            "Concurrent cluster activation latency".to_owned(),
            format!(
                "{:.3} ms",
                concurrent_cluster_latency_ns as f64 / 1_000_000.0
            ),
        ),
        (
            "Upload budget violations".to_owned(),
            upload_budget_violations.to_string(),
        ),
        (
            "Model GPU uploads cumulative".to_owned(),
            model_uploads.to_string(),
        ),
        (
            "Generic GPU uploads cumulative".to_owned(),
            generic_uploads.to_string(),
        ),
        (
            "Generic GPU ready / failed".to_owned(),
            format!("{generic_ready} / {generic_failed}"),
        ),
        (
            "Generic GPU resources released / expected".to_owned(),
            format!("{generic_release_count}/{expected_generic_resources}"),
        ),
        (
            "Generic GPU evictions / pressure events / budget rejections".to_owned(),
            format!(
                "{generic_evictions} / {generic_pressure_events} / {generic_budget_rejections}"
            ),
        ),
        (
            "Generic GPU live textures / samplers after release".to_owned(),
            format!("{generic_live_textures} / {generic_live_samplers}"),
        ),
        (
            "Standalone texture bytes estimate after release".to_owned(),
            generic_texture_bytes_estimate.to_string(),
        ),
        (
            "Generic GPU resources expected".to_owned(),
            expected_generic_resources.to_string(),
        ),
        ("Model warmup failures".to_owned(), model_failed.to_string()),
        (
            "GPU frames presented".to_owned(),
            render_frame_count.to_string(),
        ),
        (
            "GPU timestamp queries supported".to_owned(),
            gpu_timestamp_supported.to_string(),
        ),
        (
            "GPU timestamp samples completed".to_owned(),
            gpu_timestamp_samples.to_string(),
        ),
        (
            "Successful activation latency samples".to_owned(),
            format!("{activation_count}/{activation_expected}"),
        ),
        (
            "Activation latency p50/p95/p99".to_owned(),
            format_latency(&activation_latencies),
        ),
        (
            "Final unloaded regions".to_owned(),
            format!(
                "{final_unloaded_regions}/{}",
                options.regions.saturating_add(CONCURRENT_REGION_COUNT)
            ),
        ),
        (
            "Final failed regions".to_owned(),
            final_failed_regions.to_string(),
        ),
        (
            "App frame wall p95".to_owned(),
            format!(
                "{:.3} ms",
                percentile(&update_wall, 0.95) as f64 / 1_000_000.0
            ),
        ),
        (
            "Render CPU frame p95".to_owned(),
            format!(
                "{:.3} ms",
                percentile(&render_wall, 0.95) as f64 / 1_000_000.0
            ),
        ),
        (
            "GPU mesh timestamp p95".to_owned(),
            format_optional_ms(percentile(&gpu_mesh_time, 0.95), !gpu_mesh_time.is_empty()),
        ),
        ("Tail frames".to_owned(), options.tail_frames.to_string()),
    ];
    let report_path = output_dir.join("gpu-summary.md");
    write_markdown_report(
        &report_path,
        "Ribbon Streaming Stress — GPU Phase",
        &metadata,
        &checks,
        &samples,
        &activation_latencies,
    )?;

    println!(
        "[GPU] {}: frames={} model_uploads={} generic_uploads={} budget_violations={} activation={}/{} unloaded={}/{} concurrent={}/{} generic_released={}/{} evictions={} pressure_rejections={}",
        if !gpu_hardware_observed && !options.require_gpu {
            "SKIPPED"
        } else if gpu_hardware_observed
            && upload_path_observed
            && budget_passed
            && churn_passed
            && activation_passed
            && shared_dedup_passed
            && active_region_cap_passed
            && concurrent_cluster_passed
            && generic_resources_released_passed
            && (!options.expect_gpu_pressure || (gpu_pressure_rejection_passed && generic_evictions > 0 && generic_pressure_events > 0 && generic_budget_rejections > 0))
            && generic_upload_failures == 0
            && model_failed == 0
            && generic_failed == 0
            && route_failure.is_none()
        {
            "PASS"
        } else {
            "FAIL"
        },
        render_frame_count,
        model_uploads,
        generic_uploads,
        upload_budget_violations,
        activation_count,
        activation_expected,
        final_unloaded_regions,
        options.regions + CONCURRENT_REGION_COUNT,
        concurrent_cluster_peak_active,
        CONCURRENT_REGION_COUNT,
        generic_release_count,
        expected_generic_resources,
        generic_evictions,
        generic_budget_rejections,
    );
    println!(
        "[GPU] frame CSV: {}",
        output_dir.join("gpu-frames.csv").display()
    );
    println!(
        "[GPU] renderer profile: {}",
        output_dir.join("render-profile.csv").display()
    );
    println!("[GPU] report: {}", report_path.display());

    if let Some(error) = route_failure {
        return Err(format!("GPU stress route failed: {error}").into());
    }
    if options.require_gpu && !gpu_hardware_observed {
        return Err("--require-gpu was set but no real presented GPU frame was observed".into());
    }
    if gpu_hardware_observed
        && (!upload_path_observed
            || generic_upload_failures > 0
            || model_failed > 0
            || generic_failed > 0
            || !budget_passed
            || !churn_passed
            || !activation_passed
            || !shared_dedup_passed
            || !active_region_cap_passed
            || !concurrent_cluster_passed
            || !generic_resources_released_passed
            || (options.expect_gpu_pressure
                && (!gpu_pressure_rejection_passed
                    || generic_evictions == 0
                    || generic_pressure_events == 0
                    || generic_budget_rejections == 0)))
    {
        return Err(
            "one or more GPU stress acceptance checks failed; inspect gpu-summary.md".into(),
        );
    }
    if !gpu_hardware_observed {
        println!(
            "[GPU] SKIPPED: GPU hardware metrics are unavailable; headless metrics remain valid."
        );
    }
    Ok(())
}

fn build_gpu_app(options: &Options, output_dir: &Path) -> Result<App, Box<dyn Error>> {
    let streaming_config = StreamingConfig {
        memory: StreamingMemoryBudget {
            cpu_cache_bytes: Some(512 * 1024 * 1024),
            cpu_resident_bytes: Some(1024 * 1024 * 1024),
            gpu_estimate_bytes: Some(1024 * 1024 * 1024),
        },
        max_new_region_requests_per_update: 12,
        max_asset_requests_per_update: 24,
        max_region_activations_per_update: 2,
        max_entity_spawns_per_update: 32,
        max_active_regions: MAX_ACTIVE_REGIONS,
        profile_cpu: true,
        prefetch_neighbors: true,
        ..StreamingConfig::default()
    };
    let mut app = App::new();
    app.insert_resource(PlatformConfig {
        title: "Ribbon Engine — Streaming Stress GPU".to_owned(),
        width: 1280,
        height: 720,
    });
    app.insert_resource(ClearColor([0.025, 0.03, 0.045, 1.0]));
    app.add_plugin(PlatformPlugin)?;
    app.add_plugin(ScenePlugin)?;
    app.add_plugin(ribbon_assets::AssetPlugin)?;
    {
        let server = app.world.resource_mut::<AssetServer>();
        server.register_loader(GltfLoader);
        server.set_worker_count(4);
        server.add_source(10, FsSource::new(options.asset_root.clone()));
    }
    app.add_plugin(GpuWindowPlugin)?;
    app.add_plugin(Render3dPlugin)?;
    app.world.insert_resource(GpuUploadBudget {
        max_models_per_frame: options.uploads_per_frame,
        max_bytes_per_frame: Some((options.upload_budget_mib as u64).saturating_mul(1024 * 1024)),
    });
    app.add_plugin(PhysicsPlugin)?;
    app.add_plugin(ribbon_plugin_streaming::StreamingPlugin::new(
        streaming_config,
    ))?;
    app.add_plugin(
        RenderProfilePlugin::new(output_dir.join("render-profile.csv"))
            .warmup_frames(15)
            .max_samples(
                options
                    .regions
                    .saturating_mul(options.cycles)
                    .saturating_mul(options.frames_per_region)
                    .saturating_add(options.frames_per_region.max(48))
                    .saturating_add(options.tail_frames),
            ),
    )?;
    app.world.spawn((
        Transform3D::looking_at(Vec3::new(0.0, 85.0, 160.0), Vec3::ZERO, Vec3::Y),
        Camera3D {
            far: (options.regions as f32 * REGION_SPACING * 1.5).max(2_000.0),
            ..Camera3D::default()
        },
    ));
    app.world.spawn((SunLight::default(),));
    Ok(app)
}

fn validate_corpus(root: &Path, asset_count: usize) -> Result<(), Box<dyn Error>> {
    if !root.join("models/stress_0000.glb").is_file() {
        return Err(format!(
            "generated GLB corpus is missing at {}; run tools/assets/generate_asset_stress_corpus.py first",
            root.display()
        )
        .into());
    }
    for index in 0..asset_count {
        let path = root.join(format!("models/stress_{index:04}.glb"));
        if !path.is_file() {
            return Err(format!("GPU stress corpus is missing {}", path.display()).into());
        }
    }
    Ok(())
}

fn register_regions(app: &mut App, options: &Options) -> Result<(), Box<dyn Error>> {
    let coordinator = app.world.resource_mut::<StreamingCoordinator>();
    let policy = RegionPolicy {
        preload_distance: 120.0,
        activation_distance: 58.0,
        unload_distance: 178.0,
        base_priority: 50,
        auto_activate: true,
        preview_when_ready: false,
    };
    for index in 0..options.regions {
        let center = Vec3::new(index as f32 * REGION_SPACING, 0.0, 0.0);
        let bounds = RegionBounds::new(
            center - Vec3::new(42.0, 8.0, 42.0),
            center + Vec3::new(42.0, 32.0, 42.0),
        )?;
        let common_transform = Transform3D {
            translation: center + Vec3::new(-26.0, 2.0, 0.0),
            scale: Vec3::splat(14.0),
            ..Transform3D::default()
        };
        let unique_transform_a = Transform3D {
            translation: center + Vec3::new(0.0, 2.0, 0.0),
            scale: Vec3::splat(14.0),
            ..Transform3D::default()
        };
        let unique_transform_b = Transform3D {
            translation: center + Vec3::new(26.0, 2.0, 0.0),
            scale: Vec3::splat(14.0),
            ..Transform3D::default()
        };
        let unique_asset_a = 1 + (index % (options.asset_count - 1));
        let unique_asset_b = 1 + ((index + options.regions) % (options.asset_count - 1));
        let definition = RegionDefinition::new(
            RegionId::new(format!("gpu-stress-region-{index:04}")),
            format!("GPU stress region {index:04}"),
            bounds,
        )
        .with_policy(policy)
        .with_model(RegionModel::new("models/stress_0000.glb", common_transform))
        .with_model(RegionModel::new(
            format!("models/stress_{unique_asset_a:04}.glb"),
            unique_transform_a,
        ))
        .with_model(RegionModel::new(
            format!("models/stress_{unique_asset_b:04}.glb"),
            unique_transform_b,
        ))
        .with_collider(RegionCollider::new(
            Transform3D::from_translation(center),
            Collider::cuboid(Vec3::new(34.0, 2.0, 34.0)),
        ));
        coordinator.register_region(definition)?;
    }

    // Four overlapping authored regions are activated together after the linear traversal. This
    // verifies real multi-region residency/physics admission instead of merely checking a cap
    // that the one-region-at-a-time route never approaches.
    for index in 0..CONCURRENT_REGION_COUNT {
        let offset = (index as f32 - 1.5) * 12.0;
        let center = CONCURRENT_CLUSTER_CENTER + Vec3::new(offset, 0.0, 0.0);
        let bounds = RegionBounds::new(
            center - Vec3::new(42.0, 8.0, 42.0),
            center + Vec3::new(42.0, 32.0, 42.0),
        )?;
        let shared_transform = Transform3D {
            translation: center + Vec3::new(-8.0, 2.0, 0.0),
            scale: Vec3::splat(10.0),
            ..Transform3D::default()
        };
        let unique_asset = 1 + (index % (options.asset_count - 1));
        let unique_transform = Transform3D {
            translation: center + Vec3::new(8.0, 2.0, 0.0),
            scale: Vec3::splat(10.0),
            ..Transform3D::default()
        };
        let definition = RegionDefinition::new(
            RegionId::new(format!("gpu-concurrent-region-{index:02}")),
            format!("GPU concurrent stress region {index:02}"),
            bounds,
        )
        .with_policy(policy)
        .with_model(RegionModel::new("models/stress_0000.glb", shared_transform))
        .with_model(RegionModel::new(
            format!("models/stress_{unique_asset:04}.glb"),
            unique_transform,
        ))
        .with_collider(RegionCollider::new(
            Transform3D::from_translation(center),
            Collider::cuboid(Vec3::new(8.0, 2.0, 8.0)),
        ));
        coordinator.register_region(definition)?;
    }
    Ok(())
}

fn queue_generic_gpu_stress(
    app: &mut App,
    options: &Options,
) -> Result<(bool, bool), Box<dyn Error>> {
    let texture_count = options.asset_count.min(8);
    let side = options.texture_size as u32;
    let mut leases = Vec::with_capacity(texture_count * 2 + 1);
    let mut dedup_passed = false;
    let mut pressure_rejection_passed = !options.expect_gpu_pressure;
    let texture_bytes = u64::from(side)
        .saturating_mul(u64::from(side))
        .saturating_mul(4);
    let texture_estimate = texture_bytes.saturating_add(texture_bytes / 3);
    let unique_queued = {
        let warmup = app.world.resource_mut::<GpuResourceWarmup>();
        if options.expect_gpu_pressure {
            warmup.set_budget(GpuResidencyBudget {
                total_bytes: Some(
                    texture_estimate
                        .saturating_mul(texture_count as u64)
                        .saturating_add(64),
                ),
                mesh_bytes: None,
                texture_bytes: Some(texture_estimate.saturating_mul(texture_count as u64)),
                material_bytes: None,
                sampler_count: Some(1),
            });
        }
        for index in 0..texture_count {
            let key = GpuResourceKey {
                namespace: GPU_TEXTURE_NAMESPACE,
                id: index as u64,
                generation: 0,
            };
            let payload = stress_texture_payload(side, index as u8);
            let first = warmup.request(key, payload.clone(), 100 - index as i32)?;
            let before_duplicate = warmup.counts().queued;
            let second = warmup.request(key, payload, 120 - index as i32)?;
            let after_duplicate = warmup.counts().queued;
            if index == 0 {
                dedup_passed = before_duplicate == after_duplicate;
            }
            leases.push(first);
            leases.push(second);
        }
        let sampler_key = GpuResourceKey {
            namespace: GPU_TEXTURE_NAMESPACE + 1,
            id: 0,
            generation: 0,
        };
        let sampler_lease = warmup.request(
            sampler_key,
            GpuResourceData::Sampler(GpuSamplerDesc::default()),
            200,
        )?;
        leases.push(sampler_lease);

        if options.expect_gpu_pressure {
            let overcommit_key = GpuResourceKey {
                namespace: GPU_TEXTURE_NAMESPACE,
                id: u64::MAX - 1,
                generation: 0,
            };
            pressure_rejection_passed = matches!(
                warmup.request(overcommit_key, stress_texture_payload(side, 241), 250),
                Err(GpuResourceRequestError::BudgetExceeded(_))
            );
        }
        warmup.counts().queued
    };
    if unique_queued != texture_count + 1 {
        return Err(format!(
            "generic GPU warmup dedup expected {} unique entries, observed {unique_queued}",
            texture_count + 1
        )
        .into());
    }
    if !pressure_rejection_passed {
        return Err("GPU residency pressure test failed to reject an overcommit while all existing texture leases were live".into());
    }
    app.insert_resource(GpuStressLeases { _leases: leases });
    app.insert_resource(StressOptionsSnapshot {
        generic_resource_count: texture_count + 1,
        initial_resource_count: texture_count + 1,
        expect_gpu_pressure: options.expect_gpu_pressure,
        pressure_stage: 0,
        texture_size: side,
    });
    Ok((dedup_passed, pressure_rejection_passed))
}

fn stress_texture_payload(side: u32, seed: u8) -> GpuResourceData {
    let mut rgba8 = vec![0_u8; side as usize * side as usize * 4];
    let color = [
        seed.wrapping_mul(37).wrapping_add(31),
        seed.wrapping_mul(59).wrapping_add(73),
        seed.wrapping_mul(83).wrapping_add(17),
        255,
    ];
    let (pixels, remainder) = rgba8.as_chunks_mut::<4>();
    debug_assert!(remainder.is_empty());
    for pixel in pixels {
        pixel.copy_from_slice(&color);
    }
    GpuResourceData::Texture {
        image: OwnedTextureData {
            width: side,
            height: side,
            rgba8,
        },
        srgb: true,
    }
}

fn route_observer_and_camera(world: &mut ribbon_app::World) {
    let frame = world.resource::<Time>().frame;
    let Some(capture) = world.try_resource::<SharedCapture>().cloned() else {
        return;
    };
    let (position, velocity, target, concurrent_phase, release_after_ready) = {
        let Ok(mut state) = capture.0.lock() else {
            return;
        };
        let target = if frame <= state.route_frames {
            let route_slot = (frame.saturating_sub(1) / state.frames_per_region as u64) as usize;
            let cycle = route_slot / state.regions;
            let offset = route_slot % state.regions;
            Some(if cycle.is_multiple_of(2) {
                offset
            } else {
                state.regions - 1 - offset
            })
        } else {
            None
        };
        let concurrent_phase = frame > state.route_frames
            && frame <= state.route_frames.saturating_add(state.concurrent_frames);
        if concurrent_phase && state.concurrent_cluster_started.is_none() {
            state.concurrent_cluster_started = Some(Instant::now());
        }
        let position = if let Some(index) = target {
            Vec3::new(index as f32 * REGION_SPACING, 0.0, 0.0)
        } else if concurrent_phase {
            CONCURRENT_CLUSTER_CENTER
        } else {
            Vec3::new(-100_000.0, 0.0, 0.0)
        };
        if target != state.last_target {
            state.last_target = target;
            state.target_started = target.map(|_| Instant::now());
            state.target_latency_recorded = false;
        }
        let velocity = state.previous_position.map_or(Vec3::ZERO, |previous| {
            (position - previous) * (60.0 / state.frames_per_region.max(1) as f32)
        });
        state.previous_position = Some(position);
        state.frame_started = Some(Instant::now());
        let release_after_ready =
            frame > state.route_frames.saturating_add(state.concurrent_frames);
        (
            position,
            velocity,
            target,
            concurrent_phase,
            release_after_ready,
        )
    };

    let update_result = world
        .resource_mut::<StreamingCoordinator>()
        .update_observer(StreamObserver::new(OBSERVER_ID, position).with_velocity(velocity));
    if let Err(error) = update_result {
        if let Ok(mut state) = capture.0.lock() {
            state.failure = Some(if let Some(index) = target {
                format!("could not update route observer at region {index}: {error}")
            } else {
                format!("could not update route/concurrency observer: {error}")
            });
        }
    }

    if target.is_some() || concurrent_phase {
        let eye = position + Vec3::new(0.0, 78.0, 155.0);
        let target_position = position + Vec3::new(0.0, 4.0, 0.0);
        for (_, (transform, _)) in world.query_mut::<(&mut Transform3D, &Camera3D)>() {
            *transform = Transform3D::looking_at(eye, target_position, Vec3::Y);
        }
    }

    // Drop generic warmup leases only after the first real GPU uploads are ready. In pressure
    // mode, request a replacement while the old GPU entries are orphaned: admission must evict
    // at least one READY texture, and the render thread must release its actual GPU handle.
    if release_after_ready {
        let snapshot = world.try_resource::<StressOptionsSnapshot>().copied();
        if let Some(mut snapshot) = snapshot {
            let counts = world
                .try_resource::<GpuResourceWarmup>()
                .map(GpuResourceWarmup::counts)
                .unwrap_or_default();
            match snapshot.pressure_stage {
                0 => {
                    let ready = counts.ready >= snapshot.initial_resource_count
                        && counts.failed == 0
                        && counts.queued == 0
                        && counts.preparing == 0;
                    if ready {
                        drop(world.remove_resource::<GpuStressLeases>());
                        if snapshot.expect_gpu_pressure {
                            let key = GpuResourceKey {
                                namespace: GPU_TEXTURE_NAMESPACE,
                                id: u64::MAX,
                                generation: 0,
                            };
                            let replacement = world.resource_mut::<GpuResourceWarmup>().request(
                                key,
                                stress_texture_payload(snapshot.texture_size, 251),
                                255,
                            );
                            match replacement {
                                Ok(lease) => {
                                    world.insert_resource(GpuStressLeases {
                                        _leases: vec![lease],
                                    });
                                    snapshot.generic_resource_count = 1;
                                    snapshot.pressure_stage = 1;
                                    world.insert_resource(snapshot);
                                }
                                Err(error) => {
                                    if let Ok(mut state) = capture.0.lock() {
                                        state.failure = Some(format!("GPU replacement request failed under released-resource pressure: {error}"));
                                    }
                                }
                            }
                        } else {
                            snapshot.pressure_stage = 3;
                            world.insert_resource(snapshot);
                            if let Ok(mut state) = capture.0.lock() {
                                state.generic_leases_released = true;
                            }
                        }
                    }
                }
                1 => {
                    let replacement_ready =
                        counts.ready > 0 && counts.queued == 0 && counts.preparing == 0;
                    if replacement_ready {
                        drop(world.remove_resource::<GpuStressLeases>());
                        snapshot.pressure_stage = 2;
                        world.insert_resource(snapshot);
                    }
                }
                2 => {
                    let expected_releases = snapshot.initial_resource_count as u64 + 1;
                    if counts.released_resources >= expected_releases
                        && counts.queued == 0
                        && counts.preparing == 0
                        && counts.ready == 0
                        && counts.evicted_resources > 0
                    {
                        snapshot.pressure_stage = 3;
                        world.insert_resource(snapshot);
                        if let Ok(mut state) = capture.0.lock() {
                            state.generic_leases_released = true;
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

fn capture_gpu_frame(world: &mut ribbon_app::World, capture: &SharedCapture, options: &Options) {
    let frame = world.resource::<Time>().frame;
    let render = world
        .try_resource::<RenderDiagnostics>()
        .cloned()
        .unwrap_or_default();
    let stream = world
        .try_resource::<ribbon_plugin_streaming::StreamDiagnostics>()
        .cloned()
        .unwrap_or_default();
    let cpu = world.resource::<AssetServer>().cpu_memory_stats();
    let worker = world.resource::<AssetServer>().worker_stats();
    let warmup = world
        .try_resource::<GpuResourceWarmup>()
        .map(GpuResourceWarmup::counts)
        .unwrap_or_default();
    let concurrent_active = {
        let coordinator = world.resource::<StreamingCoordinator>();
        (0..CONCURRENT_REGION_COUNT)
            .filter(|index| {
                coordinator
                    .region_state(&RegionId::new(format!("gpu-concurrent-region-{index:02}")))
                    == Some(&ribbon_plugin_streaming::RegionState::Active)
            })
            .count()
    };
    let target_state = {
        let Ok(state) = capture.0.lock() else {
            return;
        };
        state
            .last_target
            .map(|index| (index, state.target_started, state.target_latency_recorded))
    };

    if let Some((index, started, recorded)) = target_state {
        if !recorded
            && world
                .resource::<StreamingCoordinator>()
                .region_state(&RegionId::new(format!("gpu-stress-region-{index:04}")))
                == Some(&ribbon_plugin_streaming::RegionState::Active)
        {
            if let (Some(started), Ok(mut state)) = (started, capture.0.lock()) {
                state
                    .activation_latencies_ns
                    .push(started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64);
                state.target_latency_recorded = true;
            }
        }
    }

    let uploads = render
        .resources
        .model_gpu_uploads_this_frame
        .saturating_add(render.resources.generic_gpu_uploads_this_frame);
    let upload_bytes = render
        .resources
        .model_gpu_upload_bytes_this_frame
        .saturating_add(render.resources.generic_gpu_upload_bytes_this_frame);
    let budget_bytes = (options.upload_budget_mib as u64).saturating_mul(1024 * 1024);
    let upload_violation = upload_budget_violated(
        uploads,
        upload_bytes,
        options.uploads_per_frame,
        budget_bytes,
    );
    let update_wall_ns = capture
        .0
        .lock()
        .ok()
        .and_then(|mut state| state.frame_started.take())
        .map(|started| started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0);
    let sample = FrameSample {
        frame,
        update_wall_ns,
        render_frame_cpu_ns: (render.frame_count > 0).then_some(render.last_frame_cpu_ns),
        streaming_update_ns: stream.last_update_cpu_ns,
        cpu_resident_bytes: cpu.resident_bytes_estimate,
        cpu_pinned_bytes: cpu.pinned_bytes_estimate,
        cpu_unpinned_bytes: cpu.unpinned_bytes_estimate,
        gpu_estimate_bytes: (render.frame_count > 0)
            .then_some(render.resources.total_bytes_estimate()),
        active_regions: stream.active_regions,
        loading_regions: stream.loading_regions,
        dormant_regions: stream.dormant_regions,
        failed_regions: stream.failed_regions,
        queued_asset_jobs: worker.queued_jobs,
        active_asset_workers: worker.active_workers,
        pending_asset_completions: worker.pending_completions,
        asset_completions_processed_last_poll: worker.completions_processed_last_poll,
        asset_source_read_ns_last_poll: worker.source_read_time_ns_last_poll,
        asset_decode_ns_last_poll: worker.decode_time_ns_last_poll,
        gpu_uploads_this_frame: (render.frame_count > 0).then_some(uploads),
        gpu_upload_bytes_this_frame: (render.frame_count > 0).then_some(upload_bytes),
        gpu_frame_count: Some(render.frame_count),
        gpu_model_uploads_this_frame: (render.frame_count > 0)
            .then_some(render.resources.model_gpu_uploads_this_frame),
        gpu_generic_uploads_this_frame: (render.frame_count > 0)
            .then_some(render.resources.generic_gpu_uploads_this_frame),
        gpu_mesh_pass_ns: render.last_gpu_mesh_pass_ns,
        gpu_timestamp_queries_supported: Some(render.gpu_timestamp_queries_supported),
        gpu_live_streamed_textures: (render.frame_count > 0)
            .then_some(render.resources.live_streamed_textures),
        gpu_live_streamed_samplers: (render.frame_count > 0)
            .then_some(render.resources.live_streamed_samplers),
        gpu_streamed_texture_bytes_estimate: (render.frame_count > 0)
            .then_some(render.resources.streamed_texture_bytes_estimate),
        gpu_generic_resources_released: Some(warmup.released_resources),
        concurrent_regions_active: Some(concurrent_active),
    };
    if let Ok(mut state) = capture.0.lock() {
        state.samples.push(sample);
        state.render_frame_count = render.frame_count;
        state.model_uploads_cumulative = render.resources.model_gpu_uploads_cumulative;
        state.generic_uploads_cumulative = render.resources.generic_gpu_uploads_cumulative;
        state.generic_upload_failures_cumulative =
            render.resources.generic_gpu_upload_failures_cumulative;
        state.model_warmup_failed = render.resources.model_warmup_failed;
        state.generic_warmup_failed = warmup.failed;
        state.generic_warmup_ready = state.generic_warmup_ready.max(warmup.ready);
        state.generic_resources_released = warmup.released_resources;
        state.generic_live_textures = render.resources.live_streamed_textures;
        state.generic_live_samplers = render.resources.live_streamed_samplers;
        state.generic_texture_bytes_estimate = render.resources.streamed_texture_bytes_estimate;
        state.generic_gpu_evictions = warmup.evicted_resources;
        state.generic_gpu_pressure_events = warmup.memory_pressure_events;
        state.generic_gpu_budget_rejections = warmup.budget_rejections;
        state.concurrent_cluster_peak_active =
            state.concurrent_cluster_peak_active.max(concurrent_active);
        if concurrent_active == CONCURRENT_REGION_COUNT {
            state.concurrent_cluster_hold_frames =
                state.concurrent_cluster_hold_frames.saturating_add(1);
            if state.concurrent_cluster_latency_ns.is_none() {
                if let Some(started) = state.concurrent_cluster_started {
                    state.concurrent_cluster_latency_ns =
                        Some(started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64);
                }
            }
        }
        state.gpu_timestamp_queries_supported = render.gpu_timestamp_queries_supported;
        state.gpu_timestamp_samples = render.gpu_timestamp_samples;
        state.max_uploads_observed = state.max_uploads_observed.max(uploads);
        state.max_upload_bytes_observed = state.max_upload_bytes_observed.max(upload_bytes);
        state.peak_active_regions = state.peak_active_regions.max(stream.active_regions);
        if render.frame_count > 0 && upload_violation {
            state.upload_budget_violations = state.upload_budget_violations.saturating_add(1);
        }
    }

    let should_exit = frame
        >= capture
            .0
            .lock()
            .map_or(u64::MAX, |state| state.total_frames);
    if should_exit {
        if let Ok(mut state) = capture.0.lock() {
            if !state.exit_sent {
                state.exit_sent = true;
                let snapshot = world.resource::<StreamingCoordinator>().snapshot();
                state.final_unloaded_regions = snapshot
                    .regions
                    .iter()
                    .filter(|region| {
                        (region.id.as_str().starts_with("gpu-stress-region-")
                            || region.id.as_str().starts_with("gpu-concurrent-region-"))
                            && region.state == ribbon_plugin_streaming::RegionState::Unloaded
                    })
                    .count();
                state.final_failed_regions = snapshot
                    .regions
                    .iter()
                    .filter(|region| {
                        (region.id.as_str().starts_with("gpu-stress-region-")
                            || region.id.as_str().starts_with("gpu-concurrent-region-"))
                            && matches!(
                                &region.state,
                                ribbon_plugin_streaming::RegionState::Failed(_)
                            )
                    })
                    .count();
                world.send_event(AppExit);
            }
        }
    }
}

fn upload_budget_violated(
    uploads: usize,
    upload_bytes: u64,
    max_uploads: usize,
    max_bytes: u64,
) -> bool {
    uploads > max_uploads || (uploads > 1 && upload_bytes > max_bytes)
}

fn expected_activation_count(regions: usize, cycles: usize) -> usize {
    regions
        .saturating_mul(cycles)
        .saturating_sub(cycles.saturating_sub(1))
}

fn status(passed: bool, skipped: bool) -> String {
    if passed {
        "PASS".to_owned()
    } else if skipped {
        "SKIP (no GPU frames)".to_owned()
    } else {
        "FAIL".to_owned()
    }
}

fn format_latency(values: &[u64]) -> String {
    if values.is_empty() {
        return "unavailable".to_owned();
    }
    format!(
        "{:.3}/{:.3}/{:.3} ms",
        percentile(values, 0.50) as f64 / 1_000_000.0,
        percentile(values, 0.95) as f64 / 1_000_000.0,
        percentile(values, 0.99) as f64 / 1_000_000.0,
    )
}

fn format_optional_ms(value_ns: u64, available: bool) -> String {
    if available {
        format!("{:.3} ms", value_ns as f64 / 1_000_000.0)
    } else {
        "unavailable (GPU timestamp queries unsupported or not completed)".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::{expected_activation_count, upload_budget_violated};

    #[test]
    fn one_oversized_first_upload_is_allowed_but_later_work_is_bounded() {
        assert!(!upload_budget_violated(
            1,
            8 * 1024 * 1024,
            2,
            2 * 1024 * 1024
        ));
        assert!(upload_budget_violated(
            2,
            8 * 1024 * 1024,
            2,
            2 * 1024 * 1024
        ));
        assert!(upload_budget_violated(3, 1024, 2, 2 * 1024 * 1024));
    }

    #[test]
    fn route_turnaround_does_not_double_count_the_shared_endpoint() {
        assert_eq!(expected_activation_count(48, 1), 48);
        assert_eq!(expected_activation_count(48, 2), 95);
        assert_eq!(expected_activation_count(48, 3), 142);
    }

    #[test]
    fn upload_budget_accepts_exact_limits_and_the_documented_oversized_first_upload() {
        assert!(!upload_budget_violated(
            2,
            2 * 1024 * 1024,
            2,
            2 * 1024 * 1024
        ));
        assert!(!upload_budget_violated(
            1,
            8 * 1024 * 1024,
            2,
            2 * 1024 * 1024
        ));
    }

    #[test]
    fn upload_budget_rejects_excess_operations_and_later_byte_overruns() {
        assert!(upload_budget_violated(3, 1024, 2, 2 * 1024 * 1024));
        assert!(upload_budget_violated(
            2,
            2 * 1024 * 1024 + 1,
            2,
            2 * 1024 * 1024
        ));
    }
}
