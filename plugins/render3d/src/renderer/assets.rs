use super::draw::texture_mip_bytes;
use super::*;
use crate::mesh::Vertex;
use std::collections::BTreeMap;

/// Precomputed per-joint and unweighted local bounds used to conservatively enclose the
/// production shader's eight-influence linear-blend skinning path.
fn build_skinned_caster_metadata(
    vertices: &[crate::mesh::SkinnedVertex],
    has_finite_mesh_bounds: bool,
) -> (Vec<(u16, Aabb3)>, Option<Aabb3>, f32, f32, bool) {
    let mut joint_bounds = BTreeMap::<u16, Aabb3>::new();
    let mut unweighted_bounds: Option<Aabb3> = None;
    let mut min_skin_weight_sum = f32::INFINITY;
    let mut max_skin_weight_sum = 0.0_f32;
    let mut skin_bounds_valid = has_finite_mesh_bounds;
    for vertex in vertices {
        let position = Vec3::from(vertex.position);
        if !position.is_finite() {
            skin_bounds_valid = false;
            continue;
        }
        let mut weight_sum = 0.0_f32;
        for weight in vertex.weights {
            if !weight.is_finite() || weight < 0.0 {
                skin_bounds_valid = false;
            }
            weight_sum += weight;
        }
        if !weight_sum.is_finite() || weight_sum < 0.0 {
            skin_bounds_valid = false;
            continue;
        }
        // Match the production WGSL threshold and the glTF model-upload validation.
        if weight_sum > 1.0e-6 {
            if (weight_sum - 1.0).abs() > 1.0e-3 {
                skin_bounds_valid = false;
            }
            min_skin_weight_sum = min_skin_weight_sum.min(weight_sum);
            max_skin_weight_sum = max_skin_weight_sum.max(weight_sum);
            for influence in 0..8 {
                if vertex.weights[influence] <= 0.0 {
                    continue;
                }
                let joint_index = vertex.joints[influence];
                joint_bounds
                    .entry(joint_index)
                    .and_modify(|bounds| bounds.include_point(position))
                    .or_insert(Aabb3 {
                        min: position,
                        max: position,
                    });
            }
        } else if let Some(bounds) = &mut unweighted_bounds {
            bounds.include_point(position);
        } else {
            unweighted_bounds = Some(Aabb3 {
                min: position,
                max: position,
            });
        }
    }
    if min_skin_weight_sum == f32::INFINITY {
        min_skin_weight_sum = 1.0;
    }
    (
        joint_bounds.into_iter().collect(),
        unweighted_bounds,
        max_skin_weight_sum,
        min_skin_weight_sum,
        skin_bounds_valid,
    )
}

impl MeshRenderer {
    pub fn upload_mesh(
        &mut self,
        device: &wgpu::Device,
        vertices: &[Vertex],
        indices: &[u32],
    ) -> MeshId {
        // Compatibility adapter for existing procedural callers. Zero-weight legacy meshes are
        // static and use the compact GPU stream; weighted inputs retain the full eight-influence
        // skinned stream. Both paths generate a stable tangent frame and alias UV1 to UV0.
        let positions: Vec<[f32; 3]> = vertices.iter().map(|vertex| vertex.position).collect();
        let normals: Vec<[f32; 3]> = vertices.iter().map(|vertex| vertex.normal).collect();
        let uv0: Vec<[f32; 2]> = vertices.iter().map(|vertex| vertex.uv).collect();
        let tangents = crate::mesh::generate_tangents(&positions, &normals, &uv0, indices);
        if crate::mesh::legacy_vertices_require_skinning(vertices) {
            let skinned: Vec<crate::mesh::SkinnedVertex> = vertices
                .iter()
                .enumerate()
                .map(|(index, vertex)| crate::mesh::SkinnedVertex {
                    position: vertex.position,
                    normal: vertex.normal,
                    uv: vertex.uv,
                    tangent: tangents.get(index).copied().unwrap_or([1.0, 0.0, 0.0, 1.0]),
                    uv1: vertex.uv,
                    joints: vertex.joints,
                    weights: vertex.weights,
                })
                .collect();
            self.upload_skinned_mesh(device, &skinned, indices)
        } else {
            let static_vertices: Vec<crate::mesh::StaticVertex> = vertices
                .iter()
                .enumerate()
                .map(|(index, vertex)| crate::mesh::StaticVertex {
                    position: vertex.position,
                    normal: vertex.normal,
                    uv: vertex.uv,
                    tangent: tangents.get(index).copied().unwrap_or([1.0, 0.0, 0.0, 1.0]),
                    uv1: vertex.uv,
                })
                .collect();
            self.upload_static_mesh(device, &static_vertices, indices)
        }
    }

    /// Upload eight-influence skinned geometry with tangent and UV0/UV1 streams (104 bytes per vertex).
    pub fn upload_skinned_mesh(
        &mut self,
        device: &wgpu::Device,
        vertices: &[crate::mesh::SkinnedVertex],
        indices: &[u32],
    ) -> MeshId {
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("skinned mesh vertices"),
            contents: bytemuck::cast_slice(vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("skinned mesh indices"),
            contents: bytemuck::cast_slice(indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let local_bounds =
            Aabb3::from_points(vertices.iter().map(|vertex| Vec3::from(vertex.position)));
        let bounds_center = local_bounds
            .map(|bounds| (bounds.min + bounds.max) * 0.5)
            .unwrap_or(Vec3::ZERO);
        let bounds_radius = local_bounds
            .map(|_| {
                vertices
                    .iter()
                    .map(|vertex| (Vec3::from(vertex.position) - bounds_center).length())
                    .fold(0.0_f32, f32::max)
            })
            .unwrap_or(if vertices.is_empty() {
                0.0
            } else {
                f32::INFINITY
            });

        let (
            joint_influence_bounds,
            unweighted_bounds,
            max_skin_weight_sum,
            min_skin_weight_sum,
            skin_bounds_valid,
        ) = build_skinned_caster_metadata(vertices, local_bounds.is_some());
        let estimated_bytes = (vertices.len() as u64)
            .saturating_mul(std::mem::size_of::<crate::mesh::SkinnedVertex>() as u64)
            .saturating_add(
                (indices.len() as u64).saturating_mul(std::mem::size_of::<u32>() as u64),
            );
        self.insert_gpu_mesh(GpuMesh {
            vertices: vertex_buffer,
            vertex_layout: MeshVertexLayout::Skinned,
            indices: index_buffer,
            index_count: indices.len() as u32,
            bounds_center,
            bounds_radius,
            local_bounds,
            joint_influence_bounds,
            unweighted_bounds,
            max_skin_weight_sum,
            min_skin_weight_sum,
            skin_bounds_valid,
            estimated_bytes,
        })
    }

    /// Upload a compact non-skinned mesh (56 bytes per vertex: position, normal, UV0, tangent, UV1).
    /// The legacy [`Self::upload_mesh`] and [`Self::upload_skinned_mesh`] remain available for
    /// existing callers; new unskinned glTF primitives use this format.
    pub fn upload_static_mesh(
        &mut self,
        device: &wgpu::Device,
        vertices: &[crate::mesh::StaticVertex],
        indices: &[u32],
    ) -> MeshId {
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("static mesh vertices"),
            contents: bytemuck::cast_slice(vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("static mesh indices"),
            contents: bytemuck::cast_slice(indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let local_bounds =
            Aabb3::from_points(vertices.iter().map(|vertex| Vec3::from(vertex.position)));
        let bounds_center = local_bounds
            .map(|bounds| (bounds.min + bounds.max) * 0.5)
            .unwrap_or(Vec3::ZERO);
        let bounds_radius = local_bounds
            .map(|_| {
                vertices
                    .iter()
                    .map(|vertex| (Vec3::from(vertex.position) - bounds_center).length())
                    .fold(0.0_f32, f32::max)
            })
            .unwrap_or(if vertices.is_empty() {
                0.0
            } else {
                f32::INFINITY
            });
        let estimated_bytes = (vertices.len() as u64)
            .saturating_mul(std::mem::size_of::<crate::mesh::StaticVertex>() as u64)
            .saturating_add(
                (indices.len() as u64).saturating_mul(std::mem::size_of::<u32>() as u64),
            );
        self.insert_gpu_mesh(GpuMesh {
            vertices: vertex_buffer,
            vertex_layout: MeshVertexLayout::Static,
            indices: index_buffer,
            index_count: indices.len() as u32,
            bounds_center,
            bounds_radius,
            local_bounds,
            joint_influence_bounds: Vec::new(),
            unweighted_bounds: None,
            max_skin_weight_sum: 0.0,
            min_skin_weight_sum: 1.0,
            skin_bounds_valid: true,
            estimated_bytes,
        })
    }

    fn insert_gpu_mesh(&mut self, mesh: GpuMesh) -> MeshId {
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

    /// Upload an independently leased RGBA8 texture using the renderer's shared content cache.
    ///
    /// The returned generation-checked handle owns a strong reference independently of PBR
    /// materials. The caller must retain the handle until every consumer has finished using it.
    pub fn upload_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        source: TextureData<'_>,
        label: &'static str,
        srgb: bool,
    ) -> TextureId {
        let texture = self.upload_cached_texture(device, queue, source, label, srgb, None);
        let id = if let Some(index) = self.free_texture_slots.pop() {
            let slot = &mut self.textures[index];
            slot.generation = slot.generation.wrapping_add(1).max(1);
            slot.texture = Some(texture);
            TextureId {
                slot: index,
                generation: slot.generation,
            }
        } else {
            let index = self.textures.len();
            self.textures.push(TextureSlot {
                generation: 0,
                texture: Some(texture),
            });
            TextureId {
                slot: index,
                generation: 0,
            }
        };
        self.update_peak_resource_estimate();
        id
    }

    /// Resolve a live standalone texture handle to its GPU view on the renderer-owning thread.
    /// A stale or released generation returns `None`.
    pub fn texture_view(&self, texture: TextureId) -> Option<&wgpu::TextureView> {
        let slot = self.textures.get(texture.slot)?;
        if slot.generation != texture.generation {
            return None;
        }
        slot.texture.as_ref().map(|texture| &texture.view)
    }

    /// Release an independently uploaded texture. Stale generation handles are rejected.
    pub fn release_texture(&mut self, texture: TextureId) -> bool {
        let Some(slot) = self.textures.get_mut(texture.slot) else {
            return false;
        };
        if slot.generation != texture.generation || slot.texture.is_none() {
            return false;
        }
        slot.texture = None;
        self.free_texture_slots.push(texture.slot);
        true
    }

    /// Create a standalone sampler on the renderer-owning thread.
    pub fn upload_sampler(&mut self, device: &wgpu::Device, desc: GpuSamplerDesc) -> SamplerId {
        let address_mode = |mode| match mode {
            GpuSamplerAddressMode::ClampToEdge => wgpu::AddressMode::ClampToEdge,
            GpuSamplerAddressMode::Repeat => wgpu::AddressMode::Repeat,
            GpuSamplerAddressMode::MirrorRepeat => wgpu::AddressMode::MirrorRepeat,
        };
        let filter = |mode| match mode {
            GpuSamplerFilter::Nearest => wgpu::FilterMode::Nearest,
            GpuSamplerFilter::Linear => wgpu::FilterMode::Linear,
        };
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("streamed standalone sampler"),
            address_mode_u: address_mode(desc.address_mode_u),
            address_mode_v: address_mode(desc.address_mode_v),
            address_mode_w: address_mode(desc.address_mode_w),
            mag_filter: filter(desc.mag_filter),
            min_filter: filter(desc.min_filter),
            mipmap_filter: filter(desc.mipmap_filter),
            lod_min_clamp: desc.lod_min_clamp,
            lod_max_clamp: desc.lod_max_clamp,
            ..wgpu::SamplerDescriptor::default()
        });
        if let Some(index) = self.free_sampler_slots.pop() {
            let slot = &mut self.samplers[index];
            slot.generation = slot.generation.wrapping_add(1).max(1);
            slot.sampler = Some(sampler);
            SamplerId {
                slot: index,
                generation: slot.generation,
            }
        } else {
            let index = self.samplers.len();
            self.samplers.push(SamplerSlot {
                generation: 0,
                sampler: Some(sampler),
            });
            SamplerId {
                slot: index,
                generation: 0,
            }
        }
    }

    /// Resolve a live standalone sampler handle to its GPU sampler. Stale handles return `None`.
    pub fn sampler(&self, sampler: SamplerId) -> Option<&wgpu::Sampler> {
        let slot = self.samplers.get(sampler.slot)?;
        if slot.generation != sampler.generation {
            return None;
        }
        slot.sampler.as_ref()
    }

    /// Release a standalone sampler. Stale generation handles are rejected.
    pub fn release_sampler(&mut self, sampler: SamplerId) -> bool {
        let Some(slot) = self.samplers.get_mut(sampler.slot) else {
            return false;
        };
        if slot.generation != sampler.generation || slot.sampler.is_none() {
            return false;
        }
        slot.sampler = None;
        self.free_sampler_slots.push(sampler.slot);
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
            particle_instances_drawn: self.last_particle_instances_drawn,
            particle_batches_drawn: self.last_particle_batches_drawn,
            shadow_draw_calls: self.last_shadow_draw_calls,
            skinned_draws_after_palette_budget: self.last_skinned_draws_after_palette_budget,
            skinned_draws_rejected_palette_capacity: self
                .last_skinned_draws_rejected_palette_capacity,
            skin_palette_matrices_staged: self.last_skin_palette_matrices_staged,
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
        // Classify every unique live cached image once. Standalone texture leases may share the
        // same content-addressed Arc with a material; in that case the allocation is attributed
        // to the material category only, never counted twice in total residency.
        let mut material_texture_ids = std::collections::HashSet::new();
        for material in self
            .materials
            .iter()
            .filter_map(|slot| slot.material.as_ref())
        {
            for texture in [
                &material._base_texture,
                &material._metallic_roughness_texture,
                &material._normal_texture,
                &material._occlusion_texture,
                &material._emissive_texture,
            ] {
                material_texture_ids.insert(Arc::as_ptr(texture) as usize);
            }
        }
        for texture in self.texture_cache.values().filter_map(Weak::upgrade) {
            if material_texture_ids.contains(&(Arc::as_ptr(&texture) as usize)) {
                stats.live_material_textures = stats.live_material_textures.saturating_add(1);
                stats.material_texture_bytes_estimate = stats
                    .material_texture_bytes_estimate
                    .saturating_add(texture.estimated_bytes);
            } else {
                stats.live_standalone_textures = stats.live_standalone_textures.saturating_add(1);
                stats.standalone_texture_bytes_estimate = stats
                    .standalone_texture_bytes_estimate
                    .saturating_add(texture.estimated_bytes);
            }
        }
        stats.live_samplers = self
            .samplers
            .iter()
            .filter(|slot| slot.sampler.is_some())
            .count();
        stats.material_bytes_estimate = stats
            .material_texture_bytes_estimate
            .saturating_add(stats.material_uniform_bytes_estimate);
        stats.material_texture_cache_hits = self.texture_cache_hits;
        stats.material_texture_cache_misses = self.texture_cache_misses;
        stats.instance_buffer_grows_cumulative = self.instance_buffer_grows_cumulative;
        stats.instance_buffer_shrinks_cumulative = self.instance_buffer_shrinks_cumulative;
        stats.buffer_bytes_estimate = (std::mem::size_of::<Globals>() as u64)
            .saturating_add(std::mem::size_of::<ShadowDepthDebugUniform>() as u64)
            .saturating_add(
                (super::shadows::DIRECTIONAL_CASCADE_COUNT as u64 + 1)
                    .saturating_mul(std::mem::size_of::<ShadowGlobals>() as u64),
            )
            .saturating_add(
                (MAX_SKIN_MATRICES_PER_FRAME as u64)
                    .saturating_mul(std::mem::size_of::<[[f32; 4]; 4]>() as u64),
            )
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
        // Four 1024x1024 directional cascade layers plus one spotlight map.
        stats.depth_target_bytes_estimate = (super::shadows::DIRECTIONAL_CASCADE_COUNT as u64 + 1)
            .saturating_mul(super::shadows::DIRECTIONAL_SHADOW_RESOLUTION as u64)
            .saturating_mul(super::shadows::DIRECTIONAL_SHADOW_RESOLUTION as u64)
            .saturating_mul(std::mem::size_of::<f32>() as u64);
        if let Some(depth) = &self.depth {
            stats.depth_target_bytes_estimate = stats.depth_target_bytes_estimate.saturating_add(
                u64::from(depth.size.0)
                    .saturating_mul(u64::from(depth.size.1))
                    .saturating_mul(4)
                    .saturating_mul(u64::from(depth.sample_count)),
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
        prepared_mips: Option<&[(u32, u32, Vec<u8>)]>,
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
        let (texture, view) =
            upload_rgba_texture(device, queue, source, label, srgb, prepared_mips);
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
        self.upload_pbr_material_with_precomputed_mips(
            device,
            queue,
            material,
            PbrMaterialMipmaps::default(),
        )
    }

    pub(crate) fn upload_pbr_material_with_precomputed_mips(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        material: PbrMaterialDesc<'_>,
        mipmaps: PbrMaterialMipmaps<'_>,
    ) -> MaterialId {
        self.upload_pbr_material_with_precomputed_mips_and_options(
            device,
            queue,
            material,
            mipmaps,
            PbrMaterialRenderOptions::default(),
        )
    }

    /// Upload a material with explicit alpha/sidedness state. This is additive to the original
    /// PBR upload API and is used by glTF model uploads.
    pub fn upload_pbr_material_with_options(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        material: PbrMaterialDesc<'_>,
        options: PbrMaterialRenderOptions,
    ) -> MaterialId {
        self.upload_pbr_material_with_precomputed_mips_and_options(
            device,
            queue,
            material,
            PbrMaterialMipmaps::default(),
            options,
        )
    }

    pub(crate) fn upload_pbr_material_with_precomputed_mips_and_options(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        material: PbrMaterialDesc<'_>,
        mipmaps: PbrMaterialMipmaps<'_>,
        options: PbrMaterialRenderOptions,
    ) -> MaterialId {
        self.upload_pbr_material_with_precomputed_mips_options_and_uvs(
            device,
            queue,
            material,
            mipmaps,
            options,
            MaterialTextureCoordinates::default(),
        )
    }

    pub(crate) fn upload_pbr_material_with_precomputed_mips_options_and_uvs(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        material: PbrMaterialDesc<'_>,
        mipmaps: PbrMaterialMipmaps<'_>,
        options: PbrMaterialRenderOptions,
        texture_uvs: MaterialTextureCoordinates,
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
        let PbrMaterialRenderOptions {
            alpha_mode,
            alpha_cutoff,
            double_sided,
        } = options;
        let white = [255_u8; 4];
        let base_source = valid_texture(base_color_texture, &white);
        let surface_source = valid_texture(metallic_roughness_texture, &white);
        let base_texture = self.upload_cached_texture(
            device,
            queue,
            base_source,
            "material sRGB base color",
            true,
            mipmaps.base_color,
        );
        // Metallic and roughness values are linear data, not sRGB colors.
        let surface_texture = self.upload_cached_texture(
            device,
            queue,
            surface_source,
            "material metallic roughness map",
            false,
            mipmaps.metallic_roughness,
        );
        let flat_normal = [128_u8, 128, 255, 255];
        let white_occlusion = [255_u8; 4];
        // With no emissive map, white leaves the glTF emissiveFactor unmodified.
        let white_emissive = [255_u8; 4];
        let normal_source = valid_texture(normal_texture, &flat_normal);
        let occlusion_source = valid_texture(occlusion_texture, &white_occlusion);
        let emissive_source = valid_texture(emissive_texture, &white_emissive);
        let normal_gpu_texture = self.upload_cached_texture(
            device,
            queue,
            normal_source,
            "material normal map",
            false,
            mipmaps.normal,
        );
        let occlusion_gpu_texture = self.upload_cached_texture(
            device,
            queue,
            occlusion_source,
            "material ambient occlusion map",
            false,
            mipmaps.occlusion,
        );
        let emissive_gpu_texture = self.upload_cached_texture(
            device,
            queue,
            emissive_source,
            "material emissive map",
            true,
            mipmaps.emissive,
        );
        let (texture_uv_sets, texture_uv_sets_extra) =
            crate::renderer::encode_material_texture_coordinates(texture_uvs);
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
            render: [
                match alpha_mode {
                    AlphaMode::Opaque => 0.0,
                    AlphaMode::Mask => 1.0,
                    AlphaMode::Blend => 2.0,
                },
                if alpha_cutoff.is_finite() {
                    alpha_cutoff.clamp(0.0, 1.0)
                } else {
                    0.5
                },
                0.0,
                0.0,
            ],
            texture_uv_sets,
            texture_uv_sets_extra,
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
            alpha_mode,
            double_sided,
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

#[cfg(test)]
mod caster_bounds_tests {
    use super::build_skinned_caster_metadata;
    use crate::mesh::SkinnedVertex;
    use crate::renderer::Aabb3;
    use glam::Vec3;

    fn vertex(joints: [u16; 8], weights: [f32; 8], position: [f32; 3]) -> SkinnedVertex {
        SkinnedVertex {
            position,
            normal: [0.0, 1.0, 0.0],
            uv: [0.0; 2],
            tangent: [1.0, 0.0, 0.0, 1.0],
            uv1: [0.0; 2],
            joints,
            weights,
        }
    }

    #[test]
    fn all_eight_positive_joint_influences_are_recorded() {
        let joints = [0, 1, 2, 3, 4, 5, 6, 7];
        let weights = [0.125; 8];
        let vertices = [vertex(joints, weights, [1.0, 2.0, 3.0])];
        let (joint_bounds, unweighted, max_sum, min_sum, valid) =
            build_skinned_caster_metadata(&vertices, true);
        assert!(valid);
        assert_eq!(joint_bounds.len(), 8);
        assert!(unweighted.is_none());
        assert!((max_sum - 1.0).abs() < 1.0e-6);
        assert!((min_sum - 1.0).abs() < 1.0e-6);
        for (joint, bounds) in joint_bounds {
            assert!(usize::from(joint) < 8);
            assert_eq!(bounds.min, Vec3::new(1.0, 2.0, 3.0));
            assert_eq!(bounds.max, Vec3::new(1.0, 2.0, 3.0));
        }
    }

    #[test]
    fn zero_weight_joint_indices_are_ignored_and_vertex_uses_unweighted_bounds() {
        let vertices = [vertex([u16::MAX; 8], [0.0; 8], [4.0, -2.0, 1.0])];
        let (joint_bounds, unweighted, max_sum, min_sum, valid) =
            build_skinned_caster_metadata(&vertices, true);
        assert!(valid);
        assert!(joint_bounds.is_empty());
        assert_eq!(
            unweighted.unwrap(),
            Aabb3 {
                min: Vec3::new(4.0, -2.0, 1.0),
                max: Vec3::new(4.0, -2.0, 1.0),
            }
        );
        assert_eq!(max_sum, 0.0);
        assert_eq!(min_sum, 1.0);
    }

    #[test]
    fn invalid_positive_weight_data_disables_skinned_caster_bounds() {
        let invalid_sum = [vertex([9; 8], [0.2; 8], [0.0, 0.0, 0.0])];
        let (_, _, _, _, valid) = build_skinned_caster_metadata(&invalid_sum, true);
        assert!(!valid);

        let invalid_weight = [vertex(
            [0; 8],
            [f32::NAN, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            [0.0; 3],
        )];
        let (_, _, _, _, valid) = build_skinned_caster_metadata(&invalid_weight, true);
        assert!(!valid);
    }
}
