use crate::report::{percentile, write_frame_csv, write_markdown_report, FrameSample};
use crate::Options;
use glam::Vec3;
use ribbon_app::{App, Time};
use ribbon_assets::{
    AssetPlugin, AssetRef, AssetServer, AssetSource, BytesAsset, CancellationToken, LoadState,
    MemorySource,
};
use ribbon_platform::{PlatformConfig, PlatformPlugin};
use ribbon_plugin_physics::{Collider, PhysicsPlugin};
use ribbon_plugin_render3d::{Render3dPlugin, Transform3D};
use ribbon_plugin_streaming::{
    RegionBounds, RegionCollider, RegionDefinition, RegionId, RegionPolicy, SceneDefinition,
    SceneId, StreamDiagnostics, StreamObserver, StreamingConfig, StreamingCoordinator,
    StreamingMemoryBudget, StreamingPlugin,
};
use ribbon_plugins::AppPluginExt;
use ribbon_render::gpu_window::GpuWindowPlugin;
use ribbon_scene::{SceneDoc, ScenePlugin};
use std::error::Error;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

const FIXED_DELTA: Duration = Duration::from_nanos(16_666_667);
const REGION_SPACING: f32 = 260.0;
const MAX_ACTIVATION_WAIT_FRAMES: usize = 48;
const MAX_ACTIVE_REGIONS: usize = 4;
const CONCURRENT_REGION_COUNT: usize = 4;
const CONCURRENT_HOLD_FRAMES: usize = 8;
const CONCURRENT_CLUSTER_CENTER: Vec3 = Vec3::new(-1_000.0, 0.0, 0.0);

struct CancellationGateSource {
    started: Arc<AtomicBool>,
    gate: Arc<(Mutex<()>, Condvar)>,
}

impl AssetSource for CancellationGateSource {
    fn read(&self, _path: &str) -> io::Result<Vec<u8>> {
        Ok(vec![0x5a; 64])
    }

    fn read_cancellable(
        &self,
        _path: &str,
        cancellation: &CancellationToken,
    ) -> io::Result<Vec<u8>> {
        self.started.store(true, Ordering::Release);
        let (lock, wake) = &*self.gate;
        let mut guard = lock
            .lock()
            .map_err(|_| io::Error::other("cancellation gate lock poisoned"))?;
        loop {
            if cancellation.is_cancelled() {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "stress harness observed cooperative cancellation",
                ));
            }
            let result = wake
                .wait_timeout(guard, Duration::from_millis(2))
                .map_err(|_| io::Error::other("cancellation gate wait poisoned"))?;
            guard = result.0;
        }
    }

    fn modified(&self, _path: &str) -> Option<SystemTime> {
        None
    }
}

pub(crate) fn run(options: &Options) -> Result<(), Box<dyn Error>> {
    let output_dir = options.output_dir.join("headless");
    std::fs::create_dir_all(&output_dir)?;
    println!("[HEADLESS] starting streaming/asset/physics stress phase");

    let mut app = build_headless_app()?;
    app.startup();

    let payload_bytes = options.asset_bytes_kib.saturating_mul(1024);
    let payload_source = MemorySource::default();
    let mut asset_paths = Vec::with_capacity(options.asset_count);
    for index in 0..options.asset_count {
        let path = format!("payload/asset_{index:04}.bytes");
        let mut payload = vec![0_u8; payload_bytes];
        for (offset, byte) in payload.iter_mut().enumerate() {
            *byte = (offset as u8).wrapping_add((index as u8).wrapping_mul(31));
        }
        payload_source.insert(path.clone(), payload);
        asset_paths.push(path);
    }
    payload_source.insert("shared/region_shared.bytes", vec![0x37; payload_bytes]);
    app.world
        .resource_mut::<AssetServer>()
        .add_source(10, payload_source.clone());

    let mut samples = Vec::new();
    let mut asset_leases: Vec<AssetRef<BytesAsset>> = {
        let server = app.world.resource_mut::<AssetServer>();
        let mut leases = Vec::with_capacity(asset_paths.len());
        for (index, path) in asset_paths.iter().enumerate() {
            leases.push(server.request_ref_with_priority::<BytesAsset>(path, 100 - index as i32)?);
        }
        leases
    };

    let assets_loaded = wait_for_assets(&mut app, &asset_paths, &mut samples, 600)?;
    if !assets_loaded {
        return Err("timed out waiting for the headless asynchronous asset corpus".into());
    }
    let cpu_loaded = app.world.resource::<AssetServer>().cpu_memory_stats();
    if cpu_loaded.loaded_assets < options.asset_count {
        return Err(format!(
            "expected at least {} loaded CPU assets, observed {}",
            options.asset_count, cpu_loaded.loaded_assets
        )
        .into());
    }

    // Two independent consumers requesting one typed path must share one generation and pin it
    // until the final lease is dropped.
    let shared_first = app
        .world
        .resource_mut::<AssetServer>()
        .request_ref_with_priority::<BytesAsset>("shared/region_shared.bytes", 80)?;
    let shared_second = app
        .world
        .resource_mut::<AssetServer>()
        .request_ref_with_priority::<BytesAsset>("shared/region_shared.bytes", 90)?;
    if shared_first.handle() != shared_second.handle() {
        return Err("shared AssetServer requests did not return the same handle generation".into());
    }
    let shared_handle = shared_first.handle();
    let shared_path = "shared/region_shared.bytes";
    if !wait_for_assets(&mut app, &[shared_path.to_owned()], &mut samples, 120)? {
        return Err("timed out waiting for the shared CPU asset".into());
    }
    let shared_lease_count = app
        .world
        .resource::<AssetServer>()
        .strong_ref_count(shared_handle);
    if shared_lease_count != 2 {
        return Err(format!("expected 2 shared asset leases, got {shared_lease_count}").into());
    }
    drop(shared_first);
    app.world.resource_mut::<AssetServer>().evict_unused();
    if app
        .world
        .resource::<AssetServer>()
        .state::<BytesAsset>(shared_path)
        != Some(&LoadState::Loaded)
    {
        return Err("shared asset was evicted while its second consumer still held a lease".into());
    }
    drop(shared_second);
    app.world.resource_mut::<AssetServer>().evict_unused();
    if app
        .world
        .resource::<AssetServer>()
        .state::<BytesAsset>(shared_path)
        != Some(&LoadState::Unloaded)
    {
        return Err(
            "shared asset did not become evictable after its final lease was released".into(),
        );
    }

    // Make half of the decoded corpus unpinned and enforce a soft CPU cache limit. The remaining
    // strong leases must remain valid even when the cache is under pressure.
    let retained = asset_leases.split_off(asset_leases.len() / 2);
    drop(asset_leases);
    let cpu_budget = (payload_bytes as u64)
        .saturating_mul(options.asset_count as u64 / 8)
        .max(1);
    let evicted_by_budget = {
        let server = app.world.resource_mut::<AssetServer>();
        server.set_cpu_memory_budget_bytes(Some(cpu_budget));
        server.enforce_cpu_memory_budget()
    };
    let cpu_after_pressure = app.world.resource::<AssetServer>().cpu_memory_stats();
    if cpu_after_pressure.pinned_assets < retained.len() {
        return Err("CPU memory pressure invalidated one or more retained AssetRef leases".into());
    }
    for lease in &retained {
        if app
            .world
            .resource::<AssetServer>()
            .get(lease.handle())
            .is_none()
        {
            return Err("a pinned CPU asset became stale during budget enforcement".into());
        }
    }
    drop(retained);
    app.world.resource_mut::<AssetServer>().evict_unused();

    let (asset_cancellation_passed, cancellation_workers_peak) =
        run_active_cancellation_test(&mut app, &mut samples)?;
    let region_unload_cancellation_passed =
        run_region_unload_cancellation_test(&mut app, &mut samples)?;
    let cancellation_passed = asset_cancellation_passed && region_unload_cancellation_passed;

    // Register a large linear partition. Each region owns a Rapier collider and is driven only by
    // the synthetic observer; no game-specific entities or gameplay scripts are required.
    register_headless_regions(&mut app, options.regions)?;
    let route_started = Instant::now();
    let mut activation_latencies_ns = Vec::new();
    let mut activation_failures = 0_usize;
    let mut active_region_peak = 0_usize;
    let mut active_transition_visits = 0_usize;
    for cycle in 0..options.cycles {
        let indices: Box<dyn Iterator<Item = usize>> = if cycle % 2 == 0 {
            Box::new(0..options.regions)
        } else {
            Box::new((0..options.regions).rev())
        };
        for index in indices {
            let region_id = RegionId::new(format!("headless-region-{index:04}"));
            let observer_position = Vec3::new(index as f32 * REGION_SPACING, 0.0, 0.0);
            let already_active = app
                .world
                .resource::<StreamingCoordinator>()
                .region_state(&region_id)
                == Some(&ribbon_plugin_streaming::RegionState::Active);
            if already_active {
                continue;
            }
            let request_started = Instant::now();
            let mut became_active = false;
            for _ in 0..MAX_ACTIVATION_WAIT_FRAMES {
                app.world
                    .resource_mut::<StreamingCoordinator>()
                    .update_observer(StreamObserver::new(7, observer_position))?;
                let sample = step(&mut app);
                active_region_peak = active_region_peak.max(sample.active_regions);
                samples.push(sample);
                let state = app
                    .world
                    .resource::<StreamingCoordinator>()
                    .region_state(&region_id)
                    .cloned();
                match state {
                    Some(ribbon_plugin_streaming::RegionState::Active) => {
                        became_active = true;
                        break;
                    }
                    Some(ribbon_plugin_streaming::RegionState::Failed(message)) => {
                        return Err(
                            format!("region {} failed: {message}", region_id.as_str()).into()
                        );
                    }
                    _ => {}
                }
            }
            if became_active {
                activation_latencies_ns.push(request_started.elapsed().as_nanos() as u64);
                active_transition_visits = active_transition_visits.saturating_add(1);
            } else {
                activation_failures = activation_failures.saturating_add(1);
            }
        }
    }

    // Move away from all authored bounds and give physics/streaming enough updates to reconcile
    // deactivation, collider removal, lease release, and final region unloading.
    app.world
        .resource_mut::<StreamingCoordinator>()
        .update_observer(StreamObserver::new(7, Vec3::new(-100_000.0, 0.0, 0.0)))?;
    for _ in 0..48 {
        samples.push(step(&mut app));
    }
    app.world
        .resource_mut::<StreamingCoordinator>()
        .remove_observer(7);

    let (concurrency_passed, concurrency_peak, concurrency_latency_ns, concurrency_hold_frames) =
        run_concurrent_region_test(&mut app, &mut samples)?;
    active_region_peak = active_region_peak.max(concurrency_peak);

    let shared_scene_result = run_shared_region_scene_test(&mut app, &mut samples)?;
    let stream_snapshot = app.world.resource::<StreamingCoordinator>().snapshot();
    let final_unloaded_regions = stream_snapshot
        .regions
        .iter()
        .filter(|region| region.state == ribbon_plugin_streaming::RegionState::Unloaded)
        .count();
    let stream_diagnostics = stream_snapshot.diagnostics;
    let frame_values: Vec<_> = samples.iter().map(|sample| sample.update_wall_ns).collect();
    let p95_update_ns = percentile(&frame_values, 0.95);
    let p99_update_ns = percentile(&frame_values, 0.99);

    let shared_text = if shared_scene_result { "PASS" } else { "FAIL" };
    // At each direction reversal the endpoint remains resident and is not activated a second
    // time. Count those continuous visits once, rather than requiring an impossible duplicate.
    let expected_activations = expected_activation_count(options.regions, options.cycles);
    let active_region_cap_passed = active_region_peak <= MAX_ACTIVE_REGIONS;
    let expected_unloaded_regions = options
        .regions
        .saturating_add(CONCURRENT_REGION_COUNT)
        .saturating_add(2); // active SceneDoc cancellation and shared scene-owned region
    let churn_passed = activation_failures == 0
        && active_transition_visits == expected_activations
        && final_unloaded_regions >= expected_unloaded_regions
        && active_region_cap_passed;
    let report_path = output_dir.join("headless-summary.md");
    let checks = vec![
        (
            "Region load/activate/unload churn".to_owned(),
            if churn_passed { "PASS" } else { "FAIL" }.to_owned(),
        ),
        (
            "Active-region concurrency cap".to_owned(),
            if active_region_cap_passed {
                "PASS"
            } else {
                "FAIL"
            }
            .to_owned(),
        ),
        (
            "Four regions active concurrently and held".to_owned(),
            if concurrency_passed { "PASS" } else { "FAIL" }.to_owned(),
        ),
        (
            "Active asynchronous asset cancellation".to_owned(),
            if asset_cancellation_passed {
                "PASS"
            } else {
                "FAIL"
            }
            .to_owned(),
        ),
        (
            "Region unload cancels an active SceneDoc read".to_owned(),
            if region_unload_cancellation_passed {
                "PASS"
            } else {
                "FAIL"
            }
            .to_owned(),
        ),
        (
            "Shared AssetRef deduplication and pinning".to_owned(),
            "PASS".to_owned(),
        ),
        (
            "CPU memory pressure preserves pinned assets".to_owned(),
            "PASS".to_owned(),
        ),
        (
            "Shared logical region retained by two scenes".to_owned(),
            shared_text.to_owned(),
        ),
    ];
    let metadata = vec![
        (
            "Mode".to_owned(),
            "Headless; real AssetServer/StreamingCoordinator/Rapier, no GPU simulation".to_owned(),
        ),
        ("Regions".to_owned(), options.regions.to_string()),
        ("Cycles".to_owned(), options.cycles.to_string()),
        (
            "CPU test assets".to_owned(),
            options.asset_count.to_string(),
        ),
        (
            "Decoded bytes per asset".to_owned(),
            format!("{} KiB", options.asset_bytes_kib),
        ),
        ("CPU budget".to_owned(), format!("{} bytes", cpu_budget)),
        (
            "CPU assets loaded before eviction".to_owned(),
            cpu_loaded.loaded_assets.to_string(),
        ),
        (
            "CPU resident bytes before eviction".to_owned(),
            cpu_loaded.resident_bytes_estimate.to_string(),
        ),
        (
            "CPU pinned bytes after pressure".to_owned(),
            cpu_after_pressure.pinned_bytes_estimate.to_string(),
        ),
        (
            "Unpinned entries evicted by budget".to_owned(),
            evicted_by_budget.to_string(),
        ),
        (
            "Cancelled active workers observed".to_owned(),
            cancellation_workers_peak.to_string(),
        ),
        (
            "Active SceneDoc load cancelled by region unload".to_owned(),
            region_unload_cancellation_passed.to_string(),
        ),
        (
            "Successful region activations".to_owned(),
            format!("{active_transition_visits}/{expected_activations}"),
        ),
        (
            "Concurrent cluster regions".to_owned(),
            format!("{concurrency_peak}/{CONCURRENT_REGION_COUNT}"),
        ),
        (
            "Concurrent cluster activation latency".to_owned(),
            format!("{:.3} ms", concurrency_latency_ns as f64 / 1_000_000.0),
        ),
        (
            "Concurrent cluster sustained frames".to_owned(),
            format!("{concurrency_hold_frames}/{CONCURRENT_HOLD_FRAMES}"),
        ),
        (
            "Activation failures".to_owned(),
            activation_failures.to_string(),
        ),
        (
            "Final unloaded regions".to_owned(),
            final_unloaded_regions.to_string(),
        ),
        (
            "Peak active regions".to_owned(),
            format!("{active_region_peak}/{MAX_ACTIVE_REGIONS}"),
        ),
        (
            "Region transitions".to_owned(),
            stream_diagnostics.region_transitions.to_string(),
        ),
        (
            "Invalid state transitions".to_owned(),
            stream_diagnostics.invalid_state_transitions.to_string(),
        ),
        (
            "p50 app-update wall time".to_owned(),
            format!(
                "{:.3} ms",
                percentile(&frame_values, 0.50) as f64 / 1_000_000.0
            ),
        ),
        (
            "p95 app-update wall time".to_owned(),
            format!("{:.3} ms", p95_update_ns as f64 / 1_000_000.0),
        ),
        (
            "p99 app-update wall time".to_owned(),
            format!("{:.3} ms", p99_update_ns as f64 / 1_000_000.0),
        ),
        (
            "Region activation latency p50/p95/p99".to_owned(),
            format!(
                "{:.3}/{:.3}/{:.3} ms",
                percentile(&activation_latencies_ns, 0.50) as f64 / 1_000_000.0,
                percentile(&activation_latencies_ns, 0.95) as f64 / 1_000_000.0,
                percentile(&activation_latencies_ns, 0.99) as f64 / 1_000_000.0,
            ),
        ),
        (
            "Route elapsed".to_owned(),
            format!("{:.3} s", route_started.elapsed().as_secs_f64()),
        ),
    ];
    write_frame_csv(&output_dir.join("headless-frames.csv"), &samples)?;
    write_markdown_report(
        &report_path,
        "Ribbon Streaming Stress — Headless Phase",
        &metadata,
        &checks,
        &samples,
        &activation_latencies_ns,
    )?;
    println!(
        "[HEADLESS] {}: regions={} cycles={} activations={}/{} unloads={} transitions={} CPU_peak={:.2}MiB p95_update={:.3}ms",
        if churn_passed && cancellation_passed && shared_scene_result && concurrency_passed { "PASS" } else { "FAIL" },
        options.regions,
        options.cycles,
        active_transition_visits,
        expected_activations,
        final_unloaded_regions,
        stream_diagnostics.region_transitions,
        cpu_loaded.resident_bytes_estimate as f64 / 1_048_576.0,
        p95_update_ns as f64 / 1_000_000.0,
    );
    println!(
        "[HEADLESS] frames: {}",
        output_dir.join("headless-frames.csv").display()
    );
    println!("[HEADLESS] report: {}", report_path.display());

    if !churn_passed || !cancellation_passed || !shared_scene_result || !concurrency_passed {
        return Err("one or more headless streaming acceptance checks failed".into());
    }
    Ok(())
}

fn build_headless_app() -> Result<App, Box<dyn Error>> {
    let config = StreamingConfig {
        memory: StreamingMemoryBudget {
            cpu_cache_bytes: None,
            cpu_resident_bytes: Some(512 * 1024 * 1024),
            gpu_estimate_bytes: None,
        },
        max_new_region_requests_per_update: 16,
        max_asset_requests_per_update: 32,
        max_region_activations_per_update: 2,
        max_entity_spawns_per_update: 64,
        max_active_regions: MAX_ACTIVE_REGIONS,
        profile_cpu: true,
        prefetch_neighbors: false,
        ..StreamingConfig::default()
    };
    let mut app = App::new();
    app.insert_resource(PlatformConfig {
        title: "Ribbon Streaming Stress — Headless".to_owned(),
        width: 640,
        height: 480,
    });
    app.add_plugin(PlatformPlugin)?;
    app.add_plugin(ScenePlugin)?;
    app.add_plugin(AssetPlugin)?;
    app.add_plugin(GpuWindowPlugin)?;
    app.add_plugin(Render3dPlugin)?;
    app.add_plugin(PhysicsPlugin)?;
    app.add_plugin(StreamingPlugin::new(config))?;
    Ok(app.build()?)
}

fn register_headless_regions(app: &mut App, count: usize) -> Result<(), Box<dyn Error>> {
    let coordinator = app.world.resource_mut::<StreamingCoordinator>();
    let policy = RegionPolicy {
        preload_distance: 80.0,
        activation_distance: 45.0,
        unload_distance: 120.0,
        base_priority: 10,
        auto_activate: true,
        preview_when_ready: false,
    };
    for index in 0..count {
        let center = Vec3::new(index as f32 * REGION_SPACING, 0.0, 0.0);
        let bounds = RegionBounds::new(
            center - Vec3::new(24.0, 8.0, 24.0),
            center + Vec3::new(24.0, 8.0, 24.0),
        )?;
        let transform = Transform3D {
            translation: center,
            ..Transform3D::default()
        };
        let definition = RegionDefinition::new(
            RegionId::new(format!("headless-region-{index:04}")),
            format!("Synthetic region {index:04}"),
            bounds,
        )
        .with_policy(policy)
        .with_collider(RegionCollider::new(
            transform,
            Collider::cuboid(Vec3::new(2.0, 1.0, 2.0)),
        ));
        coordinator.register_region(definition)?;
    }
    for index in 0..CONCURRENT_REGION_COUNT {
        let offset = (index as f32 - 1.5) * 12.0;
        let center = CONCURRENT_CLUSTER_CENTER + Vec3::new(offset, 0.0, 0.0);
        let bounds = RegionBounds::new(
            center - Vec3::new(32.0, 8.0, 32.0),
            center + Vec3::new(32.0, 8.0, 32.0),
        )?;
        let definition = RegionDefinition::new(
            RegionId::new(format!("headless-concurrent-region-{index:02}")),
            format!("Concurrent stress region {index:02}"),
            bounds,
        )
        .with_policy(policy)
        .with_collider(RegionCollider::new(
            Transform3D::from_translation(center),
            Collider::cuboid(Vec3::new(1.0, 1.0, 1.0)),
        ));
        coordinator.register_region(definition)?;
    }

    let shared_id = RegionId::new("headless-shared-scene-region");
    let shared_bounds = RegionBounds::new(
        Vec3::new(100_000.0, -4.0, -4.0),
        Vec3::new(100_008.0, 4.0, 4.0),
    )?;
    coordinator.register_region(RegionDefinition::new(
        shared_id.clone(),
        "Shared scene-owned region",
        shared_bounds,
    ))?;
    coordinator.register_scene(SceneDefinition::new(
        SceneId::new("shared-scene-a"),
        "Consumer scene A",
        [shared_id.clone()],
    ))?;
    coordinator.register_scene(SceneDefinition::new(
        SceneId::new("shared-scene-b"),
        "Consumer scene B",
        [shared_id],
    ))?;
    Ok(())
}

fn run_concurrent_region_test(
    app: &mut App,
    samples: &mut Vec<FrameSample>,
) -> Result<(bool, usize, u64, usize), Box<dyn Error>> {
    const OBSERVER_ID: u64 = 8;
    let started = Instant::now();
    let mut peak_active = 0_usize;
    let mut activated_at_ns = None;
    let mut hold_frames = 0_usize;

    for _ in 0..MAX_ACTIVATION_WAIT_FRAMES {
        app.world
            .resource_mut::<StreamingCoordinator>()
            .update_observer(StreamObserver::new(OBSERVER_ID, CONCURRENT_CLUSTER_CENTER))?;
        let sample = step(app);
        peak_active = peak_active.max(sample.active_regions);
        samples.push(sample);

        let (active, failed) = {
            let coordinator = app.world.resource::<StreamingCoordinator>();
            let mut active = 0_usize;
            let mut failed = None;
            for index in 0..CONCURRENT_REGION_COUNT {
                let id = RegionId::new(format!("headless-concurrent-region-{index:02}"));
                match coordinator.region_state(&id) {
                    Some(ribbon_plugin_streaming::RegionState::Active) => active += 1,
                    Some(ribbon_plugin_streaming::RegionState::Failed(message)) => {
                        failed = Some((id, message.clone()));
                        break;
                    }
                    _ => {}
                }
            }
            (active, failed)
        };
        if let Some((id, message)) = failed {
            return Err(format!("concurrent region {} failed: {message}", id.as_str()).into());
        }
        if active == CONCURRENT_REGION_COUNT {
            activated_at_ns = Some(started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64);
            break;
        }
    }

    if activated_at_ns.is_some() {
        for _ in 0..CONCURRENT_HOLD_FRAMES {
            app.world
                .resource_mut::<StreamingCoordinator>()
                .update_observer(StreamObserver::new(OBSERVER_ID, CONCURRENT_CLUSTER_CENTER))?;
            let sample = step(app);
            peak_active = peak_active.max(sample.active_regions);
            samples.push(sample);
            let active = {
                let coordinator = app.world.resource::<StreamingCoordinator>();
                (0..CONCURRENT_REGION_COUNT)
                    .filter(|index| {
                        let id = RegionId::new(format!("headless-concurrent-region-{index:02}"));
                        coordinator.region_state(&id)
                            == Some(&ribbon_plugin_streaming::RegionState::Active)
                    })
                    .count()
            };
            if active == CONCURRENT_REGION_COUNT {
                hold_frames = hold_frames.saturating_add(1);
            }
        }
    }

    app.world
        .resource_mut::<StreamingCoordinator>()
        .update_observer(StreamObserver::new(
            OBSERVER_ID,
            CONCURRENT_CLUSTER_CENTER + Vec3::new(10_000.0, 0.0, 0.0),
        ))?;
    for _ in 0..48 {
        samples.push(step(app));
        let unloaded = {
            let coordinator = app.world.resource::<StreamingCoordinator>();
            (0..CONCURRENT_REGION_COUNT).all(|index| {
                let id = RegionId::new(format!("headless-concurrent-region-{index:02}"));
                coordinator.region_state(&id)
                    == Some(&ribbon_plugin_streaming::RegionState::Unloaded)
            })
        };
        if unloaded {
            break;
        }
    }
    app.world
        .resource_mut::<StreamingCoordinator>()
        .remove_observer(OBSERVER_ID);

    let passed = peak_active >= CONCURRENT_REGION_COUNT
        && activated_at_ns.is_some()
        && hold_frames == CONCURRENT_HOLD_FRAMES;
    Ok((
        passed,
        peak_active,
        activated_at_ns.unwrap_or(0),
        hold_frames,
    ))
}

fn run_shared_region_scene_test(
    app: &mut App,
    samples: &mut Vec<FrameSample>,
) -> Result<bool, Box<dyn Error>> {
    let scene_a = SceneId::new("shared-scene-a");
    let scene_b = SceneId::new("shared-scene-b");
    let shared_region = RegionId::new("headless-shared-scene-region");
    {
        let coordinator = app.world.resource_mut::<StreamingCoordinator>();
        coordinator.activate_scene(&scene_a)?;
        coordinator.activate_scene(&scene_b)?;
    }
    let mut active = false;
    for _ in 0..32 {
        samples.push(step(app));
        if app
            .world
            .resource::<StreamingCoordinator>()
            .region_state(&shared_region)
            == Some(&ribbon_plugin_streaming::RegionState::Active)
        {
            active = true;
            break;
        }
    }
    if !active {
        return Ok(false);
    }
    app.world
        .resource_mut::<StreamingCoordinator>()
        .release_scene_request(&scene_a)?;
    for _ in 0..3 {
        samples.push(step(app));
    }
    if matches!(
        app.world
            .resource::<StreamingCoordinator>()
            .region_state(&shared_region),
        Some(ribbon_plugin_streaming::RegionState::Unloaded)
            | Some(ribbon_plugin_streaming::RegionState::Unloading)
    ) {
        return Ok(false);
    }
    app.world
        .resource_mut::<StreamingCoordinator>()
        .release_scene_request(&scene_b)?;
    let mut unloaded = false;
    for _ in 0..48 {
        samples.push(step(app));
        if app
            .world
            .resource::<StreamingCoordinator>()
            .region_state(&shared_region)
            == Some(&ribbon_plugin_streaming::RegionState::Unloaded)
        {
            unloaded = true;
            break;
        }
    }
    Ok(unloaded)
}

fn wait_for_assets(
    app: &mut App,
    paths: &[String],
    samples: &mut Vec<FrameSample>,
    max_frames: usize,
) -> Result<bool, Box<dyn Error>> {
    for _ in 0..max_frames {
        let all_loaded = {
            let server = app.world.resource::<AssetServer>();
            paths
                .iter()
                .all(|path| server.state::<BytesAsset>(path) == Some(&LoadState::Loaded))
        };
        if all_loaded {
            return Ok(true);
        }
        samples.push(step(app));
        // Let AssetServer workers run between tight deterministic app-update iterations.
        thread::yield_now();
    }
    Ok(paths.iter().all(|path| {
        app.world
            .resource::<AssetServer>()
            .state::<BytesAsset>(path)
            == Some(&LoadState::Loaded)
    }))
}

fn run_region_unload_cancellation_test(
    app: &mut App,
    samples: &mut Vec<FrameSample>,
) -> Result<bool, Box<dyn Error>> {
    let started = Arc::new(AtomicBool::new(false));
    let gate = Arc::new((Mutex::new(()), Condvar::new()));
    app.world.resource_mut::<AssetServer>().add_source(
        200,
        CancellationGateSource {
            started: Arc::clone(&started),
            gate,
        },
    );

    let region_id = RegionId::new("headless-active-scenedoc-cancel-region");
    let scene_doc_path = "cancel/active_region.scenedoc";
    let center = Vec3::new(500_000.0, 0.0, 0.0);
    let bounds = RegionBounds::new(center - Vec3::splat(4.0), center + Vec3::splat(4.0))?;
    let policy = RegionPolicy {
        preload_distance: 100.0,
        activation_distance: 40.0,
        unload_distance: 140.0,
        base_priority: 200,
        auto_activate: true,
        preview_when_ready: false,
    };
    app.world
        .resource_mut::<StreamingCoordinator>()
        .register_region(
            RegionDefinition::new(
                region_id.clone(),
                "Active SceneDoc cancellation region",
                bounds,
            )
            .with_policy(policy)
            .with_scene_doc(scene_doc_path),
        )?;

    app.world
        .resource_mut::<StreamingCoordinator>()
        .update_observer(StreamObserver::new(91, center))?;
    let start_deadline = Instant::now() + Duration::from_secs(5);
    while !started.load(Ordering::Acquire) && Instant::now() < start_deadline {
        samples.push(step(app));
        if matches!(
            app.world
                .resource::<StreamingCoordinator>()
                .region_state(&region_id),
            Some(ribbon_plugin_streaming::RegionState::Failed(_))
        ) {
            return Ok(false);
        }
        thread::yield_now();
    }
    if !started.load(Ordering::Acquire) {
        return Ok(false);
    }
    let loading_observed = app
        .world
        .resource::<AssetServer>()
        .state::<SceneDoc>(scene_doc_path)
        == Some(&LoadState::Loading);

    // The worker is now blocked inside the real SceneDoc AssetSource read. Moving the observer
    // beyond unload distance must make the coordinator drop its region lease and cancel that IO.
    app.world
        .resource_mut::<StreamingCoordinator>()
        .update_observer(StreamObserver::new(
            91,
            center + Vec3::new(10_000.0, 0.0, 0.0),
        ))?;
    let finish_deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < finish_deadline {
        samples.push(step(app));
        let region_unloaded = app
            .world
            .resource::<StreamingCoordinator>()
            .region_state(&region_id)
            == Some(&ribbon_plugin_streaming::RegionState::Unloaded);
        let workers_drained = app
            .world
            .resource::<AssetServer>()
            .worker_stats()
            .active_workers
            == 0;
        if region_unloaded && workers_drained {
            break;
        }
        thread::yield_now();
    }
    app.world
        .resource_mut::<StreamingCoordinator>()
        .remove_observer(91);

    let server = app.world.resource::<AssetServer>();
    let region_unloaded = app
        .world
        .resource::<StreamingCoordinator>()
        .region_state(&region_id)
        == Some(&ribbon_plugin_streaming::RegionState::Unloaded);
    let cancelled_asset = loading_observed
        && server.state::<SceneDoc>(scene_doc_path) == Some(&LoadState::Unloaded)
        && server.worker_stats().active_workers == 0;
    Ok(region_unloaded && cancelled_asset)
}

fn expected_activation_count(regions: usize, cycles: usize) -> usize {
    regions
        .saturating_mul(cycles)
        .saturating_sub(cycles.saturating_sub(1))
}

fn run_active_cancellation_test(
    app: &mut App,
    samples: &mut Vec<FrameSample>,
) -> Result<(bool, usize), Box<dyn Error>> {
    let started = Arc::new(AtomicBool::new(false));
    let gate = Arc::new((Mutex::new(()), Condvar::new()));
    app.world.resource_mut::<AssetServer>().add_source(
        100,
        CancellationGateSource {
            started: Arc::clone(&started),
            gate,
        },
    );
    let handle = app
        .world
        .resource_mut::<AssetServer>()
        .request::<BytesAsset>("cancel/blocked.bytes")?;

    let start_deadline = Instant::now() + Duration::from_secs(5);
    while !started.load(Ordering::Acquire) && Instant::now() < start_deadline {
        samples.push(step(app));
        thread::yield_now();
    }
    if !started.load(Ordering::Acquire) {
        return Err("cancellation test worker never entered the blocking source read".into());
    }
    let workers_peak = app
        .world
        .resource::<AssetServer>()
        .worker_stats()
        .active_workers;
    let cancelled = app
        .world
        .resource_mut::<AssetServer>()
        .cancel_if_unleased(handle);
    if !cancelled {
        return Ok((false, workers_peak));
    }

    let finish_deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < finish_deadline {
        samples.push(step(app));
        let workers = app
            .world
            .resource::<AssetServer>()
            .worker_stats()
            .active_workers;
        if workers == 0 {
            break;
        }
        thread::yield_now();
    }
    let server = app.world.resource::<AssetServer>();
    let state = server.state::<BytesAsset>("cancel/blocked.bytes");
    let passed = state == Some(&LoadState::Unloaded)
        && server.get(handle).is_none()
        && server.worker_stats().active_workers == 0;
    Ok((passed, workers_peak))
}

fn step(app: &mut App) -> FrameSample {
    app.world.resource_mut::<Time>().delta = FIXED_DELTA;
    let started = Instant::now();
    app.update();
    let wall_ns = started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
    let stream = app
        .world
        .try_resource::<StreamDiagnostics>()
        .cloned()
        .unwrap_or_default();
    let cpu = app.world.resource::<AssetServer>().cpu_memory_stats();
    let workers = app.world.resource::<AssetServer>().worker_stats();
    let concurrent_regions_active = {
        let coordinator = app.world.resource::<StreamingCoordinator>();
        (0..CONCURRENT_REGION_COUNT)
            .filter(|index| {
                coordinator.region_state(&RegionId::new(format!(
                    "headless-concurrent-region-{index:02}"
                ))) == Some(&ribbon_plugin_streaming::RegionState::Active)
            })
            .count()
    };
    FrameSample {
        frame: app.world.resource::<Time>().frame,
        update_wall_ns: wall_ns,
        render_frame_cpu_ns: None,
        streaming_update_ns: stream.last_update_cpu_ns,
        cpu_resident_bytes: cpu.resident_bytes_estimate,
        cpu_pinned_bytes: cpu.pinned_bytes_estimate,
        cpu_unpinned_bytes: cpu.unpinned_bytes_estimate,
        gpu_estimate_bytes: None,
        active_regions: stream.active_regions,
        loading_regions: stream.loading_regions,
        dormant_regions: stream.dormant_regions,
        failed_regions: stream.failed_regions,
        queued_asset_jobs: stream.queued_asset_jobs,
        active_asset_workers: workers.active_workers,
        pending_asset_completions: workers.pending_completions,
        asset_completions_processed_last_poll: workers.completions_processed_last_poll,
        asset_source_read_ns_last_poll: workers.source_read_time_ns_last_poll,
        asset_decode_ns_last_poll: workers.decode_time_ns_last_poll,
        gpu_uploads_this_frame: None,
        gpu_upload_bytes_this_frame: None,
        gpu_frame_count: None,
        gpu_model_uploads_this_frame: None,
        gpu_generic_uploads_this_frame: None,
        gpu_mesh_pass_ns: None,
        gpu_timestamp_queries_supported: None,
        gpu_live_streamed_textures: None,
        gpu_live_streamed_samplers: None,
        gpu_streamed_texture_bytes_estimate: None,
        gpu_generic_resources_released: None,
        concurrent_regions_active: Some(concurrent_regions_active),
    }
}

#[cfg(test)]
mod tests {
    use super::expected_activation_count;

    #[test]
    fn region_route_turnarounds_do_not_double_count_the_endpoint() {
        assert_eq!(expected_activation_count(48, 1), 48);
        assert_eq!(expected_activation_count(48, 2), 95);
        assert_eq!(expected_activation_count(48, 3), 142);
    }
}
