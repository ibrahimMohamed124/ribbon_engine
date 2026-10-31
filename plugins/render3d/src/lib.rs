//! Mesh rendering for the engine: ECS components, an instanced wgpu mesh
//! pipeline with a depth buffer, and a plugin that draws into the window.
//!
//! World space is right-handed and Y-up. Three-dimensional local transforms are
//! owned by `ribbon_scene`; this renderer consumes their resolved global transforms.

pub mod components;
pub mod extract;
pub mod gpu_streaming;
pub mod mesh;
pub mod model;
pub mod post;
pub mod prefab;
pub mod preload;
pub mod profiling;
pub mod renderer;

pub use components::{
    BuiltinMesh, Camera3D, DynamicMeshInstance3D, FogSettings, GpuMeshInstance3D, MeshInstance,
    ModelInstance, PointLight, SpotLight, StagedRender, SunLight, Transform3D,
};
pub use extract::{
    extract_scene_3d, ExtractedCamera3D, ExtractedDynamicMesh3D, ExtractedMesh3D, ExtractedModel3D,
    ExtractedScene3D, RenderExtraction3DPlugin,
};
pub use glam;
pub use gpu_streaming::{
    GpuResidencyBudget, GpuResourceClass, GpuResourceData, GpuResourceHandle, GpuResourceKey,
    GpuResourceLease, GpuResourceRequestError, GpuResourceWarmup, GpuResourceWarmupCounts,
    GpuResourceWarmupStatus, OwnedPbrMaterialData, OwnedTextureData,
};
pub use model::{try_upload_model, upload_model, validate_model_for_upload, GpuModelPart};
pub use pollster;
pub use prefab::register_prefab_components;
pub use preload::{GpuUploadBudget, ModelGpuResidencyBudget, ModelWarmup, ModelWarmupStatus};
pub use renderer::{
    Draw, GpuSamplerAddressMode, GpuSamplerDesc, GpuSamplerFilter, MaterialId, MeshId,
    MeshRenderer, PbrMaterialDesc, SamplerId, Scene, TextureData, TextureId,
};
pub use ribbon_render::gpu::{
    GpuContext, TransientTextureDescriptor, TransientTextureHandle, TransientTexturePool,
};
pub use ribbon_render::gpu_window::clear_view;
pub use ribbon_render::profiling::RenderResourceStats;
pub use ribbon_render::{graph::RenderError, Framebuffer};
pub use ribbon_scene::GlobalTransform3D;
pub use wgpu;

use crate::mesh::MeshData;
use crate::model::upload_validated_model;
use crate::post::{ToneMapper, HDR_FORMAT};
use crate::preload::{model_key, ModelWarmupCounts, ModelWarmupSnapshot};
use glam::Mat4;
use ribbon_app::World;
use ribbon_assets::{AssetServer, LoadState};
use ribbon_render::gpu_window::{ClearColor, FramePass, FrameTarget, GpuWindow};
use ribbon_render::profiling::{
    RenderDiagnostics, RenderGpuTimingSample, RenderGpuTimingSummary, RenderPassTiming,
    RenderProfileRecorder,
};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::Instant;

type ModelAssetKey = (u64, u32);
type ModelAssetRevision = (ModelAssetKey, u64);
type GpuValidationFuture = Pin<Box<dyn Future<Output = Option<wgpu::Error>> + Send + 'static>>;

fn borrow_owned_texture<'a>(
    source: &'a Option<crate::OwnedTextureData>,
) -> Option<TextureData<'a>> {
    source.as_ref().map(|image| TextureData {
        width: image.width,
        height: image.height,
        rgba8: image.rgba8.as_slice(),
    })
}

struct PendingModelValidation {
    handle: ribbon_assets::Handle<ribbon_plugin_gltf::ModelAsset>,
    path: Option<String>,
    future: GpuValidationFuture,
}

struct PendingGpuResourceValidation {
    handle: GpuResourceHandle,
    future: GpuValidationFuture,
}

/// Shared per-frame admission counters and limits for visible-scene uploads and background
/// warmups. Keeping them together ensures both paths consume the same budget.
struct ModelUploadFrameBudget {
    uploads: usize,
    uploaded_bytes: u64,
    limits: GpuUploadBudget,
}

struct CachedDynamicMesh {
    // Retaining the source Arc prevents allocator address reuse while the pointer is a cache key.
    _source: Arc<MeshData>,
    mesh: MeshId,
}

/// Compute GPU-cache liveness from runtime use, not CPU asset residency. A visible placement may
/// continue using an already-uploaded model even if its CPU representation is temporarily absent,
/// while a non-failed warmup request must retain the cache during the transition handoff.
fn collect_live_model_keys(
    extracted: &ExtractedScene3D,
    warmup_snapshots: &[ModelWarmupSnapshot],
) -> HashSet<ModelAssetKey> {
    extracted
        .models
        .iter()
        .map(|placement| placement.model)
        .chain(warmup_snapshots.iter().filter_map(|request| {
            (!matches!(&request.status, ModelWarmupStatus::Failed(_))).then_some(request.handle)
        }))
        .map(model_key)
        .collect()
}

#[derive(Default)]
struct MeshPass {
    renderer: Option<MeshRenderer>,
    /// Active HDR target leased from the transient texture pool.
    hdr: Option<TransientTextureHandle>,
    hdr_size: Option<(u32, u32)>,
    transient_targets: TransientTexturePool,
    tonemapper: Option<(wgpu::TextureFormat, ToneMapper)>,
    /// Uploaded models keyed by asset handle id and generation.
    models: HashMap<ModelAssetKey, Vec<GpuModelPart>>,
    /// Upload-size estimate for each live glTF model cache entry; subset of renderer mesh/material totals.
    model_cache_bytes: HashMap<ModelAssetKey, u64>,
    model_cache_pressure_events: u64,
    model_cache_pressure_level: u8,
    /// Last CPU-content revision observed for each model handle.
    asset_revisions: HashMap<ModelAssetKey, u64>,
    /// GPU uploads for shared runtime-provided mesh geometry, keyed by the retained Arc allocation.
    dynamic_meshes: HashMap<usize, CachedDynamicMesh>,
    model_gpu_uploads_cumulative: u64,
    model_gpu_releases_cumulative: u64,
    model_gpu_uploads_this_frame: usize,
    model_gpu_upload_bytes_this_frame: u64,
    generic_gpu_uploads_this_frame: usize,
    generic_gpu_upload_bytes_this_frame: u64,
    // Pop-error-scope futures are Send but not guaranteed Sync; the mutex keeps FramePass Sync.
    pending_model_validations: Mutex<HashMap<ModelAssetRevision, PendingModelValidation>>,
    pending_gpu_resource_validations: Mutex<HashMap<GpuResourceKey, PendingGpuResourceValidation>>,
    failed_model_uploads: HashMap<ModelAssetRevision, String>,
    cpu_phases: Vec<RenderPassTiming>,
}

impl FramePass for MeshPass {
    fn name(&self) -> &'static str {
        "mesh pass"
    }

    fn on_device_lost(&mut self, world: &mut World) {
        // Drop renderer/pipeline/texture objects on the render-owning thread before GpuContext.
        self.renderer = None;
        self.hdr = None;
        self.hdr_size = None;
        self.tonemapper = None;
        self.transient_targets = TransientTexturePool::default();
        self.models.clear();
        self.model_cache_bytes.clear();
        self.model_cache_pressure_level = 0;
        self.asset_revisions.clear();
        self.dynamic_meshes.clear();
        self.failed_model_uploads.clear();
        self.pending_model_validations
            .get_mut()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
        self.pending_gpu_resource_validations
            .get_mut()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
        if world.try_resource::<ModelWarmup>().is_some() {
            world
                .resource_mut::<ModelWarmup>()
                .reset_after_device_loss();
        }
        if world.try_resource::<GpuResourceWarmup>().is_some() {
            world
                .resource_mut::<GpuResourceWarmup>()
                .reset_after_device_loss();
        }
    }

    fn render(&mut self, world: &mut World, target: &FrameTarget<'_>) {
        // Temporarily borrow the snapshot so GPU submission can read AssetServer without
        // cloning the extracted vectors. `resource_scope` restores the snapshot even on unwind.
        if world.try_resource::<ExtractedScene3D>().is_none() {
            return;
        }
        self.cpu_phases.clear();
        let profile_cpu = world.try_resource::<RenderProfileRecorder>().is_some();
        let frame = world
            .try_resource::<RenderDiagnostics>()
            .map_or(1, |diagnostics| diagnostics.frame_count.saturating_add(1));
        let poll_started = profile_cpu.then(Instant::now);
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.poll_gpu_timing(target.device, target.queue);
        }
        if let Some((_, tonemapper)) = self.tonemapper.as_mut() {
            tonemapper.poll_gpu_timing(target.device, target.queue);
        }
        record_cpu_phase(&mut self.cpu_phases, "GPU timestamp polling", poll_started);
        world.resource_scope::<ExtractedScene3D, _>(|world, extracted| {
            self.render_extracted(world, extracted, target, profile_cpu, frame);
        });
        self.publish_diagnostics(world, target.device);
    }
}

impl MeshPass {
    fn publish_diagnostics(&mut self, world: &mut World, device: &wgpu::Device) {
        let renderer_stats = self
            .renderer
            .as_ref()
            .map(|renderer| renderer.resource_stats());
        let gpu_timing = self
            .renderer
            .as_ref()
            .map(|renderer| renderer.gpu_timing_snapshot())
            .unwrap_or_else(|| crate::profiling::GpuTimingSnapshot {
                supported: device.features().contains(wgpu::Features::TIMESTAMP_QUERY),
                ..crate::profiling::GpuTimingSnapshot::default()
            });
        let post_gpu_timing = self
            .tonemapper
            .as_ref()
            .map(|(_, tonemapper)| tonemapper.gpu_timing_snapshot())
            .unwrap_or_default();
        let frame = world
            .try_resource::<RenderDiagnostics>()
            .map_or(0, |diagnostics| diagnostics.frame_count.saturating_add(1));
        let capture_gpu_samples = world.try_resource::<RenderProfileRecorder>().is_some();
        let mut completed_gpu_samples = Vec::new();
        if let Some(renderer) = self.renderer.as_mut() {
            if capture_gpu_samples {
                completed_gpu_samples.extend(renderer.drain_gpu_timing_samples().into_iter().map(
                    |sample| RenderGpuTimingSample {
                        source: "shadow_mesh",
                        sequence: sample.sequence,
                        submitted_frame: sample.submitted_frame,
                        observed_frame: frame,
                        total_ns: sample.total_ns,
                        shadow_ns: sample.shadow_ns,
                        mesh_ns: sample.mesh_ns,
                        bloom_ns: None,
                        tone_mapping_ns: None,
                    },
                ));
            } else {
                renderer.clear_gpu_timing_samples();
            }
        }
        if let Some((_, tonemapper)) = self.tonemapper.as_mut() {
            if capture_gpu_samples {
                completed_gpu_samples.extend(
                    tonemapper
                        .drain_gpu_timing_samples()
                        .into_iter()
                        .map(|sample| RenderGpuTimingSample {
                            source: "bloom_tone_mapping",
                            sequence: sample.sequence,
                            submitted_frame: sample.submitted_frame,
                            observed_frame: frame,
                            total_ns: sample.total_ns,
                            shadow_ns: None,
                            mesh_ns: None,
                            bloom_ns: sample.shadow_ns,
                            tone_mapping_ns: sample.mesh_ns,
                        }),
                );
            } else {
                tonemapper.clear_gpu_timing_samples();
            }
        }
        if world.try_resource::<RenderProfileRecorder>().is_some() {
            if let Some(duration_ns) = world
                .try_resource::<ribbon_scene::TransformPropagationDiagnostics>()
                .filter(|diagnostics| diagnostics.sample_pending)
                .map(|diagnostics| diagnostics.last_3d_duration_ns)
            {
                self.cpu_phases.push(RenderPassTiming {
                    name: "3D transform propagation (one-shot)".to_owned(),
                    cpu_time_ns: duration_ns,
                });
            }
        }
        let pool_stats = self.transient_targets.stats();
        let post_process_bytes = self
            .tonemapper
            .as_ref()
            .map(|(_, tonemapper)| tonemapper.estimated_bytes())
            .unwrap_or(0);
        let mut resources = RenderResourceStats {
            active_transient_textures: pool_stats.active_textures,
            cached_transient_textures: pool_stats.cached_textures,
            active_transient_bytes_estimate: pool_stats.active_bytes_estimate,
            cached_transient_bytes_estimate: pool_stats.cached_bytes_estimate,
            post_process_bytes_estimate: post_process_bytes,
            transient_texture_allocations: pool_stats.allocations,
            transient_texture_reuses: pool_stats.reuses,
            transient_pool_peak_bytes_estimate: pool_stats.peak_allocated_bytes_estimate,
            transient_unknown_size_textures: pool_stats.unknown_size_textures,
            ..RenderResourceStats::default()
        };
        if let Some(extracted) = world.try_resource::<ExtractedScene3D>() {
            resources.scene_mesh_instances = extracted
                .meshes
                .len()
                .saturating_add(extracted.dynamic_meshes.len())
                .saturating_add(extracted.gpu_meshes.len());
            resources.scene_model_placements = extracted.models.len();
        }
        resources.gpu_model_cache_entries = self.models.len();
        let model_cache_bytes = self
            .model_cache_bytes
            .values()
            .fold(0_u64, |total, bytes| total.saturating_add(*bytes));
        let model_budget = world
            .try_resource::<ModelGpuResidencyBudget>()
            .copied()
            .unwrap_or_default()
            .normalized();
        let pressure_level = model_budget.pressure_level(model_cache_bytes);
        if pressure_level > self.model_cache_pressure_level {
            self.model_cache_pressure_events = self.model_cache_pressure_events.saturating_add(1);
        }
        self.model_cache_pressure_level = pressure_level;
        resources.model_gpu_cache_bytes_estimate = model_cache_bytes;
        resources.model_gpu_cache_budget_bytes = model_budget.max_bytes.unwrap_or(0);
        resources.model_gpu_cache_over_budget_bytes = model_budget
            .max_bytes
            .map_or(0, |limit| model_cache_bytes.saturating_sub(limit));
        resources.model_gpu_cache_pressure_events = self.model_cache_pressure_events;
        resources.model_gpu_cache_pressure_level = pressure_level;
        resources.model_gpu_uploads_cumulative = self.model_gpu_uploads_cumulative;
        resources.model_gpu_releases_cumulative = self.model_gpu_releases_cumulative;
        resources.model_gpu_uploads_this_frame = self.model_gpu_uploads_this_frame;
        resources.model_gpu_upload_bytes_this_frame = self.model_gpu_upload_bytes_this_frame;
        if let Some(warmup) = world.try_resource::<ModelWarmup>() {
            let counts: ModelWarmupCounts = warmup.counts();
            resources.model_warmup_requested = counts.requested;
            resources.model_warmup_waiting_for_cpu = counts.waiting_for_cpu;
            resources.model_warmup_waiting_for_gpu = counts.waiting_for_gpu;
            resources.model_warmup_ready = counts.ready;
            resources.model_warmup_failed = counts.failed;
        }
        if let Some(warmup) = world.try_resource::<GpuResourceWarmup>() {
            let counts = warmup.counts();
            resources.generic_gpu_warmup_queued = counts.queued;
            resources.generic_gpu_warmup_preparing = counts.preparing;
            resources.generic_gpu_warmup_ready = counts.ready;
            resources.generic_gpu_warmup_failed = counts.failed;
            resources.generic_gpu_uploads_cumulative = counts.completed_uploads;
            resources.generic_gpu_upload_failures_cumulative = counts.failed_uploads;
            resources.generic_gpu_releases_cumulative = counts.released_resources;
            resources.generic_gpu_resident_bytes_estimate = counts.resident_bytes_estimate;
            resources.generic_gpu_recovery_payload_bytes_estimate =
                counts.recovery_payload_bytes_estimate;
            resources.generic_gpu_queued_bytes_estimate = counts.queued_bytes_estimate;
            resources.generic_gpu_budget_overage_bytes = counts.budget_overage_bytes;
            resources.generic_gpu_evictions_cumulative = counts.evicted_resources;
            resources.generic_gpu_memory_pressure_events = counts.memory_pressure_events;
            resources.generic_gpu_budget_rejections_cumulative = counts.budget_rejections;
            resources.generic_gpu_sampler_count_over_budget = counts.sampler_count_over_budget;
        }
        resources.generic_gpu_uploads_this_frame = self.generic_gpu_uploads_this_frame;
        resources.generic_gpu_upload_bytes_this_frame = self.generic_gpu_upload_bytes_this_frame;
        if let Some(stats) = renderer_stats {
            resources.live_meshes = stats.live_meshes;
            resources.live_materials = stats.live_materials;
            resources.live_streamed_textures = stats.live_standalone_textures;
            resources.live_streamed_samplers = stats.live_samplers;
            resources.streamed_texture_bytes_estimate = stats.standalone_texture_bytes_estimate;
            resources.visible_mesh_instances = stats.visible_mesh_instances;
            resources.visible_draw_calls = stats.visible_draw_calls;
            resources.shadow_draw_calls = stats.shadow_draw_calls;
            resources.mesh_bytes_estimate = stats.mesh_bytes_estimate;
            resources.material_bytes_estimate = stats.material_bytes_estimate;
            resources.live_material_textures = stats.live_material_textures;
            resources.material_texture_bytes_estimate = stats.material_texture_bytes_estimate;
            resources.material_uniform_bytes_estimate = stats.material_uniform_bytes_estimate;
            resources.material_texture_cache_hits = stats.material_texture_cache_hits;
            resources.material_texture_cache_misses = stats.material_texture_cache_misses;
            resources.buffer_bytes_estimate = stats.buffer_bytes_estimate;
            resources.depth_target_bytes_estimate = stats.depth_target_bytes_estimate;
            resources.backend_peak_bytes_estimate = stats.peak_bytes_estimate;
            resources.renderer_instance_capacity = stats.instance_capacity;
            resources.renderer_instance_buffer_grows_cumulative =
                stats.instance_buffer_grows_cumulative;
            resources.renderer_instance_buffer_shrinks_cumulative =
                stats.instance_buffer_shrinks_cumulative;
        }
        if world.try_resource::<RenderDiagnostics>().is_none() {
            world.insert_resource(RenderDiagnostics::default());
        }
        if let Some(diagnostics) = world.try_resource::<RenderDiagnostics>() {
            resources.gpu_device_loss_events = diagnostics.gpu_device_loss_events;
            resources.gpu_device_recovery_attempts = diagnostics.gpu_device_recovery_attempts;
            resources.gpu_device_recovery_successes = diagnostics.gpu_device_recovery_successes;
            resources.gpu_device_recovery_failures = diagnostics.gpu_device_recovery_failures;
            resources.gpu_last_device_loss_message =
                diagnostics.gpu_last_device_loss_message.clone();
        }
        {
            let diagnostics = world.resource_mut::<RenderDiagnostics>();
            diagnostics.completed_gpu_samples = completed_gpu_samples;
            diagnostics.cpu_phases = self.cpu_phases.clone();
            diagnostics.gpu_samples_pending = gpu_timing
                .pending_samples
                .saturating_add(post_gpu_timing.pending_samples);
            diagnostics.gpu_samples_dropped = gpu_timing
                .dropped_samples
                .saturating_add(post_gpu_timing.dropped_samples);
            if !gpu_timing.supported {
                diagnostics.gpu_samples_unavailable =
                    diagnostics.gpu_samples_unavailable.saturating_add(1);
            }
            if !post_gpu_timing.supported {
                diagnostics.gpu_samples_unavailable =
                    diagnostics.gpu_samples_unavailable.saturating_add(1);
            }
        }
        // CPU cache accounting scans resident asset metadata. Keep it off the normal render path
        // and collect it only when an explicit bounded profile capture is installed.
        if world.try_resource::<RenderProfileRecorder>().is_some() {
            if let Some(server) = world.try_resource::<AssetServer>() {
                let stats = server.cpu_memory_stats();
                let worker_stats = server.worker_stats();
                resources.asset_worker_source_read_ns_last_poll =
                    worker_stats.source_read_time_ns_last_poll;
                resources.asset_worker_decode_ns_last_poll = worker_stats.decode_time_ns_last_poll;
                resources.asset_worker_completions_processed_last_poll =
                    worker_stats.completions_processed_last_poll;
                resources.asset_worker_pending_completions = worker_stats.pending_completions;
                resources.cpu_asset_resident_bytes_estimate = stats.resident_bytes_estimate;
                resources.cpu_asset_geometry_bytes_estimate = stats.geometry_bytes_estimate;
                resources.cpu_asset_texture_bytes_estimate = stats.texture_bytes_estimate;
                resources.cpu_asset_material_bytes_estimate = stats.material_bytes_estimate;
                resources.cpu_asset_animation_bytes_estimate = stats.animation_bytes_estimate;
                resources.cpu_asset_scene_metadata_bytes_estimate =
                    stats.scene_metadata_bytes_estimate;
                resources.cpu_asset_other_bytes_estimate = stats.other_bytes_estimate;
                resources.cpu_asset_pinned_bytes_estimate = stats.pinned_bytes_estimate;
                resources.cpu_asset_unpinned_bytes_estimate = stats.unpinned_bytes_estimate;
                resources.cpu_asset_loaded_count = stats.loaded_assets;
                resources.cpu_asset_pinned_count = stats.pinned_assets;
                resources.cpu_asset_budget_enabled = stats.budget_bytes.is_some();
                resources.cpu_asset_budget_bytes = stats.budget_bytes.unwrap_or(0);
                resources.cpu_asset_over_budget_bytes = stats.over_budget_bytes;
                resources.cpu_asset_budget_evictions_cumulative = stats.budget_evictions_cumulative;
            }
        }
        if world.try_resource::<RenderDiagnostics>().is_some() {
            let diagnostics = world.resource_mut::<RenderDiagnostics>();
            diagnostics.record_gpu_timing(RenderGpuTimingSummary {
                supported: gpu_timing.supported || post_gpu_timing.supported,
                latest_total_ns: gpu_timing.latest_duration_ns,
                latest_shadow_ns: gpu_timing.latest_shadow_pass_ns,
                latest_mesh_ns: gpu_timing.latest_mesh_pass_ns,
                mesh_sequence: gpu_timing.latest_sequence,
                latest_bloom_ns: post_gpu_timing.latest_shadow_pass_ns,
                latest_tone_mapping_ns: post_gpu_timing.latest_mesh_pass_ns,
                post_sequence: post_gpu_timing.latest_sequence,
                successful_samples: gpu_timing
                    .completed_samples
                    .saturating_add(post_gpu_timing.completed_samples),
                skipped_samples: gpu_timing
                    .skipped_samples
                    .saturating_add(post_gpu_timing.skipped_samples),
            });
            diagnostics.record_resources(resources);
        }
    }

    fn release_hdr_target(&mut self) {
        // Drop bind groups that reference the HDR view before making its texture reusable.
        self.tonemapper = None;
        self.hdr_size = None;
        if let Some(handle) = self.hdr.take() {
            self.transient_targets.release(handle);
        }
    }

    fn set_warmup_status(
        world: &mut World,
        handle: ribbon_assets::Handle<ribbon_plugin_gltf::ModelAsset>,
        status: ModelWarmupStatus,
    ) {
        if world.try_resource::<ModelWarmup>().is_some() {
            let warmup = world.resource_mut::<ModelWarmup>();
            let _ = warmup.set_status(handle, status);
        }
    }

    fn poll_pending_model_validations(&mut self, world: &mut World, device: &wgpu::Device) {
        if self
            .pending_model_validations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_empty()
        {
            return;
        }
        let _ = device.poll(wgpu::PollType::Poll);
        let waker = Waker::noop();
        let mut completed = Vec::new();
        {
            let mut pending = self
                .pending_model_validations
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            for (revision, entry) in pending.iter_mut() {
                let mut context = Context::from_waker(waker);
                if let Poll::Ready(gpu_error) = entry.future.as_mut().poll(&mut context) {
                    completed.push((
                        *revision,
                        entry.handle,
                        entry.path.clone(),
                        gpu_error.map(|error| error.to_string()),
                    ));
                }
            }
            for (revision, _, _, _) in &completed {
                pending.remove(revision);
            }
        }

        for (revision @ (key, cpu_revision), handle, path, error) in completed {
            if let Some(error) = error {
                if let Some(parts) = self.models.remove(&key) {
                    self.model_cache_bytes.remove(&key);
                    self.model_gpu_releases_cumulative =
                        self.model_gpu_releases_cumulative.saturating_add(1);
                    if let Some(renderer) = self.renderer.as_mut() {
                        for part in parts {
                            renderer.release_mesh(part.mesh);
                            renderer.release_material(part.material);
                        }
                    }
                }
                self.failed_model_uploads.insert(revision, error.clone());
                let message = match path.as_deref() {
                    Some(path) => format!("GPU resource upload failed for `{path}`: {error}"),
                    None => format!("GPU resource upload failed for model handle {key:?}: {error}"),
                };
                eprintln!("render3d model warmup failed: {message}");
                Self::set_warmup_status(world, handle, ModelWarmupStatus::Failed(message));
            } else if self.asset_revisions.get(&key) == Some(&cpu_revision)
                && self.models.contains_key(&key)
            {
                Self::set_warmup_status(world, handle, ModelWarmupStatus::Ready);
            }
        }
    }

    fn poll_pending_gpu_resource_validations(&mut self, world: &mut World, device: &wgpu::Device) {
        let has_pending = !self
            .pending_gpu_resource_validations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_empty();
        if has_pending {
            let _ = device.poll(wgpu::PollType::Poll);
        }
        let waker = Waker::noop();
        let mut completed = Vec::new();
        {
            let mut pending = self
                .pending_gpu_resource_validations
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            for (key, entry) in pending.iter_mut() {
                let mut context = Context::from_waker(waker);
                if let Poll::Ready(error) = entry.future.as_mut().poll(&mut context) {
                    completed.push((*key, entry.handle, error.map(|value| value.to_string())));
                }
            }
            for (key, _, _) in &completed {
                pending.remove(key);
            }
        }
        for (key, handle, error) in completed {
            if let Some(error) = error {
                if let Some(renderer) = self.renderer.as_mut() {
                    match handle {
                        GpuResourceHandle::Mesh(mesh) => {
                            renderer.release_mesh(mesh);
                        }
                        GpuResourceHandle::Texture(texture) => {
                            renderer.release_texture(texture);
                        }
                        GpuResourceHandle::Sampler(sampler) => {
                            renderer.release_sampler(sampler);
                        }
                        GpuResourceHandle::Material(material) => {
                            renderer.release_material(material);
                        }
                    }
                }
                if world.try_resource::<GpuResourceWarmup>().is_some() {
                    world
                        .resource_mut::<GpuResourceWarmup>()
                        .fail(key, format!("GPU resource validation failed: {error}"));
                }
            } else if world.try_resource::<GpuResourceWarmup>().is_some() {
                world
                    .resource_mut::<GpuResourceWarmup>()
                    .complete(key, handle);
            }
        }

        // Lease expiry is observed on the render thread so GPU destruction stays with its owner.
        let orphaned = if world.try_resource::<GpuResourceWarmup>().is_some() {
            world.resource_mut::<GpuResourceWarmup>().collect_orphans()
        } else {
            Vec::new()
        };
        if let Some(renderer) = self.renderer.as_mut() {
            for handle in orphaned {
                match handle {
                    GpuResourceHandle::Mesh(mesh) => {
                        renderer.release_mesh(mesh);
                    }
                    GpuResourceHandle::Texture(texture) => {
                        renderer.release_texture(texture);
                    }
                    GpuResourceHandle::Sampler(sampler) => {
                        renderer.release_sampler(sampler);
                    }
                    GpuResourceHandle::Material(material) => {
                        renderer.release_material(material);
                    }
                }
            }
        }
    }

    fn process_generic_gpu_warmups(
        &mut self,
        world: &mut World,
        target: &FrameTarget<'_>,
        frame_budget: &mut ModelUploadFrameBudget,
        profile_cpu: bool,
    ) {
        while let Some(next_bytes) = world
            .try_resource::<GpuResourceWarmup>()
            .and_then(GpuResourceWarmup::next_upload_bytes)
        {
            if !frame_budget.limits.allows_upload(
                frame_budget.uploads,
                frame_budget.uploaded_bytes,
                next_bytes,
            ) {
                break;
            }
            let job = if world.try_resource::<GpuResourceWarmup>().is_some() {
                world.resource_mut::<GpuResourceWarmup>().take_next_job()
            } else {
                None
            };
            let Some(job) = job else { break };
            let started = profile_cpu.then(Instant::now);
            if self.renderer.is_none() {
                self.renderer = Some(MeshRenderer::new(target.device, target.queue, HDR_FORMAT));
            }
            let Some(renderer) = self.renderer.as_mut() else {
                if world.try_resource::<GpuResourceWarmup>().is_some() {
                    world
                        .resource_mut::<GpuResourceWarmup>()
                        .fail(job.key, "renderer could not be initialized".to_owned());
                }
                continue;
            };
            target
                .device
                .push_error_scope(wgpu::ErrorFilter::OutOfMemory);
            target
                .device
                .push_error_scope(wgpu::ErrorFilter::Validation);
            let handle = match job.payload.as_ref() {
                GpuResourceData::Mesh { vertices, indices } => {
                    GpuResourceHandle::Mesh(renderer.upload_mesh(target.device, vertices, indices))
                }
                GpuResourceData::Texture { image, srgb } => {
                    GpuResourceHandle::Texture(renderer.upload_texture(
                        target.device,
                        target.queue,
                        TextureData {
                            width: image.width,
                            height: image.height,
                            rgba8: image.rgba8.as_slice(),
                        },
                        "streamed standalone texture",
                        *srgb,
                    ))
                }
                GpuResourceData::Sampler(sampler) => {
                    GpuResourceHandle::Sampler(renderer.upload_sampler(target.device, *sampler))
                }
                GpuResourceData::PbrMaterial(material) => {
                    GpuResourceHandle::Material(renderer.upload_pbr_material_with_maps(
                        target.device,
                        target.queue,
                        PbrMaterialDesc {
                            base_color: material.base_color,
                            metallic: material.metallic,
                            roughness: material.roughness,
                            base_color_texture: borrow_owned_texture(&material.base_color_texture),
                            metallic_roughness_texture: borrow_owned_texture(
                                &material.metallic_roughness_texture,
                            ),
                            normal_texture: borrow_owned_texture(&material.normal_texture),
                            occlusion_texture: borrow_owned_texture(&material.occlusion_texture),
                            occlusion_strength: material.occlusion_strength,
                            emissive_texture: borrow_owned_texture(&material.emissive_texture),
                            emissive_factor: material.emissive_factor,
                        },
                    ))
                }
            };
            let validation_result = target.device.pop_error_scope();
            let out_of_memory_result = target.device.pop_error_scope();
            let future: GpuValidationFuture = Box::pin(async move {
                let validation_error = validation_result.await;
                let out_of_memory_error = out_of_memory_result.await;
                validation_error.or(out_of_memory_error)
            });
            self.pending_gpu_resource_validations
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(job.key, PendingGpuResourceValidation { handle, future });
            frame_budget.uploads = frame_budget.uploads.saturating_add(1);
            frame_budget.uploaded_bytes = frame_budget
                .uploaded_bytes
                .saturating_add(job.estimated_bytes);
            self.generic_gpu_uploads_this_frame =
                self.generic_gpu_uploads_this_frame.saturating_add(1);
            self.generic_gpu_upload_bytes_this_frame = self
                .generic_gpu_upload_bytes_this_frame
                .saturating_add(job.estimated_bytes);
            record_cpu_phase(
                &mut self.cpu_phases,
                "generic GPU resource upload submission",
                started,
            );
        }
    }

    fn model_upload_pending(&self, revision: ModelAssetRevision) -> bool {
        self.pending_model_validations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains_key(&revision)
    }

    fn queue_model_upload(
        &mut self,
        world: &mut World,
        server: &AssetServer,
        target: &FrameTarget<'_>,
        handle: ribbon_assets::Handle<ribbon_plugin_gltf::ModelAsset>,
        path: Option<String>,
        frame_budget: &mut ModelUploadFrameBudget,
    ) -> Option<ModelAssetRevision> {
        let key = model_key(handle);
        let revision = server.revision(handle)?;
        let revision_key = (key, revision);
        if self.models.contains_key(&key) || self.failed_model_uploads.contains_key(&revision_key) {
            return Some(revision_key);
        }
        let upload_budget = frame_budget.limits.normalized();
        if !upload_budget.allows_upload(frame_budget.uploads, frame_budget.uploaded_bytes, 0) {
            Self::set_warmup_status(world, handle, ModelWarmupStatus::WaitingForGpu);
            return Some(revision_key);
        }
        let model = server.get(handle)?;
        let estimated_upload_bytes = model.estimated_gpu_upload_bytes();
        if !upload_budget.allows_upload(
            frame_budget.uploads,
            frame_budget.uploaded_bytes,
            estimated_upload_bytes,
        ) {
            Self::set_warmup_status(world, handle, ModelWarmupStatus::WaitingForGpu);
            return Some(revision_key);
        }
        if let Err(error) = validate_model_for_upload(model) {
            self.failed_model_uploads
                .insert(revision_key, error.clone());
            let message = match path.as_deref() {
                Some(path) => format!("model `{path}` is not renderable: {error}"),
                None => format!("model handle {key:?} is not renderable: {error}"),
            };
            eprintln!("render3d model warmup failed: {message}");
            if world.try_resource::<ModelWarmup>().is_some() {
                let warmup = world.resource_mut::<ModelWarmup>();
                let _ = warmup.set_status(handle, ModelWarmupStatus::Failed(message));
            }
            return Some(revision_key);
        }
        // This nested phase measures CPU-side GPU object creation and command submission, not
        // asynchronous GPU execution. It is only sampled when a bounded render profile is active.
        let gpu_resource_creation_started = world
            .try_resource::<RenderProfileRecorder>()
            .is_some()
            .then(Instant::now);
        if self.renderer.is_none() {
            self.renderer = Some(MeshRenderer::new(target.device, target.queue, HDR_FORMAT));
        }
        let renderer = self.renderer.as_mut()?;
        target
            .device
            .push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        target
            .device
            .push_error_scope(wgpu::ErrorFilter::Validation);
        let parts = upload_validated_model(renderer, target.device, target.queue, model);
        let validation_result = target.device.pop_error_scope();
        let out_of_memory_result = target.device.pop_error_scope();
        let future: GpuValidationFuture = Box::pin(async move {
            // Poll both popped scopes: short-circuiting on a validation error would otherwise
            // leave the OOM scope result future unobserved.
            let validation_error = validation_result.await;
            let out_of_memory_error = out_of_memory_result.await;
            validation_error.or(out_of_memory_error)
        });
        self.models.insert(key, parts);
        self.model_cache_bytes.insert(key, estimated_upload_bytes);
        self.asset_revisions.insert(key, revision);
        self.model_gpu_uploads_cumulative = self.model_gpu_uploads_cumulative.saturating_add(1);
        frame_budget.uploads = frame_budget.uploads.saturating_add(1);
        frame_budget.uploaded_bytes = frame_budget
            .uploaded_bytes
            .saturating_add(estimated_upload_bytes);
        self.pending_model_validations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(
                revision_key,
                PendingModelValidation {
                    handle,
                    path,
                    future,
                },
            );
        if world.try_resource::<ModelWarmup>().is_some() {
            let warmup = world.resource_mut::<ModelWarmup>();
            let _ = warmup.set_status(handle, ModelWarmupStatus::WaitingForGpu);
        }
        record_cpu_phase(
            &mut self.cpu_phases,
            "model GPU resource creation/submission",
            gpu_resource_creation_started,
        );
        Some(revision_key)
    }

    fn render_extracted(
        &mut self,
        world: &mut World,
        extracted: &mut ExtractedScene3D,
        target: &FrameTarget<'_>,
        profile_cpu: bool,
        frame: u64,
    ) {
        // Diagnostics describe this render invocation, including early-return frames (e.g. a
        // temporarily missing camera or a minimized window). Never leak the previous frame's
        // upload count into the current profile row.
        self.model_gpu_uploads_this_frame = 0;
        self.model_gpu_upload_bytes_this_frame = 0;
        self.generic_gpu_uploads_this_frame = 0;
        self.generic_gpu_upload_bytes_this_frame = 0;
        let (width, height) = target.size;
        if let Some(cpu_time_ns) = extracted.extraction_cpu_time_ns {
            self.cpu_phases.push(RenderPassTiming {
                name: "scene extraction".to_owned(),
                cpu_time_ns,
            });
        }
        let cache_started = profile_cpu.then(Instant::now);
        self.poll_pending_model_validations(world, target.device);
        self.poll_pending_gpu_resource_validations(world, target.device);
        let mut warmup_snapshots = world
            .try_resource::<ModelWarmup>()
            .map(ModelWarmup::snapshots)
            .unwrap_or_default();

        // Both real scene placements and explicit warmup requests retain a resident model cache
        // entry. Cache liveness is based on GPU use/request ownership, not `AssetServer::get`: CPU
        // residency can be evicted independently of GPU resources already used by a visible scene.
        // Warmup requests remain live until gameplay releases them or records a terminal failure.
        let live_model_keys = collect_live_model_keys(extracted, &warmup_snapshots);
        let current_revisions: Vec<ModelAssetRevision> = world
            .try_resource::<AssetServer>()
            .map(|server| {
                extracted
                    .models
                    .iter()
                    .map(|placement| placement.model)
                    .chain(warmup_snapshots.iter().filter_map(|request| {
                        (!matches!(&request.status, ModelWarmupStatus::Failed(_)))
                            .then_some(request.handle)
                    }))
                    .filter_map(|handle| {
                        server
                            .revision(handle)
                            .map(|revision| (model_key(handle), revision))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let revision_changed = current_revisions.iter().any(|(key, revision)| {
            self.asset_revisions
                .get(key)
                .is_some_and(|previous| previous != revision)
        });
        if revision_changed {
            self.model_gpu_releases_cumulative = self
                .model_gpu_releases_cumulative
                .saturating_add(self.models.len() as u64);
            // A model hot reload invalidates model-derived buffers, not unrelated generic GPU
            // resources. Keep the MeshRenderer (and its independent mesh/material handles) alive
            // while releasing every old model/dynamic-mesh allocation on the renderer thread.
            if let Some(renderer) = self.renderer.as_mut() {
                for (_, parts) in self.models.drain() {
                    for part in parts {
                        renderer.release_mesh(part.mesh);
                        renderer.release_material(part.material);
                    }
                }
                for (_, cached) in self.dynamic_meshes.drain() {
                    renderer.release_mesh(cached.mesh);
                }
            } else {
                self.models.clear();
                self.dynamic_meshes.clear();
            }
            self.model_cache_bytes.clear();
            self.model_cache_pressure_level = 0;
            self.asset_revisions.clear();
            self.pending_model_validations
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clear();
            self.failed_model_uploads.clear();
            self.release_hdr_target();
            // A hot-reloaded model revision invalidates derived GPU state for the pass. Any other
            // still-requested, resident warmup must return to GPU-pending rather than falsely
            // staying Ready after the renderer/cache reset.
            for request in &mut warmup_snapshots {
                if matches!(&request.status, ModelWarmupStatus::Ready) {
                    request.status = ModelWarmupStatus::WaitingForGpu;
                    if world.try_resource::<ModelWarmup>().is_some() {
                        let warmup = world.resource_mut::<ModelWarmup>();
                        let _ = warmup.set_status(request.handle, ModelWarmupStatus::WaitingForGpu);
                    }
                }
            }
        }
        for (key, revision) in current_revisions {
            self.asset_revisions.insert(key, revision);
        }
        self.asset_revisions
            .retain(|key, _| live_model_keys.contains(key));
        // `ModelWarmup::request` resets a terminal failure only when gameplay explicitly retries.
        // A non-failed snapshot therefore authorizes one retry for that exact asset revision.
        for request in &warmup_snapshots {
            if !matches!(&request.status, ModelWarmupStatus::Failed(_)) {
                if let Some(revision) = world
                    .try_resource::<AssetServer>()
                    .and_then(|server| server.revision(request.handle))
                {
                    self.failed_model_uploads
                        .remove(&(model_key(request.handle), revision));
                }
            }
        }
        self.failed_model_uploads.retain(|(key, revision), _| {
            live_model_keys.contains(key)
                && self
                    .asset_revisions
                    .get(key)
                    .is_none_or(|current| current == revision)
        });
        let stale_model_keys: Vec<_> = self
            .models
            .keys()
            .copied()
            .filter(|key| !live_model_keys.contains(key))
            .collect();
        if !stale_model_keys.is_empty() {
            for key in stale_model_keys {
                if let Some(parts) = self.models.remove(&key) {
                    self.model_cache_bytes.remove(&key);
                    self.model_gpu_releases_cumulative =
                        self.model_gpu_releases_cumulative.saturating_add(1);
                    if let Some(renderer) = self.renderer.as_mut() {
                        for part in parts {
                            renderer.release_mesh(part.mesh);
                            renderer.release_material(part.material);
                        }
                    }
                }
            }
        }

        // Keep dynamic geometry GPU-resident only while some extracted placement references it.
        // Each cache entry owns an Arc clone, so its address cannot be reused for different content.
        let live_dynamic_mesh_keys: HashSet<usize> = extracted
            .dynamic_meshes
            .iter()
            .filter(|placement| {
                !placement.mesh.vertices.is_empty()
                    && !placement.mesh.indices.is_empty()
                    && placement
                        .mesh
                        .indices
                        .iter()
                        .all(|&index| (index as usize) < placement.mesh.vertices.len())
            })
            .map(|placement| Arc::as_ptr(&placement.mesh) as usize)
            .collect();
        let stale_dynamic_mesh_keys: Vec<usize> = self
            .dynamic_meshes
            .keys()
            .copied()
            .filter(|key| !live_dynamic_mesh_keys.contains(key))
            .collect();
        if let Some(renderer) = self.renderer.as_mut() {
            for key in stale_dynamic_mesh_keys {
                if let Some(cached) = self.dynamic_meshes.remove(&key) {
                    renderer.release_mesh(cached.mesh);
                }
            }
        } else {
            self.dynamic_meshes.clear();
        }

        record_cpu_phase(
            &mut self.cpu_phases,
            "asset cache reconciliation",
            cache_started,
        );

        if width == 0 || height == 0 {
            // A minimized target cannot render visible placements, but GPU resource preparation
            // must continue: scene readiness must not deadlock waiting for a window resize.
            let mut background_budget = ModelUploadFrameBudget {
                uploads: 0,
                uploaded_bytes: 0,
                limits: world
                    .try_resource::<GpuUploadBudget>()
                    .copied()
                    .unwrap_or_default()
                    .normalized(),
            };
            self.process_generic_gpu_warmups(world, target, &mut background_budget, profile_cpu);
            self.generic_gpu_uploads_this_frame = background_budget.uploads;
            self.generic_gpu_upload_bytes_this_frame = background_budget.uploaded_bytes;
            // Release any size-specific HDR view only after its tone-mapper bindings are dropped.
            if extracted.meshes.is_empty()
                && extracted.dynamic_meshes.is_empty()
                && extracted.models.is_empty()
                && extracted.gpu_meshes.is_empty()
            {
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.maintain_instance_buffers(target.device, 0);
                }
            }
            self.release_hdr_target();
            self.transient_targets
                .trim_unused_to_budget(64 * 1024 * 1024);
            return;
        }

        let Some(camera) = extracted.camera else {
            // The renderer may be waiting for the very region that creates its first camera. Keep
            // bounded background GPU uploads progressing even while no scene camera is active.
            let mut background_budget = ModelUploadFrameBudget {
                uploads: 0,
                uploaded_bytes: 0,
                limits: world
                    .try_resource::<GpuUploadBudget>()
                    .copied()
                    .unwrap_or_default()
                    .normalized(),
            };
            self.process_generic_gpu_warmups(world, target, &mut background_budget, profile_cpu);
            self.generic_gpu_uploads_this_frame = background_budget.uploads;
            self.generic_gpu_upload_bytes_this_frame = background_budget.uploaded_bytes;
            if let Some(renderer) = self.renderer.as_mut() {
                renderer.maintain_instance_buffers(target.device, 0);
            }
            self.release_hdr_target();
            self.transient_targets
                .trim_unused_to_budget(64 * 1024 * 1024);
            return;
        };

        let valid_dynamic_mesh_count = extracted
            .dynamic_meshes
            .iter()
            .filter(|placement| {
                !placement.mesh.vertices.is_empty()
                    && !placement.mesh.indices.is_empty()
                    && placement
                        .mesh
                        .indices
                        .iter()
                        .all(|&index| (index as usize) < placement.mesh.vertices.len())
            })
            .count();
        if valid_dynamic_mesh_count > 0 && self.renderer.is_none() {
            self.renderer = Some(MeshRenderer::new(target.device, target.queue, HDR_FORMAT));
        }
        let mut dynamic_draws = Vec::with_capacity(valid_dynamic_mesh_count);
        for placement in &extracted.dynamic_meshes {
            if placement.mesh.vertices.is_empty()
                || placement.mesh.indices.is_empty()
                || placement
                    .mesh
                    .indices
                    .iter()
                    .any(|&index| (index as usize) >= placement.mesh.vertices.len())
            {
                continue;
            }
            let key = Arc::as_ptr(&placement.mesh) as usize;
            let mesh_id = if let Some(cached) = self.dynamic_meshes.get(&key) {
                cached.mesh
            } else {
                let Some(renderer) = self.renderer.as_mut() else {
                    continue;
                };
                let mesh_id = renderer.upload_mesh(
                    target.device,
                    &placement.mesh.vertices,
                    &placement.mesh.indices,
                );
                self.dynamic_meshes.insert(
                    key,
                    CachedDynamicMesh {
                        _source: Arc::clone(&placement.mesh),
                        mesh: mesh_id,
                    },
                );
                mesh_id
            };
            dynamic_draws.push(Draw {
                mesh: mesh_id,
                material: MeshRenderer::WHITE,
                model: placement.transform,
                tint: placement.color,
            });
        }

        let scene_conversion_started = profile_cpu.then(Instant::now);
        let mut draws: Vec<Draw> = extracted
            .meshes
            .iter()
            .map(|mesh| Draw {
                mesh: MeshRenderer::CUBE,
                material: MeshRenderer::WHITE,
                model: mesh.transform,
                tint: mesh.color,
            })
            .collect();
        draws.extend(dynamic_draws);
        draws.extend(extracted.gpu_meshes.iter().map(|placement| Draw {
            mesh: placement.mesh,
            material: placement.material,
            model: placement.transform,
            tint: placement.color,
        }));
        record_cpu_phase(
            &mut self.cpu_phases,
            "built-in scene conversion",
            scene_conversion_started,
        );

        let asset_resolution_started = profile_cpu.then(Instant::now);
        let mut upload_frame_budget = ModelUploadFrameBudget {
            uploads: 0,
            uploaded_bytes: 0,
            limits: world
                .try_resource::<GpuUploadBudget>()
                .copied()
                .unwrap_or_default()
                .normalized(),
        };
        world.resource_scope::<AssetServer, _>(|world, server| {
            let cpu_ready_warmup_exists = warmup_snapshots.iter().any(|request| {
                !matches!(&request.status, ModelWarmupStatus::Failed(_))
                    && server.get(request.handle).is_some()
            });
            if self.renderer.is_none() && (!extracted.models.is_empty() || cpu_ready_warmup_exists)
            {
                self.renderer = Some(MeshRenderer::new(target.device, target.queue, HDR_FORMAT));
            }

            // Existing visible scene placements have priority over background warmups. Both paths
            // share the same bounded per-frame upload budget and exact generation/revision cache.
            for placement in &extracted.models {
                let handle = placement.model;
                let key = model_key(handle);
                let Some(revision) = server.revision(handle) else {
                    continue;
                };
                let revision_key = (key, revision);
                if !self.models.contains_key(&key)
                    && !self.failed_model_uploads.contains_key(&revision_key)
                {
                    let _ = self.queue_model_upload(
                        world,
                        server,
                        target,
                        handle,
                        warmup_snapshots
                            .iter()
                            .find(|request| model_key(request.handle) == key)
                            .map(|request| request.path.clone()),
                        &mut upload_frame_budget,
                    );
                }
                if self.model_upload_pending(revision_key)
                    || self.failed_model_uploads.contains_key(&revision_key)
                {
                    continue;
                }
                let Some(parts) = self.models.get(&key) else {
                    continue;
                };
                draws.extend(parts.iter().map(|part| Draw {
                    mesh: part.mesh,
                    material: part.material,
                    model: placement.transform * part.transform,
                    tint: [1.0; 3],
                }));
            }

            // Standalone meshes/materials use the same count/byte admission budget as model
            // uploads. Visible placements above remain the highest-priority work in this pass.
            self.process_generic_gpu_warmups(world, target, &mut upload_frame_budget, profile_cpu);

            for request in &warmup_snapshots {
                if matches!(&request.status, ModelWarmupStatus::Failed(_)) {
                    continue;
                }
                let load_state = server.state::<ribbon_plugin_gltf::ModelAsset>(&request.path);
                match load_state {
                    Some(LoadState::Failed(error)) => {
                        let message =
                            format!("CPU asset `{}` failed to load: {error}", request.path);
                        Self::set_warmup_status(
                            world,
                            request.handle,
                            ModelWarmupStatus::Failed(message),
                        );
                        continue;
                    }
                    Some(LoadState::Unloaded) | None => {
                        let message = format!(
                            "CPU asset `{}` has no active asynchronous load request",
                            request.path
                        );
                        Self::set_warmup_status(
                            world,
                            request.handle,
                            ModelWarmupStatus::Failed(message),
                        );
                        continue;
                    }
                    Some(LoadState::Loading) => {
                        Self::set_warmup_status(
                            world,
                            request.handle,
                            ModelWarmupStatus::WaitingForCpu,
                        );
                        continue;
                    }
                    Some(LoadState::Loaded) => {}
                }
                if server.get(request.handle).is_none() {
                    Self::set_warmup_status(
                        world,
                        request.handle,
                        ModelWarmupStatus::WaitingForCpu,
                    );
                    continue;
                }
                let Some(revision) = server.revision(request.handle) else {
                    Self::set_warmup_status(
                        world,
                        request.handle,
                        ModelWarmupStatus::WaitingForCpu,
                    );
                    continue;
                };
                let key = model_key(request.handle);
                let revision_key = (key, revision);
                if let Some(error) = self.failed_model_uploads.get(&revision_key) {
                    Self::set_warmup_status(
                        world,
                        request.handle,
                        ModelWarmupStatus::Failed(format!(
                            "model `{}` failed GPU warmup: {error}",
                            request.path
                        )),
                    );
                    continue;
                }
                if self.models.contains_key(&key) {
                    if self.model_upload_pending(revision_key) {
                        Self::set_warmup_status(
                            world,
                            request.handle,
                            ModelWarmupStatus::WaitingForGpu,
                        );
                    } else {
                        Self::set_warmup_status(world, request.handle, ModelWarmupStatus::Ready);
                    }
                    continue;
                }
                Self::set_warmup_status(world, request.handle, ModelWarmupStatus::WaitingForGpu);
                let _ = self.queue_model_upload(
                    world,
                    server,
                    target,
                    request.handle,
                    Some(request.path.clone()),
                    &mut upload_frame_budget,
                );
            }
        });
        self.model_gpu_uploads_this_frame = upload_frame_budget
            .uploads
            .saturating_sub(self.generic_gpu_uploads_this_frame);
        self.model_gpu_upload_bytes_this_frame = upload_frame_budget
            .uploaded_bytes
            .saturating_sub(self.generic_gpu_upload_bytes_this_frame);

        record_cpu_phase(
            &mut self.cpu_phases,
            "model resolution and draw assembly",
            asset_resolution_started,
        );

        if draws.is_empty() {
            if let Some(renderer) = self.renderer.as_mut() {
                renderer.maintain_instance_buffers(target.device, 0);
            }
            self.release_hdr_target();
            self.transient_targets
                .trim_unused_to_budget(64 * 1024 * 1024);
            return;
        }
        if self.renderer.is_none() {
            self.models.clear();
            self.model_cache_bytes.clear();
            self.model_cache_pressure_level = 0;
            self.renderer = Some(MeshRenderer::new(target.device, target.queue, HDR_FORMAT));
        }
        if self.hdr_size != Some(target.size) {
            self.release_hdr_target();
            self.hdr = Some(self.transient_targets.acquire(
                target.device,
                TransientTextureDescriptor::color_target(target.size.0, target.size.1, HDR_FORMAT),
            ));
            self.hdr_size = Some(target.size);
            // Active targets are protected; old compatible targets are reused until this pool
            // exceeds its cache budget.
            self.transient_targets
                .trim_unused_to_budget(64 * 1024 * 1024);
        }
        let Some(hdr_handle) = self.hdr else {
            return;
        };
        let Some(hdr_view) = self.transient_targets.view(hdr_handle) else {
            return;
        };
        if self
            .tonemapper
            .as_ref()
            .is_none_or(|(format, _)| *format != target.format)
        {
            self.tonemapper = Some((
                target.format,
                ToneMapper::new(target.device, hdr_view, target.format, target.size),
            ));
        }

        let clear_color = world
            .try_resource::<ClearColor>()
            .copied()
            .unwrap_or_default()
            .0;
        clear_view(target.device, target.queue, hdr_view, clear_color);
        let view = Mat4::from_rotation_translation(
            camera.transform.rotation,
            camera.transform.translation,
        )
        .inverse();
        let view_proj = camera.camera.projection(width as f32 / height as f32) * view;
        if let Some(renderer) = self.renderer.as_mut() {
            let scene = Scene {
                view_proj,
                camera_position: camera.transform.translation,
                sun: &extracted.sun,
                spot_lights: &extracted.spot_lights,
                point_lights: &extracted.point_lights,
                fog: extracted.fog,
                draws: &draws,
            };
            if profile_cpu {
                self.cpu_phases.extend(renderer.render_profiled(
                    target.device,
                    target.queue,
                    hdr_view,
                    target.size,
                    &scene,
                    frame,
                ));
            } else {
                renderer.render(target.device, target.queue, hdr_view, target.size, &scene);
            }
        }
        if let Some((_, tonemapper)) = self.tonemapper.as_mut() {
            if profile_cpu {
                self.cpu_phases.extend(tonemapper.render_profiled(
                    target.device,
                    target.queue,
                    target.view,
                    frame,
                ));
            } else {
                tonemapper.render(target.device, target.queue, target.view);
            }
        }
    }
}

fn record_cpu_phase(
    phases: &mut Vec<RenderPassTiming>,
    name: &'static str,
    started: Option<Instant>,
) {
    if let Some(started) = started {
        phases.push(RenderPassTiming {
            name: name.to_owned(),
            cpu_time_ns: started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64,
        });
    }
}

/// Registers the mesh pass with the window presenter and installs a default [`SunLight`].
mod plugin;
pub use plugin::Render3dPlugin;

#[cfg(test)]
mod model_warmup_integration_tests {
    use super::{FramePass, FrameTarget, GpuContext, MeshPass, ModelWarmup, ModelWarmupStatus};
    use crate::{Camera3D, ExtractedCamera3D, ExtractedModel3D, ExtractedScene3D};
    use ribbon_app::World;
    use ribbon_assets::{AssetServer, FsSource};
    use ribbon_plugin_gltf::{GltfLoader, ModelAsset};
    use ribbon_scene::GlobalTransform3D;
    use std::path::Path;

    fn render(pass: &mut MeshPass, world: &mut World, context: &GpuContext) {
        let target = FrameTarget {
            device: context.device(),
            queue: context.queue(),
            view: context.target().view(),
            format: wgpu::TextureFormat::Rgba8Unorm,
            size: (8, 8),
        };
        pass.render(world, &target);
    }

    #[test]
    fn model_liveness_includes_visible_placements_and_pending_warmups() {
        let mut server = AssetServer::default();
        server.register_loader(GltfLoader);
        server.add_source(
            0,
            FsSource::new(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/models")),
        );
        let visible = server.load::<ModelAsset>("checker_cube.glb").unwrap();
        let warmup_only = server.load::<ModelAsset>("./checker_cube.glb").unwrap();
        let failed = server
            .load::<ModelAsset>("../models/checker_cube.glb")
            .unwrap();
        let extracted = ExtractedScene3D {
            models: vec![ExtractedModel3D {
                transform: glam::Mat4::IDENTITY,
                model: visible,
            }],
            ..ExtractedScene3D::default()
        };
        let warmups = vec![
            super::preload::ModelWarmupSnapshot {
                handle: warmup_only,
                path: "./checker_cube.glb".to_owned(),
                status: ModelWarmupStatus::WaitingForGpu,
                priority: 0,
                sequence: 0,
            },
            super::preload::ModelWarmupSnapshot {
                handle: failed,
                path: "../models/checker_cube.glb".to_owned(),
                status: ModelWarmupStatus::Failed("test failure".to_owned()),
                priority: 0,
                sequence: 1,
            },
        ];

        let live = super::collect_live_model_keys(&extracted, &warmups);
        assert!(live.contains(&super::model_key(visible)));
        assert!(live.contains(&super::model_key(warmup_only)));
        assert!(
            !live.contains(&super::model_key(failed)),
            "terminally failed warmups must not pin GPU cache entries"
        );
    }

    #[test]
    fn early_return_frame_resets_per_frame_model_upload_diagnostic() {
        let context = match pollster::block_on(GpuContext::new_offscreen(8, 8)) {
            Ok(context) => context,
            Err(error) => {
                eprintln!("skipping upload-diagnostic reset test: {}", error.0);
                return;
            }
        };
        let mut world = World::new();
        // No camera makes MeshPass take its deliberate clear-only path. Preserve a stale count to
        // prove the current render call resets it before any early return.
        world.insert_resource(crate::ExtractedScene3D::default());
        world.insert_resource(ModelWarmup::default());
        let mut pass = MeshPass {
            model_gpu_uploads_this_frame: 2,
            ..MeshPass::default()
        };
        render(&mut pass, &mut world, &context);
        assert_eq!(pass.model_gpu_uploads_this_frame, 0);
    }

    #[test]
    fn empty_scene_mesh_pass_preserves_the_window_clear_color() {
        let context = match pollster::block_on(GpuContext::new_offscreen(8, 8)) {
            Ok(context) => context,
            Err(error) => {
                eprintln!("skipping empty-scene clear preservation test: {}", error.0);
                return;
            }
        };
        super::clear_view(
            context.device(),
            context.queue(),
            context.target().view(),
            [1.0, 0.0, 1.0, 1.0],
        );
        let before = context.readback().expect("read clear frame").pixels;

        let mut world = World::new();
        world.insert_resource(ExtractedScene3D::default());
        world.insert_resource(ModelWarmup::default());
        world.insert_resource(ribbon_render::profiling::RenderProfileRecorder::new(
            "unused-test-profile.csv",
            4,
            0,
        ));
        world.insert_resource(ribbon_scene::TransformPropagationDiagnostics {
            last_3d_duration_ns: 1_234,
            sample_pending: true,
            ..ribbon_scene::TransformPropagationDiagnostics::default()
        });
        let mut pass = MeshPass::default();
        render(&mut pass, &mut world, &context);
        let after = context.readback().expect("read empty-scene frame").pixels;
        assert_eq!(
            after, before,
            "an empty scene must not replace the clear frame"
        );
        assert!(
            world
                .resource::<ribbon_render::profiling::RenderDiagnostics>()
                .cpu_phases
                .iter()
                .any(|phase| {
                    phase.name == "3D transform propagation (one-shot)"
                        && phase.cpu_time_ns == 1_234
                }),
            "a one-shot scene timing sample should appear in opt-in phase diagnostics"
        );
    }

    #[test]
    fn model_warmup_is_gpu_ready_without_placement_and_cache_survives_handoff() {
        let context = match pollster::block_on(GpuContext::new_offscreen(8, 8)) {
            Ok(context) => context,
            Err(error) => {
                eprintln!("skipping model warmup integration test: {}", error.0);
                return;
            }
        };

        let mut server = AssetServer::default();
        server.register_loader(GltfLoader);
        server.add_source(
            0,
            FsSource::new(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/models")),
        );
        // Path aliases keep distinct AssetServer keys while resolving to the same tiny packaged
        // glTF. Three unique keys exercise the real two-upload-per-frame budget without using
        // large game assets in a unit test.
        let handles = [
            (
                server
                    .load::<ModelAsset>("checker_cube.glb")
                    .expect("packaged model CPU load succeeds"),
                "checker_cube.glb",
            ),
            (
                server
                    .load::<ModelAsset>("./checker_cube.glb")
                    .expect("path alias to packaged model loads"),
                "./checker_cube.glb",
            ),
            (
                server
                    .load::<ModelAsset>("../models/checker_cube.glb")
                    .expect("parent-path alias to packaged model loads"),
                "../models/checker_cube.glb",
            ),
        ];

        let mut world = World::new();
        world.insert_resource(server);
        // The model resource-creation subphase is available only during opt-in profile capture.
        world.insert_resource(ribbon_render::profiling::RenderProfileRecorder::new(
            "unused-test-profile.csv",
            4,
            0,
        ));
        let mut warmup = ModelWarmup::default();
        for (handle, path) in handles {
            warmup.request(handle, path);
        }
        world.insert_resource(warmup);
        let extracted = ExtractedScene3D {
            camera: Some(ExtractedCamera3D {
                transform: GlobalTransform3D::default(),
                camera: Camera3D::default(),
            }),
            ..ExtractedScene3D::default()
        };
        world.insert_resource(extracted);

        let mut pass = MeshPass::default();
        let mut saw_resource_creation_phase = false;
        for _ in 0..120 {
            render(&mut pass, &mut world, &context);
            assert!(
                pass.model_gpu_uploads_this_frame <= 2,
                "background warmup must stay within the two-model-per-frame upload budget"
            );
            saw_resource_creation_phase |= world
                .try_resource::<ribbon_render::profiling::RenderDiagnostics>()
                .is_some_and(|diagnostics| {
                    diagnostics
                        .cpu_phases
                        .iter()
                        .any(|phase| phase.name == "model GPU resource creation/submission")
                });
            if handles.iter().all(|(handle, _)| {
                matches!(
                    world.resource::<ModelWarmup>().status(*handle),
                    Some(ModelWarmupStatus::Ready)
                )
            }) {
                break;
            }
        }
        for (handle, _) in handles {
            assert_eq!(
                world.resource::<ModelWarmup>().status(handle),
                Some(&ModelWarmupStatus::Ready),
                "warmup must not report success until upload validation completes"
            );
            assert!(pass.models.contains_key(&super::model_key(handle)));
        }
        assert!(world.resource::<ExtractedScene3D>().models.is_empty());
        assert!(
            saw_resource_creation_phase,
            "profile capture should expose CPU-side GPU resource creation/submission separately"
        );

        // Add a real placement and drop warmup-only retention in the same handoff window.
        let active_handle = handles[0].0;
        world
            .resource_mut::<ExtractedScene3D>()
            .models
            .push(ExtractedModel3D {
                transform: glam::Mat4::IDENTITY,
                model: active_handle,
            });
        for (handle, _) in handles {
            assert!(world.resource_mut::<ModelWarmup>().release(handle));
        }
        render(&mut pass, &mut world, &context);
        assert!(pass.models.contains_key(&super::model_key(active_handle)));
        assert_eq!(
            pass.models.len(),
            1,
            "only the active placement should retain its warmed cache entry"
        );

        world.resource_mut::<ExtractedScene3D>().models.clear();
        render(&mut pass, &mut world, &context);
        assert!(
            pass.models.is_empty(),
            "cache resources should be released after both warmup and scene use end"
        );
    }
}
