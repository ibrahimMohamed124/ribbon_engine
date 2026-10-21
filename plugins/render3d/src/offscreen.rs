//! Offscreen native 3D rendering for mutable voxel worlds.
//!
//! This is an engine rendering API, not a game renderer: callers provide a chunked
//! voxel resource, camera, palette, lighting and additional mesh instances. The engine
//! owns dirty-chunk remeshing consumption, GPU buffer upload/eviction, depth, shadows,
//! HDR tone mapping and readback to the backend-neutral framebuffer contract.

use crate::components::SunLight;
use crate::mesh::Vertex;
use crate::post::{HdrTarget, ToneMapper, HDR_FORMAT};
use crate::renderer::{Draw, MeshId, MeshRenderer, Scene};
use engine_render::gpu::GpuContext;
use engine_render::gpu_window::clear_view;
use engine_render::{graph::RenderError, Framebuffer};
use glam::{Mat4, Vec3};
use plugin_voxel::{ChunkCoord, ChunkedVoxelWorld, VoxelChunkMesh};
use std::collections::BTreeMap;
use std::sync::Arc;

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

/// Linear RGB tint for one voxel block identifier. Block identity/palette policy belongs
/// to the application, while this engine type only carries rendering data.
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

/// GPU renderer that incrementally uploads meshes produced by [`ChunkedVoxelWorld`].
/// It releases replaced/empty chunk geometry and reuses mesh slots across edits.
pub struct OffscreenVoxelRenderer {
    // Drop GPU-owned render resources before the context/device/instance.
    renderer: MeshRenderer,
    hdr: HdrTarget,
    tonemapper: ToneMapper,
    uploaded_chunks: BTreeMap<ChunkCoord, Vec<UploadedChunkMesh>>,
    size: (u32, u32),
    context: GpuContext,
}

impl OffscreenVoxelRenderer {
    /// Create an offscreen renderer that returns pixels through the engine framebuffer
    /// contract. GPU initialization errors are reported so applications can select a fallback.
    pub fn new(width: u32, height: u32) -> Result<Self, RenderError> {
        let size = (width.max(1), height.max(1));
        let context = pollster::block_on(GpuContext::new_offscreen(size.0, size.1))?;
        let renderer = MeshRenderer::new(context.device(), context.queue(), HDR_FORMAT);
        let hdr = HdrTarget::new(context.device(), size);
        let tonemapper = ToneMapper::new_offscreen_unorm(context.device(), &hdr.view, size);
        Ok(Self {
            renderer,
            hdr,
            tonemapper,
            uploaded_chunks: BTreeMap::new(),
            size,
            context,
        })
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

    /// Resize offscreen and HDR targets. Zero-sized window events are ignored safely.
    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 || self.size == (width, height) {
            return;
        }
        self.context.resize(width, height);
        self.hdr = HdrTarget::new(self.context.device(), (width, height));
        self.tonemapper =
            ToneMapper::new_offscreen_unorm(self.context.device(), &self.hdr.view, (width, height));
        self.size = (width, height);
    }

    /// Apply pending world edits to GPU meshes and render the current scene. Passing `None`
    /// for `camera` clears the frame and returns a blank framebuffer, useful for menus.
    ///
    /// `block_palette` uses linear RGB. Missing block IDs fall back to neutral gray.
    pub fn render(
        &mut self,
        world: &mut ChunkedVoxelWorld,
        camera: Option<OffscreenCamera3D>,
        block_palette: &[VoxelBlockColor],
        sun: SunLight,
        objects: &[OffscreenDraw3D],
        clear_color: [f64; 4],
    ) -> Result<Framebuffer, RenderError> {
        self.sync_dirty_chunks(world);
        clear_view(
            self.context.device(),
            self.context.queue(),
            &self.hdr.view,
            clear_color,
        );

        if let Some(camera) = camera {
            let mut draws = Vec::new();
            for meshes in self.uploaded_chunks.values() {
                for uploaded in meshes {
                    let color = block_palette
                        .iter()
                        .find(|entry| entry.block == uploaded.source.block)
                        .map_or([0.58, 0.58, 0.58], |entry| entry.color);
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
                    &self.hdr.view,
                    self.size,
                    &scene,
                );
            }
        }

        self.tonemapper.render(
            self.context.device(),
            self.context.queue(),
            self.context.target().view(),
        );
        self.context.readback()
    }

    fn sync_dirty_chunks(&mut self, world: &mut ChunkedVoxelWorld) {
        for update in world.rebuild_dirty_chunks() {
            if let Some(previous) = self.uploaded_chunks.remove(&update.coord) {
                for uploaded in previous {
                    self.renderer.release_mesh(uploaded.id);
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
                uploaded_chunk.push(UploadedChunkMesh { source, id });
            }
            if !uploaded_chunk.is_empty() {
                self.uploaded_chunks.insert(update.coord, uploaded_chunk);
            }
        }
    }
}
