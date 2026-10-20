//! Offscreen native 3D rendering for mutable voxel worlds.
//!
//! This is an engine rendering API, not a game renderer: callers provide a chunked
//! voxel resource, camera, palette, lighting and additional mesh instances. The engine
//! owns dirty-chunk remeshing consumption, GPU buffer upload/eviction, depth, shadows,
//! HDR tone mapping and readback to the backend-neutral framebuffer contract.

use crate::components::SunLight;
use crate::mesh::Vertex;
use crate::post::{HdrTarget, ToneMapper, HDR_FORMAT};
use crate::renderer::{Draw, MaterialId, MeshId, MeshRenderer, Scene, TextureData};
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

/// Texture-backed block material for general voxel worlds. Texture data is supplied by the
/// application, while upload, sampling, and GPU lifetime remain owned by the engine.
/// The texture and PBR factors for a block ID are cached on first use; `color` is a per-frame
/// linear tint and may change with visual settings.
#[derive(Clone, Copy, Debug)]
pub struct VoxelBlockMaterial<'a> {
    pub block: u8,
    pub color: [f32; 3],
    pub metallic: f32,
    pub roughness: f32,
    pub base_color_texture: Option<TextureData<'a>>,
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
    uploaded_materials: BTreeMap<u8, MaterialId>,
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
        let tonemapper = ToneMapper::new_offscreen_unorm(context.device(), &hdr.view);
        Ok(Self {
            renderer,
            hdr,
            tonemapper,
            uploaded_chunks: BTreeMap::new(),
            uploaded_materials: BTreeMap::new(),
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
        self.tonemapper = ToneMapper::new_offscreen_unorm(self.context.device(), &self.hdr.view);
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
        let materials: Vec<VoxelBlockMaterial<'_>> = block_palette
            .iter()
            .map(|entry| VoxelBlockMaterial {
                block: entry.block,
                color: entry.color,
                metallic: 0.0,
                roughness: 0.82,
                base_color_texture: None,
            })
            .collect();
        self.render_with_materials(world, camera, &materials, sun, objects, clear_color)
    }

    /// Render a mutable voxel world using application-provided block materials. Albedo textures
    /// are cached once per block ID; use `color` for dynamic tints such as a game's visual style.
    /// Chunk remeshing, texture uploads, GPU buffers, shadows, tone mapping, and readback remain
    /// generic engine responsibilities.
    pub fn render_with_materials(
        &mut self,
        world: &mut ChunkedVoxelWorld,
        camera: Option<OffscreenCamera3D>,
        block_materials: &[VoxelBlockMaterial<'_>],
        sun: SunLight,
        objects: &[OffscreenDraw3D],
        clear_color: [f64; 4],
    ) -> Result<Framebuffer, RenderError> {
        self.sync_dirty_chunks(world);
        self.sync_block_materials(block_materials);
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
                    let block_material = block_materials
                        .iter()
                        .find(|entry| entry.block == uploaded.source.block);
                    let color = block_material.map_or([0.58, 0.58, 0.58], |entry| entry.color);
                    let material = self
                        .uploaded_materials
                        .get(&uploaded.source.block)
                        .copied()
                        .unwrap_or(MeshRenderer::WHITE);
                    draws.push(Draw {
                        mesh: uploaded.id,
                        material,
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

    fn sync_block_materials(&mut self, materials: &[VoxelBlockMaterial<'_>]) {
        for material in materials {
            if self.uploaded_materials.contains_key(&material.block) {
                continue;
            }
            let id = self.renderer.upload_pbr_material(
                self.context.device(),
                self.context.queue(),
                [1.0, 1.0, 1.0, 1.0],
                material.metallic,
                material.roughness,
                material.base_color_texture,
            );
            self.uploaded_materials.insert(material.block, id);
        }
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
