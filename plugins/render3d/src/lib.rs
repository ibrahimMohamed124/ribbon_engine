//! Mesh rendering for the engine: ECS components, an instanced wgpu mesh
//! pipeline with a depth buffer, and a plugin that draws into the window.
//!
//! World space is right-handed and Y-up. Three-dimensional local transforms are
//! owned by `engine_scene`; this renderer consumes their resolved global transforms.

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
pub use engine_scene::GlobalTransform3D;
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

use crate::mesh::Vertex;
use crate::post::{ToneMapper, HDR_FORMAT};
use engine_app::{App, World};
use engine_assets::AssetServer;
use engine_plugins::Plugin;
use engine_render::gpu::{
    TransientTextureDescriptor, TransientTextureHandle, TransientTexturePool,
};
use engine_render::gpu_window::{clear_view, ClearColor, FramePass, FrameTarget, GpuWindow};
use engine_render::profiling::{RenderDiagnostics, RenderResourceStats};
use glam::Mat4;
use plugin_voxel::VoxelChunkMesh;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[derive(Default)]
struct MeshPass {
    renderer: Option<MeshRenderer>,
    /// Active HDR target leased from the transient texture pool.
    hdr: Option<TransientTextureHandle>,
    hdr_size: Option<(u32, u32)>,
    transient_targets: TransientTexturePool,
    tonemapper: Option<(wgpu::TextureFormat, ToneMapper)>,
    /// Uploaded models keyed by asset handle id and generation.
    models: HashMap<(u64, u32), Vec<GpuModelPart>>,
    /// Last CPU-content revision observed for each model handle.
    asset_revisions: HashMap<(u64, u32), u64>,
    /// Retain immutable CPU chunk meshes so pointer keys cannot be reused while cached.
    voxel_meshes: HashMap<usize, CachedVoxelMesh>,
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
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.poll_gpu_timing(target.device, target.queue);
        }
        world.resource_scope::<ExtractedScene3D, _>(|world, extracted| {
            self.render_extracted(world, extracted, target);
        });
        self.publish_diagnostics(world, target.device);
    }
}

impl MeshPass {
    fn publish_diagnostics(&self, world: &mut World, device: &wgpu::Device) {
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
        if let Some(stats) = renderer_stats {
            resources.live_meshes = stats.live_meshes;
            resources.live_materials = stats.live_materials;
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
        }
        if world.try_resource::<RenderDiagnostics>().is_some() {
            let diagnostics = world.resource_mut::<RenderDiagnostics>();
            diagnostics.record_gpu_timing(
                gpu_timing.supported,
                gpu_timing.latest_duration_ns,
                gpu_timing.latest_mesh_pass_ns,
                gpu_timing.completed_samples,
                gpu_timing.skipped_samples,
                gpu_timing.latest_sequence,
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
    ) {
        let (width, height) = target.size;
        if width == 0 || height == 0 {
            return;
        }

        // A successful hot reload retains its weak handle ID/generation but advances the CPU asset
        // revision. Rebuild this pass's GPU-side caches when an already-observed model changes.
        // Recreating the pass-owned renderer here releases old model/material/texture allocations
        // together and keeps all replacement uploads on this renderer-owning thread.
        let live_model_keys: HashSet<(u64, u32)> = extracted
            .models
            .iter()
            .map(|placement| (placement.model.id(), placement.model.generation()))
            .collect();
        let current_revisions: Vec<_> = world
            .try_resource::<AssetServer>()
            .map(|server| {
                extracted
                    .models
                    .iter()
                    .filter_map(|placement| {
                        server.revision(placement.model).map(|revision| {
                            (
                                (placement.model.id(), placement.model.generation()),
                                revision,
                            )
                        })
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
            self.models.clear();
            self.asset_revisions.clear();
            self.voxel_meshes.clear();
            self.renderer = None;
            self.release_hdr_target();
        }
        for (key, revision) in current_revisions {
            self.asset_revisions.insert(key, revision);
        }
        // Revision history and GPU-uploaded model parts should live only while the model is
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
                    if let Some(renderer) = self.renderer.as_mut() {
                        renderer.release_mesh(cached.gpu_mesh);
                    }
                }
            }
        }

        let Some(camera) = extracted.camera else {
            self.release_hdr_target();
            self.transient_targets
                .trim_unused_to_budget(64 * 1024 * 1024);
            return;
        };

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
                for placement in &extracted.models {
                    let key = (placement.model.id(), placement.model.generation());
                    let parts = match self.models.entry(key) {
                        Entry::Occupied(entry) => entry.into_mut(),
                        Entry::Vacant(entry) => {
                            let Some(model) = server.get(placement.model) else {
                                continue;
                            };
                            entry.insert(upload_model(renderer, target.device, target.queue, model))
                        }
                    };
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
                draws.push(Draw {
                    mesh: gpu_mesh,
                    material: MeshRenderer::WHITE,
                    model: placement.transform,
                    tint: placement.color,
                });
            }
        }

        if draws.is_empty() {
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
            renderer.render(
                target.device,
                target.queue,
                hdr_view,
                target.size,
                &Scene {
                    view_proj,
                    camera_position: camera.transform.translation,
                    sun: &extracted.sun,
                    spot_lights: &extracted.spot_lights,
                    point_lights: &extracted.point_lights,
                    fog: extracted.fog,
                    draws: &draws,
                },
            );
        }
        if let Some((_, tonemapper)) = self.tonemapper.as_ref() {
            tonemapper.render(target.device, target.queue, target.view);
        }
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
