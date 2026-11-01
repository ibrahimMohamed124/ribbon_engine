use super::*;

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
                size,
                scene,
            },
            skinned_draws,
            None,
        );
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
            size,
            scene,
        } = target;
        let profile_cpu = profile_frame.is_some();
        let submitted_frame = profile_frame.unwrap_or_default();
        let mut cpu_phases = Vec::new();
        self.last_visible_mesh_instances = 0;
        self.last_visible_draw_calls = 0;
        self.last_shadow_draw_calls = 0;
        self.last_skinned_draws_after_palette_budget = 0;
        self.last_skinned_draws_rejected_palette_capacity = 0;
        self.last_skin_palette_matrices_staged = 0;
        if size.0 == 0 || size.1 == 0 {
            return cpu_phases;
        }
        let mut phase_started = profile_cpu.then(Instant::now);
        if scene.draws.is_empty() && skinned_draws.is_empty() {
            self.maintain_instance_buffers(device, 0);
            return cpu_phases;
        }
        self.ensure_depth(device, size);
        // Static and skinned draws share the same generation-checked mesh/material path. The
        // skinned palette is retained by Arc through extraction and copied to the GPU once here.
        let mut draws: Vec<PreparedDraw> = scene
            .draws
            .iter()
            .copied()
            .map(|draw| PreparedDraw {
                draw,
                palette: None,
                bone_offset: 0,
            })
            .chain(skinned_draws.iter().map(|skinned| PreparedDraw {
                draw: skinned.draw,
                palette: Some(Arc::clone(&skinned.joint_matrices)),
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
        self.last_skinned_draws_after_palette_budget =
            draws.iter().filter(|draw| draw.palette.is_some()).count();
        self.last_skinned_draws_rejected_palette_capacity = rejected_skin_palettes;
        self.last_skin_palette_matrices_staged = skin_matrices.len();
        if draws.is_empty() {
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
        if visible_draws.is_empty() {
            return cpu_phases;
        }
        self.last_visible_mesh_instances = visible_draws.len();
        self.last_visible_draw_calls = visible_draws
            .chunk_by(|left, right| {
                (left.draw.mesh, left.draw.material) == (right.draw.mesh, right.draw.material)
            })
            .count();
        self.last_shadow_draw_calls = draws
            .chunk_by(|left, right| left.draw.mesh == right.draw.mesh)
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
        let shadow_raw = to_raw(&draws);
        let visible_raw = to_raw(&visible_draws);
        record_cpu_phase(
            &mut cpu_phases,
            "instance data preparation",
            phase_started.take(),
        );
        phase_started = profile_cpu.then(Instant::now);
        queue.write_buffer(&self.shadow_instances, 0, bytemuck::cast_slice(&shadow_raw));
        queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(&visible_raw));
        record_cpu_phase(
            &mut cpu_phases,
            "instance buffer updates",
            phase_started.take(),
        );
        phase_started = profile_cpu.then(Instant::now);
        let light_view_proj = Self::directional_shadow_matrix(scene);
        let (active_spot, shadow_spot) = select_spot_lights(scene.spot_lights);
        let spot_view_proj = shadow_spot
            .map(Self::spot_shadow_matrix)
            .unwrap_or(Mat4::IDENTITY);
        let (point_positions_range, point_colors_intensity) = pack_point_lights(scene.point_lights);
        let spot = active_spot.unwrap_or_default();
        let globals = Globals {
            view_proj: scene.view_proj.to_cols_array_2d(),
            light_view_proj: light_view_proj.to_cols_array_2d(),
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
            point_count: [scene.point_lights.len().min(8) as f32, 0.0, 0.0, 0.0],
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
        let shadow_globals = ShadowGlobals {
            light_view_proj: light_view_proj.to_cols_array_2d(),
        };
        queue.write_buffer(&self.shadow_globals, 0, bytemuck::bytes_of(&shadow_globals));
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
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("directional shadow map"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.shadow_target.view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: timestamp_query_set.map(|query_set| {
                    wgpu::RenderPassTimestampWrites {
                        query_set,
                        beginning_of_pass_write_index: Some(0),
                        end_of_pass_write_index: shadow_spot.is_none().then_some(1),
                    }
                }),
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.shadow_pipeline);
            pass.set_bind_group(0, &self.shadow_globals_group, &[]);
            pass.set_vertex_buffer(1, self.shadow_instances.slice(..));
            let mut start = 0;
            while start < draws.len() {
                let mesh_id = draws[start].draw.mesh;
                let end = start
                    + draws[start..]
                        .iter()
                        .take_while(|draw| draw.draw.mesh == mesh_id)
                        .count();
                let mesh = self.meshes[mesh_id.slot]
                    .mesh
                    .as_ref()
                    .expect("draw list was validated before recording the shadow pass");
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
            pass.set_pipeline(&self.spot_shadow_pipeline);
            pass.set_bind_group(0, &self.spot_shadow_globals_group, &[]);
            pass.set_vertex_buffer(1, self.shadow_instances.slice(..));
            let mut start = 0;
            while start < draws.len() {
                let mesh_id = draws[start].draw.mesh;
                let end = start
                    + draws[start..]
                        .iter()
                        .take_while(|draw| draw.draw.mesh == mesh_id)
                        .count();
                let mesh = self.meshes[mesh_id.slot]
                    .mesh
                    .as_ref()
                    .expect("draw list was validated before recording spotlight shadows");
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
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("meshes"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth.view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: timestamp_query_set.map(|query_set| {
                    wgpu::RenderPassTimestampWrites {
                        query_set,
                        beginning_of_pass_write_index: Some(2),
                        end_of_pass_write_index: Some(3),
                    }
                }),
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipeline);
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
                pass.set_bind_group(1, &material.bind_group, &[]);
                pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..mesh.index_count, 0, start as u32..end as u32);
                start = end;
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
