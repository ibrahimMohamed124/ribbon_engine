use super::*;

type StableDrawKey = (MeshId, MaterialId, [u32; 16], [u32; 3], u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ColorPipelineKey {
    OpaqueCull,
    OpaqueDoubleSided,
    BlendCull,
    BlendDoubleSided,
}

fn color_pipeline_key(alpha_mode: AlphaMode, double_sided: bool) -> ColorPipelineKey {
    match (alpha_mode == AlphaMode::Blend, double_sided) {
        (false, false) => ColorPipelineKey::OpaqueCull,
        (false, true) => ColorPipelineKey::OpaqueDoubleSided,
        (true, false) => ColorPipelineKey::BlendCull,
        (true, true) => ColorPipelineKey::BlendDoubleSided,
    }
}

fn compare_visible_order(
    left_blended: bool,
    right_blended: bool,
    left_depth: f32,
    right_depth: f32,
    left_key: StableDrawKey,
    right_key: StableDrawKey,
) -> std::cmp::Ordering {
    match (left_blended, right_blended) {
        (false, false) => left_key.cmp(&right_key),
        (false, true) => std::cmp::Ordering::Less,
        (true, false) => std::cmp::Ordering::Greater,
        (true, true) => right_depth
            .total_cmp(&left_depth)
            .then_with(|| left_key.cmp(&right_key)),
    }
}

fn is_blended(renderer: &MeshRenderer, prepared: &PreparedDraw) -> bool {
    renderer
        .materials
        .get(prepared.draw.material.slot)
        .and_then(|slot| slot.material.as_ref())
        .is_some_and(|material| material.alpha_mode == AlphaMode::Blend)
}

fn camera_forward(view_proj: Mat4, camera_position: Vec3) -> Vec3 {
    let inverse = view_proj.inverse();
    if !inverse
        .to_cols_array()
        .iter()
        .all(|value| value.is_finite())
    {
        return Vec3::NEG_Z;
    }
    let far_h = inverse * Vec4::new(0.0, 0.0, 0.999, 1.0);
    if !far_h.is_finite() || far_h.w.abs() <= 1.0e-6 {
        return Vec3::NEG_Z;
    }
    (far_h.truncate() / far_h.w - camera_position)
        .try_normalize()
        .unwrap_or(Vec3::NEG_Z)
}

fn transparent_camera_depth(
    renderer: &MeshRenderer,
    prepared: &PreparedDraw,
    camera_position: Vec3,
    forward: Vec3,
) -> f32 {
    let center = renderer
        .meshes
        .get(prepared.draw.mesh.slot)
        .and_then(|slot| slot.mesh.as_ref())
        .map(|mesh| prepared.draw.model.transform_point3(mesh.bounds_center))
        .unwrap_or(prepared.draw.model.w_axis.truncate());
    let depth = (center - camera_position).dot(forward);
    if depth.is_finite() {
        depth
    } else {
        (center - camera_position).length()
    }
}

fn palette_order_hash(palette: &[Mat4]) -> u64 {
    // Explicit FNV-1a avoids relying on DefaultHasher's unspecified implementation/seed.
    // The byte order is fixed so equal palettes receive the same stable tie-breaker on hosts.
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for matrix in palette {
        for value in matrix.to_cols_array() {
            for byte in value.to_bits().to_le_bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
    }
    hash
}

fn stable_draw_key(prepared: &PreparedDraw) -> StableDrawKey {
    (
        prepared.draw.mesh,
        prepared.draw.material,
        prepared.draw.model.to_cols_array().map(f32::to_bits),
        prepared.draw.tint.map(f32::to_bits),
        prepared.palette_order_hash,
    )
}

fn sort_visible_draws(renderer: &MeshRenderer, draws: &mut [PreparedDraw], scene: &Scene<'_>) {
    let forward = camera_forward(scene.view_proj, scene.camera_position);
    draws.sort_by(|left, right| {
        let left_blended = is_blended(renderer, left);
        let right_blended = is_blended(renderer, right);
        let left_depth = transparent_camera_depth(renderer, left, scene.camera_position, forward);
        let right_depth = transparent_camera_depth(renderer, right, scene.camera_position, forward);
        compare_visible_order(
            left_blended,
            right_blended,
            left_depth,
            right_depth,
            stable_draw_key(left),
            stable_draw_key(right),
        )
    });
}

/// Append one immutable skin palette to the bounded frame-local upload list.
/// Returns the matrix offset for the instance, or `None` without mutating the destination when
/// the palette is empty or would exceed the renderer's per-frame storage-buffer capacity.
pub(super) fn append_joint_palette(
    palette: &[Mat4],
    destination: &mut Vec<[[f32; 4]; 4]>,
) -> Option<u32> {
    let start = destination.len();
    let end = start.checked_add(palette.len())?;
    if palette.is_empty() || end > MAX_SKIN_MATRICES_PER_FRAME || end > u32::MAX as usize {
        return None;
    }
    let offset = u32::try_from(start).ok()?;
    destination.extend(palette.iter().map(Mat4::to_cols_array_2d));
    Some(offset)
}

impl MeshRenderer {
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        view: &wgpu::TextureView,
        size: (u32, u32),
        scene: &Scene<'_>,
    ) {
        let _ = self.render_internal(
            RenderTarget {
                device,
                queue,
                view,
                resolve_view: None,
                size,
                scene,
            },
            &[],
            None,
        );
    }

    /// Render static and skeletal instances together. Joint palettes are copied to a bounded
    /// frame-local storage buffer before any color or shadow pass consumes them.
    pub fn render_with_skinning(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        view: &wgpu::TextureView,
        size: (u32, u32),
        scene: &Scene<'_>,
        skinned_draws: &[SkinnedDraw],
    ) {
        let _ = self.render_internal(
            RenderTarget {
                device,
                queue,
                view,
                resolve_view: None,
                size,
                scene,
            },
            skinned_draws,
            None,
        );
    }

    pub(crate) fn render_with_skinning_resolve(
        &mut self,
        target: RenderTarget<'_, '_>,
        skinned_draws: &[SkinnedDraw],
    ) {
        let _ = self.render_internal(target, skinned_draws, None);
    }

    pub(crate) fn render_profiled_with_skinning(
        &mut self,
        target: RenderTarget<'_, '_>,
        skinned_draws: &[SkinnedDraw],
        submitted_frame: u64,
    ) -> Vec<RenderPassTiming> {
        self.render_internal(target, skinned_draws, Some(submitted_frame))
    }

    fn render_internal(
        &mut self,
        target: RenderTarget<'_, '_>,
        skinned_draws: &[SkinnedDraw],
        profile_frame: Option<u64>,
    ) -> Vec<RenderPassTiming> {
        let RenderTarget {
            device,
            queue,
            view,
            resolve_view,
            size,
            scene,
        } = target;
        let profile_cpu = profile_frame.is_some();
        let submitted_frame = profile_frame.unwrap_or_default();
        let mut cpu_phases = Vec::new();
        self.last_visible_mesh_instances = 0;
        self.last_visible_draw_calls = 0;
        self.last_particle_instances_drawn = 0;
        self.last_particle_batches_drawn = 0;
        self.last_shadow_draw_calls = 0;
        self.last_skinned_draws_after_palette_budget = 0;
        self.last_skinned_draws_rejected_palette_capacity = 0;
        self.last_skin_palette_matrices_staged = 0;
        if size.0 == 0 || size.1 == 0 {
            return cpu_phases;
        }
        let (particle_instances, particle_batches) = self.particles.prepare(
            device,
            queue,
            scene.particles,
            scene.camera_position,
            scene.camera_rotation,
        );
        self.last_particle_instances_drawn = particle_instances;
        self.last_particle_batches_drawn = particle_batches;
        let mut phase_started = profile_cpu.then(Instant::now);
        if scene.draws.is_empty()
            && skinned_draws.is_empty()
            && scene.particles.is_empty()
            && self.directional_shadow_debug.mode != super::DirectionalShadowDebugMode::DepthLayer
        {
            self.maintain_instance_buffers(device, 0);
            return cpu_phases;
        }
        self.ensure_depth(device, size, self.sample_count);
        debug_assert_eq!(self.sample_count > 1, resolve_view.is_some());
        // Static and skinned draws share the same generation-checked mesh/material path. The
        // skinned palette is retained by Arc through extraction and copied to the GPU once here.
        let mut draws: Vec<PreparedDraw> = scene
            .draws
            .iter()
            .copied()
            .map(|draw| PreparedDraw {
                draw,
                palette: None,
                palette_order_hash: 0,
                bone_offset: 0,
            })
            .chain(skinned_draws.iter().map(|skinned| PreparedDraw {
                draw: skinned.draw,
                palette: Some(Arc::clone(&skinned.joint_matrices)),
                palette_order_hash: palette_order_hash(&skinned.joint_matrices),
                bone_offset: 0,
            }))
            .filter(|prepared| {
                let draw = prepared.draw;
                self.meshes.get(draw.mesh.slot).is_some_and(|slot| {
                    slot.generation == draw.mesh.generation && slot.mesh.is_some()
                }) && self.materials.get(draw.material.slot).is_some_and(|slot| {
                    slot.generation == draw.material.generation && slot.material.is_some()
                })
            })
            .collect();
        draws.sort_by_key(|prepared| (prepared.draw.mesh, prepared.draw.material));

        let mut skin_matrices = Vec::<[[f32; 4]; 4]>::new();
        let mut rejected_skin_palettes = 0_usize;
        draws.retain_mut(|prepared| {
            let Some(palette) = prepared.palette.as_ref() else {
                return true;
            };
            let Some(offset) = append_joint_palette(palette, &mut skin_matrices) else {
                // Reject only the over-capacity skinned draw. Static and earlier accepted draws
                // continue to render; no invalid offset reaches the vertex shader.
                rejected_skin_palettes = rejected_skin_palettes.saturating_add(1);
                return false;
            };
            prepared.bone_offset = offset;
            true
        });
        let shadow_draws: Vec<PreparedDraw> = draws
            .iter()
            .filter(|prepared| !is_blended(self, prepared))
            .cloned()
            .collect();
        self.last_skinned_draws_after_palette_budget =
            draws.iter().filter(|draw| draw.palette.is_some()).count();
        self.last_skinned_draws_rejected_palette_capacity = rejected_skin_palettes;
        self.last_skin_palette_matrices_staged = skin_matrices.len();
        let depth_layer_debug =
            self.directional_shadow_debug.mode == super::DirectionalShadowDebugMode::DepthLayer;
        if draws.is_empty() && scene.particles.is_empty() && !depth_layer_debug {
            self.maintain_instance_buffers(device, 0);
            return cpu_phases;
        }
        self.maintain_instance_buffers(device, draws.len());
        if !skin_matrices.is_empty() {
            queue.write_buffer(&self.skin_matrices, 0, bytemuck::cast_slice(&skin_matrices));
        }
        let planes = frustum_planes(scene.view_proj);
        let visible_draws: Vec<PreparedDraw> = draws
            .iter()
            .filter(|prepared| {
                // Bind-pose mesh bounds are not conservative for an animated skeleton: a joint
                // can move vertices outside that sphere. Keep skinned placements in the main
                // pass and let clip-space clipping decide visibility until animated bounds exist.
                if prepared.palette.is_some() {
                    return true;
                }
                let draw = prepared.draw;
                let Some(mesh) = self.meshes[draw.mesh.slot].mesh.as_ref() else {
                    return false;
                };
                let center = draw.model.transform_point3(mesh.bounds_center);
                let x = draw.model.x_axis.truncate();
                let y = draw.model.y_axis.truncate();
                let z = draw.model.z_axis.truncate();
                let max_scale =
                    (x.length_squared() + y.length_squared() + z.length_squared()).sqrt();
                sphere_visible(&planes, center, mesh.bounds_radius * max_scale)
            })
            .cloned()
            .collect();
        let mut visible_draws = visible_draws;
        sort_visible_draws(self, &mut visible_draws, scene);
        self.last_visible_mesh_instances = visible_draws.len();
        self.last_visible_draw_calls = visible_draws
            .chunk_by(|left, right| {
                (left.draw.mesh, left.draw.material) == (right.draw.mesh, right.draw.material)
            })
            .count();
        let shadow_draw_calls_per_pass = shadow_draws
            .chunk_by(|left, right| {
                (left.draw.mesh, left.draw.material) == (right.draw.mesh, right.draw.material)
            })
            .count();
        record_cpu_phase(
            &mut cpu_phases,
            "draw validation, sorting, and visibility",
            phase_started.take(),
        );
        phase_started = profile_cpu.then(Instant::now);
        let to_raw = |instances: &[PreparedDraw]| {
            instances
                .iter()
                .map(|prepared| {
                    let draw = prepared.draw;
                    RawInstance {
                        model: draw.model.to_cols_array_2d(),
                        tint: [draw.tint[0], draw.tint[1], draw.tint[2], 1.0],
                        bone_offset: prepared.bone_offset,
                        _padding: [0; 3],
                    }
                })
                .collect::<Vec<_>>()
        };
        let shadow_raw = to_raw(&shadow_draws);
        let visible_raw = to_raw(&visible_draws);
        record_cpu_phase(
            &mut cpu_phases,
            "instance data preparation",
            phase_started.take(),
        );
        phase_started = profile_cpu.then(Instant::now);
        if !shadow_raw.is_empty() {
            queue.write_buffer(&self.shadow_instances, 0, bytemuck::cast_slice(&shadow_raw));
        }
        if !visible_raw.is_empty() {
            queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(&visible_raw));
        }
        record_cpu_phase(
            &mut cpu_phases,
            "instance buffer updates",
            phase_started.take(),
        );
        phase_started = profile_cpu.then(Instant::now);
        // Build world-space caster bounds from the exact accepted opaque/masked shadow draw set.
        // This list already excludes rejected skin palettes and blended materials, matching the
        // draw calls submitted below. The cascade builder projects these bounds once and reuses
        // them for all four cascades.
        self.directional_shadow_caster_bounds_scratch.clear();
        for prepared in &shadow_draws {
            if let Some(world_bounds) = self.directional_shadow_caster_bounds(prepared) {
                self.directional_shadow_caster_bounds_scratch
                    .push(world_bounds);
            }
        }
        let directional_cascades = Self::directional_shadow_cascades_with_casters(
            scene,
            &self.directional_shadow_caster_bounds_scratch,
            self.directional_shadow_caster_extrusion_distance,
        );
        record_cpu_phase(
            &mut cpu_phases,
            "directional caster bounds and cascade fitting",
            phase_started.take(),
        );
        phase_started = profile_cpu.then(Instant::now);
        let (active_spot, shadow_spot) =
            select_spot_lights(scene.spot_lights, scene.camera_position);
        let shadow_pass_count = if shadow_spot.is_some() {
            super::shadows::DIRECTIONAL_CASCADE_COUNT + 1
        } else {
            super::shadows::DIRECTIONAL_CASCADE_COUNT
        };
        self.last_shadow_draw_calls = shadow_draw_calls_per_pass.saturating_mul(shadow_pass_count);
        let spot_view_proj = shadow_spot
            .map(Self::spot_shadow_matrix)
            .unwrap_or(Mat4::IDENTITY);
        let selected_point_lights = select_point_lights(scene.point_lights, scene.camera_position);
        let (point_positions_range, point_colors_intensity) =
            pack_point_lights(&selected_point_lights);
        let spot = active_spot.unwrap_or_default();
        let globals = Globals {
            view_proj: scene.view_proj.to_cols_array_2d(),
            light_view_proj: directional_cascades
                .matrices
                .map(|matrix| matrix.to_cols_array_2d()),
            cascade_splits: directional_cascades.split_depths,
            cascade_params: [
                directional_cascades.near_depth,
                super::shadows::DIRECTIONAL_CASCADE_BLEND,
                self.directional_shadow_debug.mode as u32 as f32,
                self.directional_shadow_debug.depth_layer as f32,
            ],
            camera_forward: directional_cascades.camera_forward.extend(0.0).to_array(),
            spot_view_proj: spot_view_proj.to_cols_array_2d(),
            sun_dir: scene
                .sun
                .direction
                .normalize_or_zero()
                .extend(0.0)
                .to_array(),
            sun_color: scene.sun.color.max(Vec3::ZERO).extend(1.0).to_array(),
            ambient: scene.sun.ambient.max(Vec3::ZERO).extend(1.0).to_array(),
            camera_pos: scene.camera_position.extend(1.0).to_array(),
            spot_position_range: [
                spot.position.x,
                spot.position.y,
                spot.position.z,
                spot.range.max(0.0),
            ],
            spot_direction_outer: [
                spot.direction.normalize_or_zero().x,
                spot.direction.normalize_or_zero().y,
                spot.direction.normalize_or_zero().z,
                spot.outer_cone_angle.clamp(0.01, 1.55).cos(),
            ],
            spot_color_intensity: [
                spot.color.x,
                spot.color.y,
                spot.color.z,
                spot.intensity.max(0.0),
            ],
            spot_params: [
                spot.inner_cone_angle
                    .clamp(0.0, spot.outer_cone_angle)
                    .cos(),
                if shadow_spot.is_some() { 1.0 } else { 0.0 },
                if active_spot.is_some() { 1.0 } else { 0.0 },
                0.0,
            ],
            point_positions_range,
            point_colors_intensity,
            point_count: [selected_point_lights.len() as f32, 0.0, 0.0, 0.0],
            fog_color_density: [
                scene.fog.color.x,
                scene.fog.color.y,
                scene.fog.color.z,
                scene.fog.density.max(0.0),
            ],
            fog_params: [
                scene.fog.base_height,
                scene.fog.height_falloff.max(0.0),
                0.0,
                0.0,
            ],
        };
        queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&globals));
        let depth_debug_uniform = super::ShadowDepthDebugUniform {
            selected_layer: [self.directional_shadow_debug.depth_layer, 0, 0, 0],
            target_size: [size.0, size.1, 0, 0],
        };
        queue.write_buffer(
            &self.shadow_depth_debug_params,
            0,
            bytemuck::bytes_of(&depth_debug_uniform),
        );
        for (buffer, matrix) in self
            .directional_shadow_globals
            .iter()
            .zip(directional_cascades.matrices)
        {
            let shadow_globals = ShadowGlobals {
                light_view_proj: matrix.to_cols_array_2d(),
            };
            queue.write_buffer(buffer, 0, bytemuck::bytes_of(&shadow_globals));
        }
        if let Some(light) = shadow_spot {
            let spot_globals = ShadowGlobals {
                light_view_proj: Self::spot_shadow_matrix(light).to_cols_array_2d(),
            };
            queue.write_buffer(
                &self.spot_shadow_globals,
                0,
                bytemuck::bytes_of(&spot_globals),
            );
        }
        record_cpu_phase(
            &mut cpu_phases,
            "uniform preparation and updates",
            phase_started.take(),
        );
        let Some(depth) = self.depth.as_ref() else {
            return cpu_phases;
        };
        let timing_slot = self
            .gpu_timing
            .as_mut()
            .and_then(|profiler| profiler.reserve_slot(submitted_frame));
        let timestamp_query_set = if timing_slot.is_some() {
            self.gpu_timing
                .as_ref()
                .map(GpuTimestampProfiler::query_set)
        } else {
            None
        };
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("mesh and directional shadow passes"),
        });
        phase_started = profile_cpu.then(Instant::now);
        for (cascade_index, (layer_view, shadow_globals_group)) in self
            .shadow_target
            .layer_views
            .iter()
            .zip(self.directional_shadow_globals_groups.iter())
            .enumerate()
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("directional shadow cascade"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: layer_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: timestamp_query_set.and_then(|query_set| {
                    let is_first_cascade = cascade_index == 0;
                    let is_last_cascade_without_spot = cascade_index + 1
                        == super::shadows::DIRECTIONAL_CASCADE_COUNT
                        && shadow_spot.is_none();
                    (is_first_cascade || is_last_cascade_without_spot).then_some(
                        wgpu::RenderPassTimestampWrites {
                            query_set,
                            beginning_of_pass_write_index: is_first_cascade.then_some(0),
                            end_of_pass_write_index: is_last_cascade_without_spot.then_some(1),
                        },
                    )
                }),
                occlusion_query_set: None,
            });
            pass.set_bind_group(0, shadow_globals_group, &[]);
            pass.set_vertex_buffer(1, self.shadow_instances.slice(..));
            let mut start = 0;
            while start < shadow_draws.len() {
                let key = (
                    shadow_draws[start].draw.mesh,
                    shadow_draws[start].draw.material,
                );
                let end = start
                    + shadow_draws[start..]
                        .iter()
                        .take_while(|draw| (draw.draw.mesh, draw.draw.material) == key)
                        .count();
                let mesh = self.meshes[key.0.slot]
                    .mesh
                    .as_ref()
                    .expect("draw list was validated before recording the shadow pass");
                let material = self.materials[key.1.slot]
                    .material
                    .as_ref()
                    .expect("shadow draw material was validated before recording the pass");
                let shadow_pipeline = match (mesh.vertex_layout, material.double_sided) {
                    (MeshVertexLayout::Static, false) => &self.static_shadow_pipeline,
                    (MeshVertexLayout::Static, true) => &self.static_shadow_double_sided_pipeline,
                    (MeshVertexLayout::Skinned, false) => &self.shadow_pipeline,
                    (MeshVertexLayout::Skinned, true) => &self.shadow_double_sided_pipeline,
                };
                pass.set_pipeline(shadow_pipeline);
                pass.set_bind_group(1, &material.bind_group, &[]);
                pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..mesh.index_count, 0, start as u32..end as u32);
                start = end;
            }
        }
        if shadow_spot.is_some() {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("spotlight perspective shadow map"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.spot_shadow_target.view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: timestamp_query_set.and_then(|query_set| {
                    shadow_spot.map(|_| wgpu::RenderPassTimestampWrites {
                        query_set,
                        beginning_of_pass_write_index: None,
                        end_of_pass_write_index: Some(1),
                    })
                }),
                occlusion_query_set: None,
            });
            pass.set_bind_group(0, &self.spot_shadow_globals_group, &[]);
            pass.set_vertex_buffer(1, self.shadow_instances.slice(..));
            let mut start = 0;
            while start < shadow_draws.len() {
                let key = (
                    shadow_draws[start].draw.mesh,
                    shadow_draws[start].draw.material,
                );
                let end = start
                    + shadow_draws[start..]
                        .iter()
                        .take_while(|draw| (draw.draw.mesh, draw.draw.material) == key)
                        .count();
                let mesh = self.meshes[key.0.slot]
                    .mesh
                    .as_ref()
                    .expect("draw list was validated before recording spotlight shadows");
                let material = self.materials[key.1.slot]
                    .material
                    .as_ref()
                    .expect("spotlight shadow material was validated before recording the pass");
                let spot_pipeline = match mesh.vertex_layout {
                    MeshVertexLayout::Static => &self.static_spot_shadow_pipeline,
                    MeshVertexLayout::Skinned => &self.spot_shadow_pipeline,
                };
                pass.set_pipeline(spot_pipeline);
                pass.set_bind_group(1, &material.bind_group, &[]);
                pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..mesh.index_count, 0, start as u32..end as u32);
                start = end;
            }
        }
        record_cpu_phase(
            &mut cpu_phases,
            "shadow pass command encoding",
            phase_started.take(),
        );
        phase_started = profile_cpu.then(Instant::now);
        if depth_layer_debug {
            // Keep the main depth attachment deterministic for downstream GTAO/post-processing,
            // even though the depth-layer visualization itself is a fullscreen pass.
            let _depth_clear_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("clear main depth before directional shadow depth preview"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth.view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(if depth_layer_debug {
                    "directional shadow depth-layer debug preview"
                } else {
                    "meshes and directional shadow diagnostics"
                }),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: resolve_view,
                    ops: wgpu::Operations {
                        load: if depth_layer_debug {
                            wgpu::LoadOp::Clear(wgpu::Color::BLACK)
                        } else {
                            wgpu::LoadOp::Load
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: if depth_layer_debug {
                    None
                } else {
                    Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &depth.view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    })
                },
                timestamp_writes: timestamp_query_set.map(|query_set| {
                    wgpu::RenderPassTimestampWrites {
                        query_set,
                        beginning_of_pass_write_index: Some(2),
                        end_of_pass_write_index: Some(3),
                    }
                }),
                occlusion_query_set: None,
            });
            if depth_layer_debug {
                // Inspect the full selected depth-array layer after this frame's shadow passes.
                pass.set_pipeline(&self.shadow_depth_debug_pipeline);
                pass.set_bind_group(0, &self.shadow_depth_debug_group, &[]);
                pass.draw(0..3, 0..1);
            } else {
                pass.set_bind_group(0, &self.globals_group, &[]);
                pass.set_vertex_buffer(1, self.instances.slice(..));
                let mut start = 0;
                while start < visible_draws.len() {
                    let key = (
                        visible_draws[start].draw.mesh,
                        visible_draws[start].draw.material,
                    );
                    let end = start
                        + visible_draws[start..]
                            .iter()
                            .take_while(|d| (d.draw.mesh, d.draw.material) == key)
                            .count();
                    let mesh = self.meshes[key.0.slot]
                        .mesh
                        .as_ref()
                        .expect("draw list was validated before recording the mesh pass");
                    let material = self.materials[key.1.slot]
                        .material
                        .as_ref()
                        .expect("draw list was validated before recording the material pass");
                    let pipeline = match mesh.vertex_layout {
                        MeshVertexLayout::Static => {
                            match color_pipeline_key(material.alpha_mode, material.double_sided) {
                                ColorPipelineKey::OpaqueCull => &self.static_pipeline,
                                ColorPipelineKey::OpaqueDoubleSided => {
                                    &self.static_double_sided_pipeline
                                }
                                ColorPipelineKey::BlendCull => &self.static_blend_pipeline,
                                ColorPipelineKey::BlendDoubleSided => {
                                    &self.static_blend_double_sided_pipeline
                                }
                            }
                        }
                        MeshVertexLayout::Skinned => {
                            match color_pipeline_key(material.alpha_mode, material.double_sided) {
                                ColorPipelineKey::OpaqueCull => &self.pipeline,
                                ColorPipelineKey::OpaqueDoubleSided => &self.double_sided_pipeline,
                                ColorPipelineKey::BlendCull => &self.blend_pipeline,
                                ColorPipelineKey::BlendDoubleSided => {
                                    &self.blend_double_sided_pipeline
                                }
                            }
                        }
                    };
                    pass.set_pipeline(pipeline);
                    pass.set_bind_group(1, &material.bind_group, &[]);
                    pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                    pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..mesh.index_count, 0, start as u32..end as u32);
                    start = end;
                }
                // Diagnostic modes deliberately suppress particles so cascade colors and blend
                // weights remain attributable to mesh fragments only.
                if self.directional_shadow_debug.mode == super::DirectionalShadowDebugMode::Disabled
                {
                    self.particles.draw(&mut pass, &self.globals_group);
                }
            }
        }
        record_cpu_phase(
            &mut cpu_phases,
            "mesh pass command encoding",
            phase_started.take(),
        );
        phase_started = profile_cpu.then(Instant::now);
        if let (Some(slot), Some(profiler)) = (timing_slot, self.gpu_timing.as_ref()) {
            profiler.encode_resolve(&mut encoder, slot);
        }
        queue.submit(Some(encoder.finish()));
        record_cpu_phase(
            &mut cpu_phases,
            "mesh queue submission",
            phase_started.take(),
        );
        if let (Some(slot), Some(profiler)) = (timing_slot, self.gpu_timing.as_mut()) {
            profiler.submitted(slot);
        }
        self.update_peak_resource_estimate();
        cpu_phases
    }

    /// Grow instance buffers immediately when demand increases, but shrink them only after 120
    /// consecutive frames at one-quarter capacity or less. This prevents large-scene captures from
    /// retaining their high-water allocation forever without causing allocation churn during brief
    /// camera/frustum or entity-count changes. Call this even while the scene has no drawable items.
    pub fn maintain_instance_buffers(&mut self, device: &wgpu::Device, required_instances: usize) {
        if required_instances > self.instance_capacity {
            let desired = required_instances
                .checked_next_power_of_two()
                .unwrap_or(required_instances);
            self.instance_capacity = desired.max(MIN_INSTANCE_CAPACITY);
            self.instances = instance_buffer(device, self.instance_capacity);
            self.shadow_instances = instance_buffer(device, self.instance_capacity);
            self.instance_buffer_grows_cumulative =
                self.instance_buffer_grows_cumulative.saturating_add(1);
            self.underused_instance_frames = 0;
            self.update_peak_resource_estimate();
            return;
        }

        if self.instance_capacity > INSTANCE_BUFFER_SHRINK_THRESHOLD
            && required_instances <= self.instance_capacity / 4
        {
            self.underused_instance_frames = self.underused_instance_frames.saturating_add(1);
            if self.underused_instance_frames >= INSTANCE_BUFFER_SHRINK_AFTER_FRAMES {
                let requested_floor = required_instances.max(MIN_INSTANCE_CAPACITY);
                let desired = requested_floor
                    .checked_next_power_of_two()
                    .unwrap_or(requested_floor)
                    .max(MIN_INSTANCE_CAPACITY);
                if desired < self.instance_capacity {
                    self.instance_capacity = desired;
                    self.instances = instance_buffer(device, self.instance_capacity);
                    self.shadow_instances = instance_buffer(device, self.instance_capacity);
                    self.instance_buffer_shrinks_cumulative =
                        self.instance_buffer_shrinks_cumulative.saturating_add(1);
                    self.update_peak_resource_estimate();
                }
                self.underused_instance_frames = 0;
            }
        } else {
            self.underused_instance_frames = 0;
        }
    }
}

pub(super) fn texture_mip_bytes(width: u32, height: u32) -> u64 {
    let (mut width, mut height) = (width.max(1), height.max(1));
    let mut bytes = 0_u64;
    loop {
        bytes = bytes.saturating_add(
            u64::from(width)
                .saturating_mul(u64::from(height))
                .saturating_mul(4),
        );
        if width == 1 && height == 1 {
            break;
        }
        width = (width / 2).max(1);
        height = (height / 2).max(1);
    }
    bytes
}

#[cfg(test)]
mod alpha_pipeline_tests {
    use super::{color_pipeline_key, compare_visible_order, palette_order_hash, ColorPipelineKey};
    use crate::renderer::{MaterialId, MeshId};
    use glam::{Mat4, Vec3};
    use ribbon_plugin_gltf::AlphaMode;
    use std::cmp::Ordering;

    #[test]
    fn alpha_mode_and_double_sided_select_the_expected_pipeline_variant() {
        assert_eq!(
            color_pipeline_key(AlphaMode::Opaque, false),
            ColorPipelineKey::OpaqueCull
        );
        assert_eq!(
            color_pipeline_key(AlphaMode::Mask, false),
            ColorPipelineKey::OpaqueCull
        );
        assert_eq!(
            color_pipeline_key(AlphaMode::Opaque, true),
            ColorPipelineKey::OpaqueDoubleSided
        );
        assert_eq!(
            color_pipeline_key(AlphaMode::Mask, true),
            ColorPipelineKey::OpaqueDoubleSided
        );
        assert_eq!(
            color_pipeline_key(AlphaMode::Blend, false),
            ColorPipelineKey::BlendCull
        );
        assert_eq!(
            color_pipeline_key(AlphaMode::Blend, true),
            ColorPipelineKey::BlendDoubleSided
        );
    }

    #[test]
    fn skinned_palette_tie_breaker_is_repeatable_and_content_sensitive() {
        let first = [
            Mat4::IDENTITY,
            Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0)),
        ];
        let second = [
            Mat4::IDENTITY,
            Mat4::from_translation(Vec3::new(1.0, 2.0, 4.0)),
        ];
        assert_eq!(palette_order_hash(&first), palette_order_hash(&first));
        assert_ne!(palette_order_hash(&first), palette_order_hash(&second));
    }

    #[test]
    fn transparent_draws_are_sorted_back_to_front_after_opaque_and_masked() {
        let key_a = (
            MeshId {
                slot: 1,
                generation: 0,
            },
            MaterialId {
                slot: 1,
                generation: 0,
            },
            [0; 16],
            [0; 3],
            0,
        );
        let key_b = (
            MeshId {
                slot: 2,
                generation: 0,
            },
            MaterialId {
                slot: 2,
                generation: 0,
            },
            [1; 16],
            [1; 3],
            0,
        );
        assert_eq!(
            compare_visible_order(false, true, 2.0, 100.0, key_a, key_b),
            Ordering::Less
        );
        assert_eq!(
            compare_visible_order(true, false, 100.0, 2.0, key_b, key_a),
            Ordering::Greater
        );
        assert_eq!(
            compare_visible_order(true, true, 2.0, 10.0, key_a, key_b),
            Ordering::Greater
        );
        assert_eq!(
            compare_visible_order(true, true, 10.0, 2.0, key_a, key_b),
            Ordering::Less
        );
        assert_eq!(
            compare_visible_order(true, true, 5.0, 5.0, key_a, key_b),
            Ordering::Less
        );
        assert_eq!(
            compare_visible_order(true, true, 5.0, 5.0, key_b, key_a),
            Ordering::Greater
        );
        let same_mesh_material_a = (
            MeshId {
                slot: 1,
                generation: 0,
            },
            MaterialId {
                slot: 1,
                generation: 0,
            },
            [0; 16],
            [0; 3],
            0,
        );
        let same_mesh_material_b = (
            MeshId {
                slot: 1,
                generation: 0,
            },
            MaterialId {
                slot: 1,
                generation: 0,
            },
            [1; 16],
            [0; 3],
            0,
        );
        assert_eq!(
            compare_visible_order(
                true,
                true,
                5.0,
                5.0,
                same_mesh_material_a,
                same_mesh_material_b,
            ),
            Ordering::Less
        );
    }
}
