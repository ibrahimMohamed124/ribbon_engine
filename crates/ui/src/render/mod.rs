//! GPU pass for immediate-mode UI geometry and the R8 glyph atlas.
use ribbon_render::gpu_window::{FramePass, FrameTarget};

struct PipelineBundle {
    format: wgpu::TextureFormat,
    pipeline: wgpu::RenderPipeline,
    atlas_layout: wgpu::BindGroupLayout,
}
struct GpuAtlas {
    width: u32,
    height: u32,
    _texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
}

#[derive(Default)]
pub struct UiPass {
    pipeline: Option<PipelineBundle>,
    atlas: Option<GpuAtlas>,
    buffer: Option<wgpu::Buffer>,
    capacity: u64,
    cpu_vertices: Vec<crate::UiVertex>,
}
impl UiPass {
    fn ensure_pipeline(&mut self, target: &FrameTarget<'_>) {
        if self
            .pipeline
            .as_ref()
            .is_some_and(|pipeline| pipeline.format == target.format)
        {
            return;
        }
        let shader = target
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("ribbon ui shader"),
                source: wgpu::ShaderSource::Wgsl(include_str!("ui.wgsl").into()),
            });
        let atlas_layout =
            target
                .device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("ribbon ui glyph atlas layout"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                            count: None,
                        },
                    ],
                });
        let layout = target
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("ribbon ui pipeline layout"),
                bind_group_layouts: &[&atlas_layout],
                push_constant_ranges: &[],
            });
        let pipe = target
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("ribbon ui"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<crate::UiVertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![
                            0 => Float32x2,
                            1 => Float32x2,
                            2 => Float32x4,
                            3 => Uint32,
                            4 => Float32x2,
                            5 => Float32x2,
                            6 => Float32,
                            7 => Float32
                        ],
                    }],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: target.format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview: None,
                cache: None,
            });
        self.pipeline = Some(PipelineBundle {
            format: target.format,
            pipeline: pipe,
            atlas_layout,
        });
        self.atlas = None;
    }

    fn ensure_atlas(&mut self, target: &FrameTarget<'_>, text: &mut crate::TextSystem) {
        let (width, height) = text.atlas_dimensions();
        if self
            .atlas
            .as_ref()
            .is_some_and(|atlas| atlas.width == width && atlas.height == height)
        {
            return;
        }
        let texture = target.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ribbon ui r8 glyph atlas"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = target.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ribbon ui glyph sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let Some(pipeline) = self.pipeline.as_ref() else {
            return;
        };
        let bind_group = target.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ribbon ui glyph atlas bind group"),
            layout: &pipeline.atlas_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let initial_pixels = text.atlas_snapshot();
        // The initial texture upload contains every current glyph. Discard the
        // atlas dirty queue now so the same full image is not uploaded again below.
        let _ = text.take_atlas_updates();
        target.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &initial_pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.atlas = Some(GpuAtlas {
            width,
            height,
            _texture: texture,
            bind_group,
        });
    }

    fn upload_atlas_updates(&mut self, target: &FrameTarget<'_>, text: &mut crate::TextSystem) {
        let (full, regions) = text.take_atlas_updates();
        let Some(atlas) = self.atlas.as_ref() else {
            return;
        };
        if let Some(pixels) = full {
            target.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &atlas._texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(atlas.width),
                    rows_per_image: Some(atlas.height),
                },
                wgpu::Extent3d {
                    width: atlas.width,
                    height: atlas.height,
                    depth_or_array_layers: 1,
                },
            );
            return;
        }
        for region in regions {
            target.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &atlas._texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: region.x,
                        y: region.y,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &region.pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(region.width),
                    rows_per_image: Some(region.height),
                },
                wgpu::Extent3d {
                    width: region.width,
                    height: region.height,
                    depth_or_array_layers: 1,
                },
            );
        }
    }

    fn draw_frame(
        &mut self,
        context: &crate::UiContext,
        text: &mut crate::TextSystem,
        target: &FrameTarget<'_>,
    ) {
        if target.size.0 == 0
            || target.size.1 == 0
            || context.screen.w <= 0.0
            || context.screen.h <= 0.0
        {
            return;
        }
        let physical_scale = target.size.0 as f32 / context.screen.w;
        let prepared_text = text.prepare(&context.draw.texts, physical_scale);
        if context.draw.vertices.is_empty() && prepared_text.vertices.is_empty() {
            return;
        }
        self.ensure_pipeline(target);
        self.ensure_atlas(target, text);
        self.upload_atlas_updates(target, text);

        self.cpu_vertices.clear();
        self.cpu_vertices
            .extend(context.draw.vertices.iter().map(|vertex| {
                let mut output = *vertex;
                output.position = [
                    (vertex.position[0] / context.screen.w) * 2.0 - 1.0,
                    1.0 - (vertex.position[1] / context.screen.h) * 2.0,
                ];
                output
            }));
        let text_vertex_offset = self.cpu_vertices.len() as u32;
        self.cpu_vertices
            .extend(prepared_text.vertices.iter().map(|vertex| {
                let mut output = *vertex;
                output.position = [
                    (vertex.position[0] / context.screen.w) * 2.0 - 1.0,
                    1.0 - (vertex.position[1] / context.screen.h) * 2.0,
                ];
                output
            }));

        let bytes = bytemuck::cast_slice(&self.cpu_vertices);
        let required = bytes.len() as u64;
        if required > self.capacity {
            self.capacity = required.next_power_of_two().max(256);
            self.buffer = Some(target.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("ribbon ui vertex buffer"),
                size: self.capacity,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        let Some(buffer) = self.buffer.as_ref() else {
            return;
        };
        target.queue.write_buffer(buffer, 0, bytes);
        let Some(pipeline) = self.pipeline.as_ref() else {
            return;
        };
        let Some(atlas) = self.atlas.as_ref() else {
            return;
        };

        let mut encoder = target
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("ribbon ui command encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ribbon ui overlay"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipeline.pipeline);
            pass.set_bind_group(0, &atlas.bind_group, &[]);
            pass.set_vertex_buffer(0, buffer.slice(..));

            let scale_x = target.size.0 as f32 / context.screen.w;
            let scale_y = target.size.1 as f32 / context.screen.h;
            // Keep immediate-mode z-order: text and shapes are interleaved in the
            // command stream, so later widgets correctly cover earlier commands.
            let mut ordered =
                Vec::with_capacity(context.draw.batches.len() + prepared_text.batches.len());
            ordered.extend(
                context
                    .draw
                    .batches
                    .iter()
                    .enumerate()
                    .map(|(index, batch)| (batch.order, false, index)),
            );
            ordered.extend(
                prepared_text
                    .batches
                    .iter()
                    .enumerate()
                    .map(|(index, batch)| (batch.order, true, index)),
            );
            ordered.sort_by_key(|entry| entry.0);
            for (_, is_text, index) in ordered {
                if is_text {
                    draw_batch(
                        &mut pass,
                        &prepared_text.batches[index],
                        text_vertex_offset,
                        scale_x,
                        scale_y,
                        target.size,
                    );
                } else {
                    draw_batch(
                        &mut pass,
                        &context.draw.batches[index],
                        0,
                        scale_x,
                        scale_y,
                        target.size,
                    );
                }
            }
        }
        target.queue.submit(Some(encoder.finish()));
    }
}

fn draw_batch(
    pass: &mut wgpu::RenderPass<'_>,
    batch: &crate::DrawBatch,
    offset: u32,
    scale_x: f32,
    scale_y: f32,
    target_size: (u32, u32),
) {
    if batch.vertex_range.is_empty() {
        return;
    }
    let clip = batch.clip.unwrap_or([0, 0, u32::MAX, u32::MAX]);
    let x = ((clip[0] as f32 * scale_x).floor() as u32).min(target_size.0);
    let y = ((clip[1] as f32 * scale_y).floor() as u32).min(target_size.1);
    let right =
        (((clip[0].saturating_add(clip[2])) as f32 * scale_x).ceil() as u32).min(target_size.0);
    let bottom =
        (((clip[1].saturating_add(clip[3])) as f32 * scale_y).ceil() as u32).min(target_size.1);
    let width = right.saturating_sub(x);
    let height = bottom.saturating_sub(y);
    if width == 0 || height == 0 {
        return;
    }
    pass.set_scissor_rect(x, y, width, height);
    pass.draw(
        batch.vertex_range.start + offset..batch.vertex_range.end + offset,
        0..1,
    );
}

impl FramePass for UiPass {
    fn name(&self) -> &'static str {
        "ribbon ui"
    }

    fn on_device_lost(&mut self, _: &mut ribbon_app::World) {
        self.pipeline = None;
        self.atlas = None;
        self.buffer = None;
        self.capacity = 0;
        self.cpu_vertices.clear();
    }

    fn render(&mut self, world: &mut ribbon_app::World, target: &FrameTarget<'_>) {
        if world.try_resource::<crate::UiContext>().is_none()
            || world.try_resource::<crate::TextSystem>().is_none()
        {
            return;
        }
        world.resource_scope::<crate::UiContext, _>(|world, context| {
            world.resource_scope::<crate::TextSystem, _>(|world, text| {
                self.draw_frame(context, text, target);
                if text.take_atlas_overflow_warning()
                    && world.try_resource::<ribbon_app::LogBuffer>().is_some()
                {
                    world.resource_mut::<ribbon_app::LogBuffer>().push(
                        ribbon_app::LogLevel::Warn,
                        "UI glyph atlas is full; glyphs that did not fit were omitted",
                    );
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_shader_is_valid_wgsl() {
        let module =
            naga::front::wgsl::parse_str(include_str!("ui.wgsl")).expect("parse UI shader");
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("validate UI shader");
    }

    #[test]
    #[ignore = "requires a working wgpu adapter; validates UI shapes and glyphs on GPU"]
    fn ui_pass_renders_a_panel_and_text_into_an_offscreen_target() {
        use ribbon_render::gpu::GpuContext;
        use ribbon_render::gpu_window::{clear_view, FramePass, FrameTarget};
        use std::future::Future;
        use std::sync::Arc;
        use std::task::{Context, Poll, Wake, Waker};

        struct ThreadWake(std::thread::Thread);
        impl Wake for ThreadWake {
            fn wake(self: Arc<Self>) {
                self.0.unpark();
            }
            fn wake_by_ref(self: &Arc<Self>) {
                self.0.unpark();
            }
        }
        fn block_on<F: Future>(future: F) -> F::Output {
            let waker = Waker::from(Arc::new(ThreadWake(std::thread::current())));
            let mut context = Context::from_waker(&waker);
            let mut future = Box::pin(future);
            loop {
                match future.as_mut().poll(&mut context) {
                    Poll::Ready(value) => return value,
                    Poll::Pending => std::thread::park(),
                }
            }
        }

        let gpu = block_on(GpuContext::new_offscreen(96, 64))
            .expect("UI GPU test requires an available adapter");
        clear_view(
            gpu.device(),
            gpu.queue(),
            gpu.target().view(),
            [0.0, 0.0, 0.0, 1.0],
        );

        let mut app = ribbon_app::App::new();
        let mut ui = crate::UiContext::synthetic(
            crate::Rect {
                x: 0.0,
                y: 0.0,
                w: 96.0,
                h: 64.0,
            },
            crate::UiInput::default(),
        );
        ui.begin();
        app.insert_resource(ui);
        app.insert_resource(crate::TextSystem::new());
        crate::ui_scope(&mut app.world, |ui| {
            ui.panel(
                "GPU",
                crate::Anchor::Center,
                crate::PanelSize::Fixed(88.0, 52.0),
                |ui| {
                    ui.heading("مرحبا");
                },
            );
        });

        let target = FrameTarget {
            adapter: gpu.adapter(),
            device: gpu.device(),
            queue: gpu.queue(),
            view: gpu.target().view(),
            format: wgpu::TextureFormat::Rgba8Unorm,
            size: (gpu.target().width, gpu.target().height),
        };
        UiPass::default().render(&mut app.world, &target);
        let pixels = gpu.readback().expect("UI offscreen readback").pixels;
        assert!(pixels.iter().any(|pixel| *pixel & 0x00ff_ffff != 0));
    }
}
