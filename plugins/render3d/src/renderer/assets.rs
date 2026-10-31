use super::draw::texture_mip_bytes;
use super::*;

impl MeshRenderer {
    pub fn upload_mesh(
        &mut self,
        device: &wgpu::Device,
        vertices: &[Vertex],
        indices: &[u32],
    ) -> MeshId {
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mesh vertices"),
            contents: bytemuck::cast_slice(vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mesh indices"),
            contents: bytemuck::cast_slice(indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let mut bounds_min = Vec3::splat(f32::INFINITY);
        let mut bounds_max = Vec3::splat(f32::NEG_INFINITY);
        for vertex in vertices {
            let position = Vec3::from(vertex.position);
            bounds_min = bounds_min.min(position);
            bounds_max = bounds_max.max(position);
        }
        let bounds_center = if vertices.is_empty() {
            Vec3::ZERO
        } else {
            (bounds_min + bounds_max) * 0.5
        };
        let bounds_radius = vertices
            .iter()
            .map(|vertex| (Vec3::from(vertex.position) - bounds_center).length())
            .fold(0.0_f32, f32::max);
        let estimated_bytes = (vertices.len() as u64)
            .saturating_mul(std::mem::size_of::<Vertex>() as u64)
            .saturating_add(
                (indices.len() as u64).saturating_mul(std::mem::size_of::<u32>() as u64),
            );
        let mesh = GpuMesh {
            vertices: vertex_buffer,
            indices: index_buffer,
            index_count: indices.len() as u32,
            bounds_center,
            bounds_radius,
            estimated_bytes,
        };
        let id = if let Some(index) = self.free_mesh_slots.pop() {
            let slot = &mut self.meshes[index];
            slot.generation = slot.generation.wrapping_add(1).max(1);
            slot.mesh = Some(mesh);
            MeshId {
                slot: index,
                generation: slot.generation,
            }
        } else {
            let index = self.meshes.len();
            self.meshes.push(MeshSlot {
                generation: 0,
                mesh: Some(mesh),
            });
            MeshId {
                slot: index,
                generation: 0,
            }
        };
        self.update_peak_resource_estimate();
        id
    }

    /// Release a previously uploaded dynamic mesh. Built-in meshes are permanent, and
    /// released slots are reused by later uploads so repeated chunk edits do not grow the
    /// GPU mesh table without bound. Stale handles are rejected even after the slot is reused.
    pub fn release_mesh(&mut self, mesh: MeshId) -> bool {
        if mesh == Self::CUBE {
            return false;
        }
        let Some(slot) = self.meshes.get_mut(mesh.slot) else {
            return false;
        };
        if slot.generation != mesh.generation || slot.mesh.is_none() {
            return false;
        }
        slot.mesh = None;
        self.free_mesh_slots.push(mesh.slot);
        true
    }

    /// Release a dynamic material and allow its slot to be reused with a new generation.
    ///
    /// The built-in white material is permanent. Callers must first remove every future draw that
    /// references the handle; already-submitted GPU commands retain their own wgpu resource refs.
    pub fn release_material(&mut self, material: MaterialId) -> bool {
        if material == Self::WHITE {
            return false;
        }
        let Some(slot) = self.materials.get_mut(material.slot) else {
            return false;
        };
        if slot.generation != material.generation || slot.material.is_none() {
            return false;
        }
        slot.material = None;
        self.free_material_slots.push(material.slot);
        true
    }

    /// Current tracked GPU allocation estimates for this renderer.
    pub fn resource_stats(&self) -> MeshRendererResourceStats {
        let mut stats = MeshRendererResourceStats {
            visible_mesh_instances: self.last_visible_mesh_instances,
            visible_draw_calls: self.last_visible_draw_calls,
            shadow_draw_calls: self.last_shadow_draw_calls,
            ..MeshRendererResourceStats::default()
        };
        for mesh in self.meshes.iter().filter_map(|slot| slot.mesh.as_ref()) {
            stats.live_meshes = stats.live_meshes.saturating_add(1);
            stats.mesh_bytes_estimate = stats
                .mesh_bytes_estimate
                .saturating_add(mesh.estimated_bytes);
        }
        for material in self
            .materials
            .iter()
            .filter_map(|slot| slot.material.as_ref())
        {
            stats.live_materials = stats.live_materials.saturating_add(1);
            stats.material_uniform_bytes_estimate = stats
                .material_uniform_bytes_estimate
                .saturating_add(material.uniform_bytes_estimate);
        }
        // The cache stores weak references only, so this counts each unique live texture exactly
        // once without keeping textures alive after the last material releases them.
        for texture in self.texture_cache.values().filter_map(Weak::upgrade) {
            stats.live_material_textures = stats.live_material_textures.saturating_add(1);
            stats.material_texture_bytes_estimate = stats
                .material_texture_bytes_estimate
                .saturating_add(texture.estimated_bytes);
        }
        stats.material_bytes_estimate = stats
            .material_texture_bytes_estimate
            .saturating_add(stats.material_uniform_bytes_estimate);
        stats.material_texture_cache_hits = self.texture_cache_hits;
        stats.material_texture_cache_misses = self.texture_cache_misses;
        stats.instance_buffer_grows_cumulative = self.instance_buffer_grows_cumulative;
        stats.instance_buffer_shrinks_cumulative = self.instance_buffer_shrinks_cumulative;
        stats.buffer_bytes_estimate = (std::mem::size_of::<Globals>() as u64)
            .saturating_add(2_u64.saturating_mul(std::mem::size_of::<ShadowGlobals>() as u64))
            .saturating_add(
                (self.instance_capacity as u64)
                    .saturating_mul(std::mem::size_of::<RawInstance>() as u64)
                    .saturating_mul(2),
            )
            .saturating_add(
                self.gpu_timing
                    .as_ref()
                    .map(|profiler| profiler.snapshot().buffer_bytes_estimate)
                    .unwrap_or(0),
            );
        stats.instance_capacity = self.instance_capacity;
        // Two 1024x1024 Depth32Float shadow targets are fixed for this renderer.
        stats.depth_target_bytes_estimate = 2_u64
            .saturating_mul(1024)
            .saturating_mul(1024)
            .saturating_mul(4);
        if let Some(depth) = &self.depth {
            stats.depth_target_bytes_estimate = stats.depth_target_bytes_estimate.saturating_add(
                u64::from(depth.size.0)
                    .saturating_mul(u64::from(depth.size.1))
                    .saturating_mul(4),
            );
        }
        stats.peak_bytes_estimate = self
            .peak_resource_bytes_estimate
            .max(stats.total_bytes_estimate());
        stats
    }

    /// Poll completed GPU timestamp queries without blocking for unfinished submissions.
    pub fn poll_gpu_timing(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        if let Some(profiler) = self.gpu_timing.as_mut() {
            profiler.poll(device, queue);
        }
    }

    /// Most recent asynchronous GPU timing result and profiler support counters.
    pub fn gpu_timing_snapshot(&self) -> GpuTimingSnapshot {
        self.gpu_timing
            .as_ref()
            .map(GpuTimestampProfiler::snapshot)
            .unwrap_or_default()
    }

    pub(crate) fn drain_gpu_timing_samples(&mut self) -> Vec<GpuCompletedSample> {
        self.gpu_timing
            .as_mut()
            .map_or_else(Vec::new, GpuTimestampProfiler::drain_completed)
    }

    pub fn clear_gpu_timing_samples(&mut self) {
        if let Some(profiler) = self.gpu_timing.as_mut() {
            profiler.clear_completed();
        }
    }

    pub(super) fn update_peak_resource_estimate(&mut self) {
        let estimate = self.resource_stats().total_bytes_estimate();
        self.peak_resource_bytes_estimate = self.peak_resource_bytes_estimate.max(estimate);
    }

    /// Upload a dielectric material with default metallic-roughness factors.
    pub fn upload_material(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        base_color: [f32; 4],
        texture: Option<TextureData<'_>>,
    ) -> MaterialId {
        self.upload_pbr_material(device, queue, base_color, 0.0, 0.65, texture)
    }

    /// Upload a glTF-style metallic-roughness material and optional sRGB base-color texture.
    pub fn upload_pbr_material(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        base_color: [f32; 4],
        metallic: f32,
        roughness: f32,
        texture: Option<TextureData<'_>>,
    ) -> MaterialId {
        self.upload_pbr_material_with_maps(
            device,
            queue,
            PbrMaterialDesc {
                base_color,
                metallic,
                roughness,
                base_color_texture: texture,
                metallic_roughness_texture: None,
                normal_texture: None,
                occlusion_texture: None,
                occlusion_strength: 1.0,
                emissive_texture: None,
                emissive_factor: [0.0; 3],
            },
        )
    }

    fn upload_cached_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        source: TextureData<'_>,
        label: &'static str,
        srgb: bool,
    ) -> Arc<GpuTexture> {
        let key = texture_cache_key(source, srgb);
        if let Some(texture) = self.texture_cache.get(&key).and_then(Weak::upgrade) {
            self.texture_cache_hits = self.texture_cache_hits.saturating_add(1);
            return texture;
        }

        self.texture_cache_misses = self.texture_cache_misses.saturating_add(1);
        // Keep dead weak entries bounded across repeated hot reloads and scene changes.
        if self.texture_cache.len() >= 128 {
            self.texture_cache
                .retain(|_, texture| texture.strong_count() > 0);
        }
        let (texture, view) = upload_rgba_texture(device, queue, source, label, srgb);
        let shared = Arc::new(GpuTexture {
            _texture: texture,
            view,
            estimated_bytes: texture_mip_bytes(source.width, source.height),
        });
        self.texture_cache.insert(key, Arc::downgrade(&shared));
        shared
    }

    /// Upload a material and its optional glTF metallic-roughness texture maps.
    pub fn upload_pbr_material_with_maps(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        material: PbrMaterialDesc<'_>,
    ) -> MaterialId {
        let PbrMaterialDesc {
            base_color,
            metallic,
            roughness,
            base_color_texture,
            metallic_roughness_texture,
            normal_texture,
            occlusion_texture,
            occlusion_strength,
            emissive_texture,
            emissive_factor,
        } = material;
        let white = [255_u8; 4];
        let base_source = valid_texture(base_color_texture, &white);
        let surface_source = valid_texture(metallic_roughness_texture, &white);
        let base_texture = self.upload_cached_texture(
            device,
            queue,
            base_source,
            "material sRGB base color",
            true,
        );
        // Metallic and roughness values are linear data, not sRGB colors.
        let surface_texture = self.upload_cached_texture(
            device,
            queue,
            surface_source,
            "material metallic roughness map",
            false,
        );
        let flat_normal = [128_u8, 128, 255, 255];
        let white_occlusion = [255_u8; 4];
        // With no emissive map, white leaves the glTF emissiveFactor unmodified.
        let white_emissive = [255_u8; 4];
        let normal_source = valid_texture(normal_texture, &flat_normal);
        let occlusion_source = valid_texture(occlusion_texture, &white_occlusion);
        let emissive_source = valid_texture(emissive_texture, &white_emissive);
        let normal_gpu_texture =
            self.upload_cached_texture(device, queue, normal_source, "material normal map", false);
        let occlusion_gpu_texture = self.upload_cached_texture(
            device,
            queue,
            occlusion_source,
            "material ambient occlusion map",
            false,
        );
        let emissive_gpu_texture = self.upload_cached_texture(
            device,
            queue,
            emissive_source,
            "material emissive map",
            true,
        );
        let factors = MaterialUniform {
            base_color,
            surface: [
                metallic.clamp(0.0, 1.0),
                roughness.clamp(0.04, 1.0),
                occlusion_strength.clamp(0.0, 1.0),
                0.0,
            ],
            emissive: [
                emissive_factor[0].max(0.0),
                emissive_factor[1].max(0.0),
                emissive_factor[2].max(0.0),
                1.0,
            ],
        };
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("PBR material factors"),
            contents: bytemuck::bytes_of(&factors),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("material bind group"),
            layout: &self.material_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&base_texture.view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&surface_texture.view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&normal_gpu_texture.view),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&occlusion_gpu_texture.view),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(&emissive_gpu_texture.view),
                },
            ],
        });
        let material = GpuMaterial {
            bind_group,
            _uniform: uniform,
            _base_texture: base_texture,
            _metallic_roughness_texture: surface_texture,
            _normal_texture: normal_gpu_texture,
            _occlusion_texture: occlusion_gpu_texture,
            _emissive_texture: emissive_gpu_texture,
            uniform_bytes_estimate: std::mem::size_of::<MaterialUniform>() as u64,
        };
        let id = if let Some(index) = self.free_material_slots.pop() {
            let slot = &mut self.materials[index];
            slot.generation = slot.generation.wrapping_add(1).max(1);
            slot.material = Some(material);
            MaterialId {
                slot: index,
                generation: slot.generation,
            }
        } else {
            let index = self.materials.len();
            self.materials.push(MaterialSlot {
                generation: 0,
                material: Some(material),
            });
            MaterialId {
                slot: index,
                generation: 0,
            }
        };
        self.update_peak_resource_estimate();
        id
    }
}
