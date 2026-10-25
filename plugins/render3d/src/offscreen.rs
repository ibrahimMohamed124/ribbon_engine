//! Offscreen native 3D rendering for mutable voxel worlds.
//!
//! This is an engine rendering API, not a game renderer: callers provide a chunked
//! voxel resource, camera, palette, lighting and additional mesh instances. The engine
//! owns dirty-chunk remeshing consumption, GPU buffer upload/eviction, depth, shadows,
//! HDR tone mapping and readback to the backend-neutral framebuffer contract.

use crate::components::SunLight;
use crate::mesh::Vertex;
use crate::post::{ToneMapper, HDR_FORMAT};
use crate::profiling::GpuTimingSnapshot;
use crate::renderer::{Draw, MeshId, MeshRenderer, Scene};
use glam::{Mat4, Vec3};
use ribbon_plugin_voxel::{ChunkCoord, ChunkedVoxelWorld, VoxelChunkMesh};
use ribbon_render::gpu::{
    GpuContext, TransientTextureDescriptor, TransientTextureHandle, TransientTexturePool,
};
use ribbon_render::gpu_window::clear_view;
use ribbon_render::profiling::RenderResourceStats;
use ribbon_render::{graph::RenderError, Framebuffer};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

/// Camera parameters for a perspective 3D view.
#[derive(Clone, Copy, Debug)]
pub struct OffscreenCamera3D {
    pub position: Vec3,
    /// World-space direction the camera looks toward.
    pub forward: Vec3,
    /// Camera up vector, generally `Vec3::Y` for engine Y-up worlds.
    pub up: Vec3,
    pub fov_y: f32,
    pub near: f32,
    pub far: f32,
}

impl Default for OffscreenCamera3D {
    fn default() -> Self {
        Self {
            position: Vec3::new(0.0, 2.0, 5.0),
            forward: Vec3::NEG_Z,
            up: Vec3::Y,
            fov_y: 60.0_f32.to_radians(),
            near: 0.1,
            far: 500.0,
        }
    }
}

/// Linear RGB tint for one voxel block identifier in [`OffscreenVoxelRenderer::render`].
///
/// The palette is a deliberately small data contract: each chunk mesh uses the renderer's shared
/// white PBR material multiplied by this tint. It does not accept `MaterialTable` or execute the
/// CPU-only `SurfaceShader` callbacks, and it does not currently select distinct PBR texture maps
/// per voxel block. Applications needing textured per-block materials should use the mesh APIs
/// directly or treat such mapping as a separate future capability.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VoxelBlockColor {
    pub block: u8,
    pub color: [f32; 3],
}

/// Additional unit-cube instances for general scene objects, not voxel-grid cells.
#[derive(Clone, Copy, Debug)]
pub struct OffscreenDraw3D {
    pub model: Mat4,
    pub tint: [f32; 3],
}

struct UploadedChunkMesh {
    source: Arc<VoxelChunkMesh>,
    id: MeshId,
}

/// CPU timings for the most recent offscreen render/readback call.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OffscreenFrameTiming {
    /// Number of render calls measured through their readback attempt.
    pub frame_count: u64,
    /// Most recent total CPU duration, including synchronous framebuffer readback.
    pub last_frame_cpu_ns: u64,
    /// Running arithmetic mean of total CPU duration.
    pub average_frame_cpu_ns: u64,
    /// Peak total CPU duration observed.
    pub peak_frame_cpu_ns: u64,
}

/// GPU renderer that incrementally uploads meshes produced by [`ChunkedVoxelWorld`].
/// It releases replaced/empty chunk geometry and reuses mesh and transient-target slots across edits.
pub struct OffscreenVoxelRenderer {
    // Drop GPU-owned render resources before the context/device/instance.
    renderer: MeshRenderer,
    hdr: TransientTextureHandle,
    tonemapper: Option<ToneMapper>,
    transient_targets: TransientTexturePool,
    uploaded_chunks: BTreeMap<ChunkCoord, Vec<UploadedChunkMesh>>,
    cpu_timing: OffscreenFrameTiming,
    peak_tracked_gpu_bytes_estimate: u64,
    last_scene_mesh_instances: usize,
    voxel_mesh_gpu_uploads_cumulative: u64,
    voxel_mesh_gpu_releases_cumulative: u64,
    size: (u32, u32),
    context: GpuContext,
}

const DEFAULT_VOXEL_BLOCK_TINT: [f32; 3] = [0.58, 0.58, 0.58];

fn voxel_block_tint(block: u8, palette: &[VoxelBlockColor]) -> [f32; 3] {
    palette
        .iter()
        .find(|entry| entry.block == block)
        .map_or(DEFAULT_VOXEL_BLOCK_TINT, |entry| entry.color)
}

impl OffscreenVoxelRenderer {
    /// Create an offscreen renderer that returns pixels through the engine framebuffer
    /// contract. GPU initialization errors are reported so applications can select a fallback.
    pub fn new(width: u32, height: u32) -> Result<Self, RenderError> {
        let size = (width.max(1), height.max(1));
        let context = pollster::block_on(GpuContext::new_offscreen(size.0, size.1))?;
        let renderer = MeshRenderer::new(context.device(), context.queue(), HDR_FORMAT);
        let mut transient_targets = TransientTexturePool::default();
        let hdr = transient_targets.acquire(
            context.device(),
            TransientTextureDescriptor::color_target(size.0, size.1, HDR_FORMAT),
        );
        let tonemapper = {
            let view = transient_targets
                .view(hdr)
                .expect("a newly acquired HDR target must be leased");
            ToneMapper::new_offscreen_unorm(context.device(), view, size)
        };
        let mut offscreen = Self {
            renderer,
            hdr,
            tonemapper: Some(tonemapper),
            transient_targets,
            uploaded_chunks: BTreeMap::new(),
            cpu_timing: OffscreenFrameTiming::default(),
            peak_tracked_gpu_bytes_estimate: 0,
            last_scene_mesh_instances: 0,
            voxel_mesh_gpu_uploads_cumulative: 0,
            voxel_mesh_gpu_releases_cumulative: 0,
            size,
            context,
        };
        offscreen.update_peak_resource_estimate();
        Ok(offscreen)
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    pub fn uploaded_chunk_count(&self) -> usize {
        self.uploaded_chunks.len()
    }

    pub fn uploaded_mesh_count(&self) -> usize {
        self.uploaded_chunks.values().map(Vec::len).sum()
    }

    /// CPU timing summary for render calls, including blocking framebuffer readback.
    pub fn cpu_timing(&self) -> OffscreenFrameTiming {
        self.cpu_timing
    }

    /// Latest completed asynchronous GPU timing sample. Returns unsupported/empty values when
    /// the adapter does not expose timestamp queries; this method never waits for the GPU.
    pub fn gpu_timing_snapshot(&self) -> GpuTimingSnapshot {
        self.renderer.gpu_timing_snapshot()
    }

    /// Highest combined tracked GPU allocation estimate observed at initialization, resize,
    /// or the end of a render call. This is not a driver-reported VRAM measurement.
    pub fn peak_resource_bytes_estimate(&self) -> u64 {
        self.peak_tracked_gpu_bytes_estimate
    }

    /// Estimated tracked GPU memory for this offscreen renderer and its owned targets.
    /// These are allocation estimates, not driver-reported VRAM measurements.
    pub fn resource_stats(&self) -> RenderResourceStats {
        let renderer = self.renderer.resource_stats();
        let pool = self.transient_targets.stats();
        let uploaded_mesh_count = self.uploaded_mesh_count();
        RenderResourceStats {
            live_meshes: renderer.live_meshes,
            live_materials: renderer.live_materials,
            mesh_bytes_estimate: renderer.mesh_bytes_estimate,
            material_bytes_estimate: renderer.material_bytes_estimate,
            live_material_textures: renderer.live_material_textures,
            material_texture_bytes_estimate: renderer.material_texture_bytes_estimate,
            material_uniform_bytes_estimate: renderer.material_uniform_bytes_estimate,
            material_texture_cache_hits: renderer.material_texture_cache_hits,
            material_texture_cache_misses: renderer.material_texture_cache_misses,
            buffer_bytes_estimate: renderer.buffer_bytes_estimate,
            depth_target_bytes_estimate: renderer.depth_target_bytes_estimate,
            active_transient_textures: pool.active_textures,
            cached_transient_textures: pool.cached_textures,
            active_transient_bytes_estimate: pool.active_bytes_estimate,
            cached_transient_bytes_estimate: pool.cached_bytes_estimate,
            post_process_bytes_estimate: self
                .tonemapper
                .as_ref()
                .map(ToneMapper::estimated_bytes)
                .unwrap_or(0),
            context_target_bytes_estimate: self.context.target_bytes_estimate(),
            context_readback_bytes_estimate: self.context.readback_buffer_bytes_estimate(),
            backend_peak_bytes_estimate: renderer.peak_bytes_estimate,
            transient_texture_allocations: pool.allocations,
            transient_texture_reuses: pool.reuses,
            transient_pool_peak_bytes_estimate: pool.peak_allocated_bytes_estimate,
            transient_unknown_size_textures: pool.unknown_size_textures,
            scene_mesh_instances: self.last_scene_mesh_instances,
            scene_voxel_placements: uploaded_mesh_count,
            gpu_voxel_mesh_cache_entries: uploaded_mesh_count,
            voxel_mesh_gpu_uploads_cumulative: self.voxel_mesh_gpu_uploads_cumulative,
            voxel_mesh_gpu_releases_cumulative: self.voxel_mesh_gpu_releases_cumulative,
            renderer_instance_capacity: renderer.instance_capacity,
            renderer_instance_buffer_grows_cumulative: renderer.instance_buffer_grows_cumulative,
            renderer_instance_buffer_shrinks_cumulative: renderer
                .instance_buffer_shrinks_cumulative,
            ..RenderResourceStats::default()
        }
    }

    /// Resize offscreen and HDR targets. Zero-sized window events are ignored safely.
    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 || self.size == (width, height) {
            return;
        }
        // Drop the bind group before returning its source texture to the reusable pool.
        self.tonemapper = None;
        self.transient_targets.release(self.hdr);
        self.context.resize(width, height);
        self.hdr = self.transient_targets.acquire(
            self.context.device(),
            TransientTextureDescriptor::color_target(width, height, HDR_FORMAT),
        );
        self.tonemapper = Some({
            let view = self
                .transient_targets
                .view(self.hdr)
                .expect("a newly acquired HDR target must be leased");
            ToneMapper::new_offscreen_unorm(self.context.device(), view, (width, height))
        });
        self.transient_targets
            .trim_unused_to_budget(64 * 1024 * 1024);
        self.size = (width, height);
        self.update_peak_resource_estimate();
    }

    /// Apply pending world edits to GPU meshes and render the current scene. Passing `None`
    /// for `camera` clears the frame and returns a blank framebuffer, useful for menus.
    ///
    /// `block_palette` maps block IDs to linear-RGB tints; missing IDs use a neutral-gray tint.
    /// All voxel chunk meshes use the shared white PBR material multiplied by that tint. This
    /// method does not consume `SurfaceShader` or `MaterialTable` and cannot reproduce arbitrary
    /// CPU per-hit callbacks or assign independent texture maps to block IDs. Use
    /// `MeshRenderer::upload_pbr_material_with_maps` for supported PBR mesh materials.
    pub fn render(
        &mut self,
        world: &mut ChunkedVoxelWorld,
        camera: Option<OffscreenCamera3D>,
        block_palette: &[VoxelBlockColor],
        sun: SunLight,
        objects: &[OffscreenDraw3D],
        clear_color: [f64; 4],
    ) -> Result<Framebuffer, RenderError> {
        let frame_started = Instant::now();
        self.last_scene_mesh_instances = objects.len();
        self.renderer
            .poll_gpu_timing(self.context.device(), self.context.queue());
        // The offscreen API exposes the latest snapshot, not a completion stream. Consume the
        // completed records here so the bounded profiler queue does not report false drops.
        self.renderer.clear_gpu_timing_samples();
        self.sync_dirty_chunks(world);
        let hdr_view = self
            .transient_targets
            .view(self.hdr)
            .expect("the offscreen HDR target remains leased while rendering");
        clear_view(
            self.context.device(),
            self.context.queue(),
            hdr_view,
            clear_color,
        );

        if let Some(camera) = camera {
            let mut draws = Vec::new();
            for meshes in self.uploaded_chunks.values() {
                for uploaded in meshes {
                    let color = voxel_block_tint(uploaded.source.block, block_palette);
                    draws.push(Draw {
                        mesh: uploaded.id,
                        material: MeshRenderer::WHITE,
                        model: Mat4::from_translation(Vec3::from_array(
                            uploaded.source.origin_y_up,
                        )),
                        tint: color,
                    });
                }
            }
            draws.extend(objects.iter().map(|object| Draw {
                mesh: MeshRenderer::CUBE,
                material: MeshRenderer::WHITE,
                model: object.model,
                tint: object.tint,
            }));

            if !draws.is_empty() {
                let forward = if camera.forward.length_squared() > 1.0e-8 {
                    camera.forward.normalize()
                } else {
                    Vec3::NEG_Z
                };
                let up = if camera.up.length_squared() > 1.0e-8 {
                    camera.up.normalize()
                } else {
                    Vec3::Y
                };
                let aspect = self.size.0 as f32 / self.size.1 as f32;
                let view = Mat4::look_at_rh(camera.position, camera.position + forward, up);
                let projection = Mat4::perspective_rh(
                    camera.fov_y.clamp(0.1_f32.to_radians(), 3.0),
                    aspect.max(1.0e-4),
                    camera.near.max(0.01),
                    camera.far.max(camera.near + 0.1),
                );
                let scene = Scene {
                    view_proj: projection * view,
                    camera_position: camera.position,
                    sun: &sun,
                    spot_lights: &[],
                    point_lights: &[],
                    fog: crate::FogSettings::default(),
                    draws: &draws,
                };
                self.renderer.render(
                    self.context.device(),
                    self.context.queue(),
                    hdr_view,
                    self.size,
                    &scene,
                );
            }
        }

        if let Some(tonemapper) = self.tonemapper.as_mut() {
            tonemapper.poll_gpu_timing(self.context.device(), self.context.queue());
            // Offscreen callers cannot consume individual Bloom/tone-mapping completions, so
            // discard consumed records after updating the latest snapshot.
            tonemapper.clear_gpu_timing_samples();
            tonemapper.render(
                self.context.device(),
                self.context.queue(),
                self.context.target().view(),
            );
        }
        let result = self.context.readback();
        self.record_cpu_timing(frame_started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64);
        self.update_peak_resource_estimate();
        result
    }

    fn update_peak_resource_estimate(&mut self) {
        self.peak_tracked_gpu_bytes_estimate = self
            .peak_tracked_gpu_bytes_estimate
            .max(self.resource_stats().total_bytes_estimate());
    }

    fn record_cpu_timing(&mut self, elapsed_ns: u64) {
        self.cpu_timing.frame_count = self.cpu_timing.frame_count.saturating_add(1);
        self.cpu_timing.last_frame_cpu_ns = elapsed_ns;
        self.cpu_timing.peak_frame_cpu_ns = self.cpu_timing.peak_frame_cpu_ns.max(elapsed_ns);
        if self.cpu_timing.frame_count == 1 {
            self.cpu_timing.average_frame_cpu_ns = elapsed_ns;
        } else {
            let previous = u128::from(self.cpu_timing.average_frame_cpu_ns);
            let sample = u128::from(elapsed_ns);
            let previous_count = u128::from(self.cpu_timing.frame_count - 1);
            let count = u128::from(self.cpu_timing.frame_count);
            let average = (previous * previous_count + sample) / count;
            self.cpu_timing.average_frame_cpu_ns = average.min(u128::from(u64::MAX)) as u64;
        }
    }

    fn sync_dirty_chunks(&mut self, world: &mut ChunkedVoxelWorld) {
        for update in world.rebuild_dirty_chunks() {
            if let Some(previous) = self.uploaded_chunks.remove(&update.coord) {
                for uploaded in previous {
                    if self.renderer.release_mesh(uploaded.id) {
                        self.voxel_mesh_gpu_releases_cumulative =
                            self.voxel_mesh_gpu_releases_cumulative.saturating_add(1);
                    }
                }
            }
            if update.meshes.is_empty() {
                continue;
            }
            let mut uploaded_chunk = Vec::with_capacity(update.meshes.len());
            for source in update.meshes {
                if source.vertices.is_empty() || source.indices.is_empty() {
                    continue;
                }
                let vertices: Vec<Vertex> = source
                    .vertices
                    .iter()
                    .map(|vertex| Vertex {
                        position: vertex.position,
                        normal: vertex.normal,
                        uv: vertex.uv,
                    })
                    .collect();
                let id =
                    self.renderer
                        .upload_mesh(self.context.device(), &vertices, &source.indices);
                self.voxel_mesh_gpu_uploads_cumulative =
                    self.voxel_mesh_gpu_uploads_cumulative.saturating_add(1);
                uploaded_chunk.push(UploadedChunkMesh { source, id });
            }
            if !uploaded_chunk.is_empty() {
                self.uploaded_chunks.insert(update.coord, uploaded_chunk);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{OffscreenCamera3D, OffscreenDraw3D, OffscreenVoxelRenderer, VoxelBlockColor};
    use crate::glam::Vec3;
    use crate::SunLight;
    use ribbon_plugin_voxel::{ChunkedVoxelWorld, VoxelWorld};

    const AIR: u8 = 0;
    const STONE: u8 = 1;
    const WIDTH: usize = 8;
    const HEIGHT: usize = 4;
    const DEPTH: usize = 2;

    #[test]
    fn voxel_block_palette_is_linear_rgb_with_a_neutral_missing_id_fallback() {
        let palette = [
            VoxelBlockColor {
                block: 1,
                color: [0.12, 0.34, 0.56],
            },
            VoxelBlockColor {
                block: 7,
                color: [0.91, 0.42, 0.08],
            },
        ];

        assert_eq!(super::voxel_block_tint(1, &palette), [0.12, 0.34, 0.56]);
        assert_eq!(super::voxel_block_tint(7, &palette), [0.91, 0.42, 0.08]);
        assert_eq!(super::voxel_block_tint(99, &palette), [0.58, 0.58, 0.58]);
        assert_eq!(super::voxel_block_tint(99, &[]), [0.58, 0.58, 0.58]);
    }

    fn world_with_boundary_block() -> ChunkedVoxelWorld {
        let blocks = vec![AIR; WIDTH * HEIGHT * DEPTH];
        let mut world = ChunkedVoxelWorld::new(
            VoxelWorld::new(WIDTH, HEIGHT, DEPTH, blocks),
            [4, 4, 2],
            AIR,
        );
        assert_eq!(world.set_block(3, 1, 0, STONE), Some(AIR));
        world
    }

    fn render(renderer: &mut OffscreenVoxelRenderer, world: &mut ChunkedVoxelWorld) {
        let frame = renderer
            .render(
                world,
                Some(OffscreenCamera3D {
                    position: Vec3::new(3.5, 0.5, 2.5),
                    forward: Vec3::NEG_Z,
                    up: Vec3::Y,
                    fov_y: 60.0_f32.to_radians(),
                    near: 0.1,
                    far: 100.0,
                }),
                &[VoxelBlockColor {
                    block: STONE,
                    color: [0.5, 0.5, 0.5],
                }],
                SunLight::default(),
                &[] as &[OffscreenDraw3D],
                [0.0, 0.0, 0.0, 1.0],
            )
            .expect("offscreen mesh render/readback should succeed");
        assert_eq!(frame.width, renderer.size().0 as usize);
        assert_eq!(frame.height, renderer.size().1 as usize);
        if renderer.uploaded_mesh_count() > 0 {
            assert!(
                frame.pixels.iter().any(|pixel| *pixel & 0x00ff_ffff != 0),
                "a non-empty chunk mesh should contribute visible pixels"
            );
        } else {
            assert!(
                frame.pixels.iter().all(|pixel| *pixel == frame.pixels[0]),
                "an empty world should render a uniform clear frame, not stale mesh geometry"
            );
        }
    }

    #[test]
    fn dynamic_chunk_meshes_upload_replace_and_evict_without_stale_gpu_entries() {
        let mut renderer = match OffscreenVoxelRenderer::new(32, 32) {
            Ok(renderer) => renderer,
            Err(error) => {
                if std::env::var_os("RIBBON_REQUIRE_GPU_TESTS").is_some() {
                    panic!(
                        "required offscreen mesh lifecycle GPU test could not initialize: {error}"
                    );
                }
                eprintln!(
                    "SKIP: offscreen mesh lifecycle test needs a usable GPU adapter: {error}"
                );
                return;
            }
        };
        let mut world = world_with_boundary_block();

        // Initial snapshot uploads the one non-empty chunk; the empty neighbor creates no mesh.
        render(&mut renderer, &mut world);
        assert_eq!(
            renderer.uploaded_chunk_count(),
            1,
            "uploaded chunks: {:?}",
            renderer.uploaded_chunks.keys().collect::<Vec<_>>()
        );
        assert_eq!(renderer.uploaded_mesh_count(), 1);
        let initial = renderer.resource_stats();
        assert_eq!(initial.gpu_voxel_mesh_cache_entries, 1);
        // MeshRenderer keeps its built-in cube alive; the second live mesh is the voxel chunk.
        assert_eq!(initial.live_meshes, 2);
        assert_eq!(initial.voxel_mesh_gpu_uploads_cumulative, 1);
        assert_eq!(initial.voxel_mesh_gpu_releases_cumulative, 0);

        // A no-op edit must not dirty chunks or create GPU uploads/releases.
        let revision = world.revision();
        assert_eq!(world.set_block(3, 1, 0, STONE), Some(STONE));
        assert_eq!(world.revision(), revision);
        assert_eq!(world.dirty_chunk_count(), 0);
        render(&mut renderer, &mut world);
        let no_op = renderer.resource_stats();
        assert_eq!(no_op.voxel_mesh_gpu_uploads_cumulative, 1);
        assert_eq!(no_op.voxel_mesh_gpu_releases_cumulative, 0);
        assert_eq!(renderer.uploaded_chunk_count(), 1);

        // Adding the adjacent voxel at the chunk face must invalidate both neighbors before upload.
        let revision_before_boundary_edit = world.revision();
        assert_eq!(world.set_block(4, 1, 0, STONE), Some(AIR));
        assert_eq!(world.revision(), revision_before_boundary_edit + 1);
        assert_eq!(world.dirty_chunk_count(), 2);
        render(&mut renderer, &mut world);
        assert_eq!(world.dirty_chunk_count(), 0);
        let boundary = renderer.resource_stats();
        assert_eq!(renderer.uploaded_chunk_count(), 2);
        assert_eq!(renderer.uploaded_mesh_count(), 2);
        assert_eq!(boundary.gpu_voxel_mesh_cache_entries, 2);
        assert_eq!(boundary.live_meshes, 3);
        assert_eq!(boundary.voxel_mesh_gpu_uploads_cumulative, 3);
        assert_eq!(boundary.voxel_mesh_gpu_releases_cumulative, 1);

        // Removing the left block evicts that empty chunk and replaces the solid neighbor.
        assert_eq!(world.set_block(3, 1, 0, AIR), Some(STONE));
        assert_eq!(world.dirty_chunk_count(), 2);
        render(&mut renderer, &mut world);
        assert_eq!(world.dirty_chunk_count(), 0);
        let left_empty = renderer.resource_stats();
        assert_eq!(renderer.uploaded_chunk_count(), 1);
        assert_eq!(renderer.uploaded_mesh_count(), 1);
        assert_eq!(left_empty.gpu_voxel_mesh_cache_entries, 1);
        assert_eq!(left_empty.live_meshes, 2);
        assert_eq!(left_empty.voxel_mesh_gpu_uploads_cumulative, 4);
        assert_eq!(left_empty.voxel_mesh_gpu_releases_cumulative, 3);

        // Removing the last voxel drops the final GPU mesh. Repeated empty updates stay empty.
        assert_eq!(world.set_block(4, 1, 0, AIR), Some(STONE));
        assert_eq!(world.dirty_chunk_count(), 2);
        render(&mut renderer, &mut world);
        assert_eq!(world.dirty_chunk_count(), 0);
        let all_empty = renderer.resource_stats();
        assert_eq!(renderer.uploaded_chunk_count(), 0);
        assert_eq!(renderer.uploaded_mesh_count(), 0);
        assert_eq!(all_empty.gpu_voxel_mesh_cache_entries, 0);
        assert_eq!(
            all_empty.live_meshes, 1,
            "only the built-in cube may remain live"
        );
        assert_eq!(all_empty.voxel_mesh_gpu_uploads_cumulative, 4);
        assert_eq!(all_empty.voxel_mesh_gpu_releases_cumulative, 4);
        assert_eq!(world.dirty_chunk_count(), 0);

        render(&mut renderer, &mut world);
        let repeated = renderer.resource_stats();
        assert_eq!(renderer.uploaded_chunk_count(), 0);
        assert_eq!(repeated.gpu_voxel_mesh_cache_entries, 0);
        assert_eq!(
            repeated.live_meshes, 1,
            "empty frames must not leak dynamic meshes"
        );
        assert_eq!(repeated.voxel_mesh_gpu_uploads_cumulative, 4);
        assert_eq!(repeated.voxel_mesh_gpu_releases_cumulative, 4);

        // Re-add and remove a boundary voxel to force freed GPU mesh slots to be reused.
        // Cumulative counters remain monotonic, while the live-resource count returns to baseline.
        assert_eq!(world.set_block(3, 1, 0, STONE), Some(AIR));
        assert_eq!(world.dirty_chunk_count(), 2);
        render(&mut renderer, &mut world);
        assert_eq!(world.dirty_chunk_count(), 0);
        assert_eq!(renderer.uploaded_chunk_count(), 1);
        let readded = renderer.resource_stats();
        assert_eq!(readded.live_meshes, 2);
        assert_eq!(readded.voxel_mesh_gpu_uploads_cumulative, 5);
        assert_eq!(readded.voxel_mesh_gpu_releases_cumulative, 4);

        assert_eq!(world.set_block(3, 1, 0, AIR), Some(STONE));
        assert_eq!(world.dirty_chunk_count(), 2);
        render(&mut renderer, &mut world);
        assert_eq!(world.dirty_chunk_count(), 0);
        assert_eq!(renderer.uploaded_chunk_count(), 0);
        let removed_again = renderer.resource_stats();
        assert_eq!(removed_again.live_meshes, 1);
        assert_eq!(removed_again.voxel_mesh_gpu_uploads_cumulative, 5);
        assert_eq!(removed_again.voxel_mesh_gpu_releases_cumulative, 5);
        render(&mut renderer, &mut world);
        let stable_empty = renderer.resource_stats();
        assert_eq!(stable_empty.live_meshes, 1);
        assert_eq!(stable_empty.voxel_mesh_gpu_uploads_cumulative, 5);
        assert_eq!(stable_empty.voxel_mesh_gpu_releases_cumulative, 5);
    }
}
