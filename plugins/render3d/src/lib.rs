//! Mesh rendering for the engine: ECS components, an instanced wgpu mesh
//! pipeline with a depth buffer, and a plugin that draws into the window.
//!
//! World space is right-handed and Y-up. Three-dimensional local transforms are
//! owned by `ribbon_scene`; this renderer consumes their resolved global transforms.

pub mod components;
pub mod extract;
pub mod mesh;
pub mod model;
pub mod offscreen;
pub mod post;
pub mod profiling;
pub mod renderer;

pub use components::{
    BuiltinMesh, Camera3D, FogSettings, MeshInstance, ModelInstance, PointLight, SpotLight,
    SunLight, Transform3D, VoxelMeshInstance3D,
};
pub use extract::{
    extract_scene_3d, ExtractedCamera3D, ExtractedMesh3D, ExtractedModel3D, ExtractedScene3D,
    ExtractedVoxelMesh3D, RenderExtraction3DPlugin,
};
pub use glam;
pub use model::{upload_model, GpuModelPart};
pub use offscreen::{
    OffscreenCamera3D, OffscreenDraw3D, OffscreenFrameTiming, OffscreenVoxelRenderer,
    VoxelBlockColor,
};
pub use renderer::{Draw, MaterialId, MeshId, MeshRenderer, PbrMaterialDesc, Scene, TextureData};
pub use ribbon_scene::GlobalTransform3D;

use crate::mesh::Vertex;
use crate::post::{ToneMapper, HDR_FORMAT};
use glam::Mat4;
use ribbon_app::{App, World};
use ribbon_assets::AssetServer;
use ribbon_plugin_voxel::VoxelChunkMesh;
use ribbon_plugins::Plugin;
use ribbon_render::gpu::{
    TransientTextureDescriptor, TransientTextureHandle, TransientTexturePool,
};
use ribbon_render::gpu_window::{clear_view, ClearColor, FramePass, FrameTarget, GpuWindow};
use ribbon_render::profiling::{
    RenderDiagnostics, RenderGpuTimingSample, RenderPassTiming, RenderProfileRecorder,
    RenderResourceStats,
};
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

type ModelAssetKey = (u64, u32);
type ModelAssetRevision = (ModelAssetKey, u64);

/// Bound synchronous model GPU-cache population work in one render frame.
/// A single large model may still be expensive; this limits first-use bursts across assets.
const MAX_MODEL_GPU_UPLOADS_PER_FRAME: usize = 2;

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
    /// Last CPU-content revision observed for each model handle.
    asset_revisions: HashMap<ModelAssetKey, u64>,
    /// Retain immutable CPU chunk meshes so pointer keys cannot be reused while cached.
    voxel_meshes: HashMap<usize, CachedVoxelMesh>,
    model_gpu_uploads_cumulative: u64,
    model_gpu_releases_cumulative: u64,
    voxel_mesh_gpu_uploads_cumulative: u64,
    voxel_mesh_gpu_releases_cumulative: u64,
    cpu_phases: Vec<RenderPassTiming>,
}

struct CachedVoxelMesh {
    // Strong ownership prevents an allocator-reused Arc pointer from aliasing this cache key.
    _source: Arc<VoxelChunkMesh>,
    gpu_mesh: crate::renderer::MeshId,
}

impl FramePass for MeshPass {
    fn name(&self) -> &'static str {
        "mesh pass"
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
            resources.scene_mesh_instances = extracted.meshes.len();
            resources.scene_model_placements = extracted.models.len();
            resources.scene_voxel_placements = extracted.voxels.len();
        }
        resources.gpu_model_cache_entries = self.models.len();
        resources.gpu_voxel_mesh_cache_entries = self.voxel_meshes.len();
        resources.model_gpu_uploads_cumulative = self.model_gpu_uploads_cumulative;
        resources.model_gpu_releases_cumulative = self.model_gpu_releases_cumulative;
        resources.voxel_mesh_gpu_uploads_cumulative = self.voxel_mesh_gpu_uploads_cumulative;
        resources.voxel_mesh_gpu_releases_cumulative = self.voxel_mesh_gpu_releases_cumulative;
        if let Some(stats) = renderer_stats {
            resources.live_meshes = stats.live_meshes;
            resources.live_materials = stats.live_materials;
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
                resources.cpu_asset_resident_bytes_estimate = stats.resident_bytes_estimate;
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
            diagnostics.record_gpu_timing(
                gpu_timing.supported || post_gpu_timing.supported,
                gpu_timing.latest_duration_ns,
                gpu_timing.latest_shadow_pass_ns,
                gpu_timing.latest_mesh_pass_ns,
                gpu_timing.latest_sequence,
                post_gpu_timing.latest_shadow_pass_ns,
                post_gpu_timing.latest_mesh_pass_ns,
                post_gpu_timing.latest_sequence,
                gpu_timing
                    .completed_samples
                    .saturating_add(post_gpu_timing.completed_samples),
                gpu_timing
                    .skipped_samples
                    .saturating_add(post_gpu_timing.skipped_samples),
            );
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

    fn render_extracted(
        &mut self,
        world: &mut World,
        extracted: &mut ExtractedScene3D,
        target: &FrameTarget<'_>,
        profile_cpu: bool,
        frame: u64,
    ) {
        let (width, height) = target.size;
        let cache_started = profile_cpu.then(Instant::now);

        // A successful hot reload retains its weak handle ID/generation but advances the CPU asset
        // revision. Rebuild this pass's GPU-side caches when an already-observed model changes.
        // Recreating the pass-owned renderer here releases old model/material/texture allocations
        // together and keeps all replacement uploads on this renderer-owning thread.
        // A weak ModelInstance handle must not keep GPU resources alive after its CPU asset has
        // been evicted. Only resident model handles count as live cache keys; the extraction may
        // still contain a stale handle for one frame while the scene/asset lifecycle catches up.
        let (live_model_keys, current_revisions): (
            HashSet<ModelAssetKey>,
            Vec<ModelAssetRevision>,
        ) = world
            .try_resource::<AssetServer>()
            .map(|server| {
                let mut live_keys = HashSet::new();
                let mut seen = HashSet::new();
                let mut revisions = Vec::new();
                for placement in &extracted.models {
                    let key = (placement.model.id(), placement.model.generation());
                    // A scene commonly repeats one model across many placements. Asset residency
                    // is unchanged during this pass, so query each generation-checked key once.
                    if !seen.insert(key) {
                        continue;
                    }
                    if server.get(placement.model).is_none() {
                        continue;
                    }
                    live_keys.insert(key);
                    if let Some(revision) = server.revision(placement.model) {
                        revisions.push((key, revision));
                    }
                }
                (live_keys, revisions)
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
            self.voxel_mesh_gpu_releases_cumulative = self
                .voxel_mesh_gpu_releases_cumulative
                .saturating_add(self.voxel_meshes.len() as u64);
            self.models.clear();
            self.asset_revisions.clear();
            self.voxel_meshes.clear();
            self.renderer = None;
            self.release_hdr_target();
        }
        for (key, revision) in current_revisions {
            self.asset_revisions.insert(key, revision);
        }
        // Revision history and GPU-uploaded model parts should live only while a resident model is
        // referenced by the extracted scene. Release every unique slot through generation-checked
        // handles; duplicate node placements may share the same primitive/material IDs.
        self.asset_revisions
            .retain(|key, _| live_model_keys.contains(key));
        let stale_model_keys: Vec<_> = self
            .models
            .keys()
            .copied()
            .filter(|key| !live_model_keys.contains(key))
            .collect();
        if !stale_model_keys.is_empty() {
            for key in stale_model_keys {
                if let Some(parts) = self.models.remove(&key) {
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

        // Drop GPU buffers for chunk meshes that no longer appear in the extracted scene.
        // This prevents repeated dynamic remeshes and despawns from growing the cache forever.
        let live_voxel_keys: HashSet<usize> = extracted
            .voxels
            .iter()
            .map(|placement| Arc::as_ptr(&placement.mesh) as usize)
            .collect();
        let stale_voxel_keys: Vec<usize> = self
            .voxel_meshes
            .keys()
            .copied()
            .filter(|key| !live_voxel_keys.contains(key))
            .collect();
        if !stale_voxel_keys.is_empty() {
            for key in stale_voxel_keys {
                if let Some(cached) = self.voxel_meshes.remove(&key) {
                    self.voxel_mesh_gpu_releases_cumulative =
                        self.voxel_mesh_gpu_releases_cumulative.saturating_add(1);
                    if let Some(renderer) = self.renderer.as_mut() {
                        renderer.release_mesh(cached.gpu_mesh);
                    }
                }
            }
        }
        record_cpu_phase(
            &mut self.cpu_phases,
            "asset cache reconciliation",
            cache_started,
        );

        if width == 0 || height == 0 {
            // A minimized/zero-sized target must not postpone asset-cache cleanup. Release any
            // size-specific HDR view only after its tone-mapper bindings are dropped. Keep instance
            // capacity while placements remain live; empty stress-cycle intervals may shrink it.
            if extracted.meshes.is_empty()
                && extracted.models.is_empty()
                && extracted.voxels.is_empty()
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
            if let Some(renderer) = self.renderer.as_mut() {
                renderer.maintain_instance_buffers(target.device, 0);
            }
            self.release_hdr_target();
            self.transient_targets
                .trim_unused_to_budget(64 * 1024 * 1024);
            return;
        };

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
        record_cpu_phase(
            &mut self.cpu_phases,
            "built-in scene conversion",
            scene_conversion_started,
        );

        let asset_resolution_started = profile_cpu.then(Instant::now);
        if !extracted.models.is_empty() {
            if let Some(server) = world.try_resource::<AssetServer>() {
                // Lazily create the device resources before importing model parts.
                if self.renderer.is_none() {
                    self.models.clear();
                    self.renderer =
                        Some(MeshRenderer::new(target.device, target.queue, HDR_FORMAT));
                }
                let Some(renderer) = self.renderer.as_mut() else {
                    return;
                };
                let mut model_uploads_this_frame = 0_usize;
                for placement in &extracted.models {
                    let key = (placement.model.id(), placement.model.generation());
                    let was_missing = !self.models.contains_key(&key);
                    let parts = match self.models.entry(key) {
                        Entry::Occupied(entry) => entry.into_mut(),
                        Entry::Vacant(entry) => {
                            // `upload_model` creates mesh/material buffers and may upload several
                            // textures. Do not synchronously populate every missing model cache
                            // entry in one frame: defer remaining placements until a later frame.
                            if model_uploads_this_frame >= MAX_MODEL_GPU_UPLOADS_PER_FRAME {
                                continue;
                            }
                            let Some(model) = server.get(placement.model) else {
                                continue;
                            };
                            let parts = upload_model(renderer, target.device, target.queue, model);
                            model_uploads_this_frame += 1;
                            entry.insert(parts)
                        }
                    };
                    if was_missing {
                        self.model_gpu_uploads_cumulative =
                            self.model_gpu_uploads_cumulative.saturating_add(1);
                    }
                    draws.extend(parts.iter().map(|part| Draw {
                        mesh: part.mesh,
                        material: part.material,
                        model: placement.transform * part.transform,
                        tint: [1.0; 3],
                    }));
                }
            }
        }

        if !extracted.voxels.is_empty() {
            if self.renderer.is_none() {
                self.models.clear();
                self.renderer = Some(MeshRenderer::new(target.device, target.queue, HDR_FORMAT));
            }
            let Some(renderer) = self.renderer.as_mut() else {
                return;
            };
            for placement in &extracted.voxels {
                let key = Arc::as_ptr(&placement.mesh) as usize;
                let was_missing = !self.voxel_meshes.contains_key(&key);
                let gpu_mesh = match self.voxel_meshes.entry(key) {
                    std::collections::hash_map::Entry::Occupied(entry) => entry.get().gpu_mesh,
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        let vertices: Vec<Vertex> = placement
                            .mesh
                            .vertices
                            .iter()
                            .map(|vertex| Vertex {
                                position: vertex.position,
                                normal: vertex.normal,
                                uv: vertex.uv,
                            })
                            .collect();
                        let gpu_mesh =
                            renderer.upload_mesh(target.device, &vertices, &placement.mesh.indices);
                        entry.insert(CachedVoxelMesh {
                            _source: Arc::clone(&placement.mesh),
                            gpu_mesh,
                        });
                        gpu_mesh
                    }
                };
                if was_missing {
                    self.voxel_mesh_gpu_uploads_cumulative =
                        self.voxel_mesh_gpu_uploads_cumulative.saturating_add(1);
                }
                draws.push(Draw {
                    mesh: gpu_mesh,
                    material: MeshRenderer::WHITE,
                    model: placement.transform,
                    tint: placement.color,
                });
            }
        }
        record_cpu_phase(
            &mut self.cpu_phases,
            "model/voxel resolution and draw assembly",
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
pub struct Render3dPlugin;

impl Plugin for Render3dPlugin {
    fn name(&self) -> &'static str {
        "Render3dPlugin"
    }

    fn dependencies(&self) -> &'static [&'static str] {
        &["GpuWindowPlugin", "ScenePlugin"]
    }

    fn build(&self, app: &mut App) {
        if app.world.try_resource::<SunLight>().is_none() {
            app.insert_resource(SunLight::default());
        }
        if app.world.try_resource::<FogSettings>().is_none() {
            app.insert_resource(FogSettings::default());
        }
        // ScenePlugin has already registered PostUpdate transform propagation because it is
        // an explicit dependency. Install extraction after it so all snapshots are world-space.
        extract::install_extraction(app);
        app.world
            .resource_mut::<GpuWindow>()
            .add_pass(Box::new(MeshPass::default()));
    }
}
