//! GPU presentation and rendering backend.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use engine_core::gfx::{DrawCommand, DrawList, Effect, Framebuffer};
use engine_scene::app::FramePlan;
use winit::window::Window;

const PRESENT_SHADER: &str = include_str!("../shaders/present.wgsl");
const QUAD_SHADER: &str = include_str!("../shaders/ui.wgsl");

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ScreenUniform {
    size: [f32; 2],
    mode: u32,
    frame: u32,
    time: f32,
    amount: f32,
    pad: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct QuadInstance {
    rect: [f32; 4],
    color: [f32; 4],
}

struct RenderTarget {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

/// GPU overlay and presentation pipeline; world pixels are uploaded by the CPU reference renderer.
pub struct GpuPresenter {
    _instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface_config: wgpu::SurfaceConfiguration,
    present_pipeline: wgpu::RenderPipeline,
    internal_pipeline: wgpu::RenderPipeline,
    quad_pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    quad_bind_group: wgpu::BindGroup,
    screen_buffer: wgpu::Buffer,
    targets: [RenderTarget; 2],
    sample_bind_groups: [wgpu::BindGroup; 2],
    quad_buffer: wgpu::Buffer,
    quads: Vec<QuadInstance>,
    uniform_cursor: u32,
    quad_cursor: u64,
    internal_size: (u32, u32),
    upload_bytes: Vec<u8>,
}

impl GpuPresenter {
    pub fn new(window: Arc<Window>, internal_size: (u32, u32)) -> Result<Self, String> {
        pollster::block_on(Self::new_async(window, internal_size))
    }

    async fn new_async(window: Arc<Window>, internal_size: (u32, u32)) -> Result<Self, String> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let surface = instance
            .create_surface(window.clone())
            .map_err(|error| format!("could not create wgpu surface: {error}"))?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .map_err(|error| format!("no compatible wgpu adapter: {error}"))?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("STUDS GPU presenter device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
            })
            .await
            .map_err(|error| format!("could not create wgpu device: {error}"))?;

        let size = window.inner_size();
        let capabilities = surface.get_capabilities(&adapter);
        let format = capabilities
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .or_else(|| capabilities.formats.first().copied())
            .ok_or_else(|| "wgpu surface exposes no color formats".to_owned())?;
        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: capabilities
                .alpha_modes
                .first()
                .copied()
                .ok_or_else(|| "wgpu surface exposes no alpha modes".to_owned())?,
            view_formats: vec![],
        };
        surface.configure(&device, &surface_config);

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("nearest upscale bindings"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(
                            std::mem::size_of::<ScreenUniform>() as u64,
                        ),
                    },
                    count: None,
                },
            ],
        });
        let screen_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("persistent per-pass uniform arena"),
            size: 256 * 128,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("nearest upscale shader"),
            source: wgpu::ShaderSource::Wgsl(PRESENT_SHADER.into()),
        });
        let quad_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("overlay quad shader"),
            source: wgpu::ShaderSource::Wgsl(QUAD_SHADER.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("present pipeline layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });
        let present_pipeline = create_pipeline(
            &device,
            &shader,
            &pipeline_layout,
            PipelineSpec {
                format,
                vertex_entry: "vs_main",
                fragment_entry: "fs_main",
                buffers: &[],
                label: "nearest upscale pipeline",
            },
        );
        let internal_pipeline = create_pipeline(
            &device,
            &shader,
            &pipeline_layout,
            PipelineSpec {
                format: wgpu::TextureFormat::Rgba8Unorm,
                vertex_entry: "vs_main",
                fragment_entry: "fs_internal",
                buffers: &[],
                label: "internal effect pipeline",
            },
        );
        let quad_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("quad uniform layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: wgpu::BufferSize::new(
                        std::mem::size_of::<ScreenUniform>() as u64
                    ),
                },
                count: None,
            }],
        });
        let quad_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("quad uniforms"),
            layout: &quad_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &screen_buffer,
                    offset: 0,
                    size: wgpu::BufferSize::new(std::mem::size_of::<ScreenUniform>() as u64),
                }),
            }],
        });
        const QUAD_ATTRIBUTES: [wgpu::VertexAttribute; 2] =
            wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4];
        let quad_vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<QuadInstance>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &QUAD_ATTRIBUTES,
        };
        let quad_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("overlay pipeline layout"),
            bind_group_layouts: &[&quad_layout],
            push_constant_ranges: &[],
        });
        let quad_pipeline = create_pipeline(
            &device,
            &quad_shader,
            &quad_pipeline_layout,
            PipelineSpec {
                format: wgpu::TextureFormat::Rgba8Unorm,
                vertex_entry: "vs_quad",
                fragment_entry: "fs_quad",
                buffers: &[quad_vertex_layout],
                label: "overlay quad pipeline",
            },
        );
        let targets = [
            create_render_target(&device, internal_size),
            create_render_target(&device, internal_size),
        ];
        let sample_bind_groups = [
            create_bind_group(
                &device,
                &bind_group_layout,
                &targets[0].view,
                &screen_buffer,
            ),
            create_bind_group(
                &device,
                &bind_group_layout,
                &targets[1].view,
                &screen_buffer,
            ),
        ];
        let quad_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("persistent overlay instances"),
            size: 65_536 * std::mem::size_of::<QuadInstance>() as u64 * 8,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Ok(Self {
            _instance: instance,
            surface,
            device,
            queue,
            surface_config,
            present_pipeline,
            internal_pipeline,
            quad_pipeline,
            bind_group_layout,
            quad_bind_group,
            screen_buffer,
            targets,
            sample_bind_groups,
            quad_buffer,
            quads: Vec::with_capacity(65_536),
            uniform_cursor: 0,
            quad_cursor: 0,
            internal_size,
            upload_bytes: vec![0; internal_size.0 as usize * internal_size.1 as usize * 4],
        })
    }

    pub fn resize_surface(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.surface_config.width = width;
        self.surface_config.height = height;
        self.surface.configure(&self.device, &self.surface_config);
    }

    pub fn present_frame(
        &mut self,
        framebuffer: &Framebuffer,
        plan: &FramePlan<'_>,
    ) -> Result<(), String> {
        if framebuffer.width == 0 || framebuffer.height == 0 {
            return Ok(());
        }
        let size = (framebuffer.width as u32, framebuffer.height as u32);
        if size != self.internal_size {
            self.resize_internal(size);
        }
        for (pixel, rgba) in framebuffer
            .pixels
            .iter()
            .zip(self.upload_bytes.as_chunks_mut::<4>().0)
        {
            rgba[0] = (pixel >> 16) as u8;
            rgba[1] = (pixel >> 8) as u8;
            rgba[2] = *pixel as u8;
            rgba[3] = 255;
        }
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.targets[0].texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &self.upload_bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size.0 * 4),
                rows_per_image: Some(size.1),
            },
            wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
        );
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("single frame encoder"),
            });
        self.uniform_cursor = 0;
        self.quad_cursor = 0;
        let mut current = 0usize;
        self.apply_draw_list(&mut encoder, &plan.world_layer, &mut current)?;
        self.fullscreen_pass(&mut encoder, 6, 0, 0.0, plan.brightness, &mut current);
        self.apply_draw_list(&mut encoder, &plan.ui_layer, &mut current)?;
        let present_uniform_offset = self.write_uniform(ScreenUniform {
            size: [
                self.surface_config.width as f32,
                self.surface_config.height as f32,
            ],
            mode: 0,
            frame: 0,
            time: 0.0,
            amount: 1.0,
            pad: [0.0; 2],
        });

        let frame = match self.surface.get_current_texture() {
            Ok(frame) => frame,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                self.surface.configure(&self.device, &self.surface_config);
                self.surface
                    .get_current_texture()
                    .map_err(|error| format!("surface recovery failed: {error}"))?
            }
            Err(error) => return Err(format!("surface frame unavailable: {error}")),
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("nearest upscale pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.present_pipeline);
            pass.set_bind_group(
                0,
                &self.sample_bind_groups[current],
                &[present_uniform_offset],
            );
            pass.draw(0..3, 0..1);
        }
        self.queue.submit(Some(encoder.finish()));
        frame.present();
        Ok(())
    }

    fn resize_internal(&mut self, size: (u32, u32)) {
        self.internal_size = size;
        self.targets = [
            create_render_target(&self.device, size),
            create_render_target(&self.device, size),
        ];
        self.sample_bind_groups = [
            create_bind_group(
                &self.device,
                &self.bind_group_layout,
                &self.targets[0].view,
                &self.screen_buffer,
            ),
            create_bind_group(
                &self.device,
                &self.bind_group_layout,
                &self.targets[1].view,
                &self.screen_buffer,
            ),
        ];
        self.upload_bytes
            .resize(size.0 as usize * size.1 as usize * 4, 0);
    }

    fn write_uniform(&mut self, uniform: ScreenUniform) -> u32 {
        let offset = u64::from(self.uniform_cursor) * 256;
        self.uniform_cursor = self.uniform_cursor.saturating_add(1).min(127);
        self.queue
            .write_buffer(&self.screen_buffer, offset, bytemuck::bytes_of(&uniform));
        offset as u32
    }

    fn apply_draw_list(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        draw_list: &DrawList,
        current: &mut usize,
    ) -> Result<(), String> {
        self.quads.clear();
        for command in &draw_list.commands {
            match command {
                DrawCommand::Rect { x, y, w, h, color } => self.push_quad(*x, *y, *w, *h, *color),
                DrawCommand::Text {
                    value,
                    x,
                    y,
                    scale,
                    color,
                } => {
                    let scale = (*scale).max(1);
                    let mut pen_x = *x;
                    for ch in value.chars() {
                        for (row, bits) in engine_core::gfx::font::glyph(ch.to_ascii_uppercase())
                            .iter()
                            .enumerate()
                        {
                            for col in 0..5 {
                                if bits & (1 << (4 - col)) != 0 {
                                    self.push_quad(
                                        pen_x + col * scale,
                                        *y + row as i32 * scale,
                                        scale,
                                        scale,
                                        *color,
                                    );
                                }
                            }
                        }
                        pen_x += 6 * scale;
                    }
                }
                DrawCommand::Dim(amount) => {
                    self.flush_quads(encoder, draw_list, *current)?;
                    self.fullscreen_pass(encoder, 1, 0, 0.0, *amount, current);
                }
                DrawCommand::Effect(effect) => {
                    self.flush_quads(encoder, draw_list, *current)?;
                    let (mode, frame, time) = match *effect {
                        Effect::FogBackground { frame } => (2, frame, 0.0),
                        Effect::CaughtNoise { time, frame } => (3, frame, time),
                        Effect::WonNoise { time, frame } => (4, frame, time),
                        Effect::SolidWhite => (5, 0, 0.0),
                    };
                    self.fullscreen_pass(encoder, mode, frame, time, 0.0, current);
                }
            }
        }
        self.flush_quads(encoder, draw_list, *current)
    }

    fn push_quad(&mut self, x: i32, y: i32, w: i32, h: i32, color: u32) {
        if w <= 0 || h <= 0 || self.quads.len() == 65_536 {
            return;
        }
        self.quads.push(QuadInstance {
            rect: [x as f32, y as f32, w as f32, h as f32],
            color: [
                ((color >> 16) & 0xff) as f32 / 255.0,
                ((color >> 8) & 0xff) as f32 / 255.0,
                (color & 0xff) as f32 / 255.0,
                1.0,
            ],
        });
    }

    fn flush_quads(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        draw_list: &DrawList,
        current: usize,
    ) -> Result<(), String> {
        if self.quads.is_empty() {
            return Ok(());
        }
        let uniform_offset = self.write_uniform(ScreenUniform {
            size: [draw_list.width as f32, draw_list.height as f32],
            mode: 0,
            frame: 0,
            time: 0.0,
            amount: 1.0,
            pad: [0.0; 2],
        });
        let quad_offset = self.quad_cursor;
        self.queue.write_buffer(
            &self.quad_buffer,
            quad_offset,
            bytemuck::cast_slice(&self.quads),
        );
        self.quad_cursor += std::mem::size_of_val(self.quads.as_slice()) as u64;
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ordered overlay quads"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.targets[current].view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&self.quad_pipeline);
        pass.set_bind_group(0, &self.quad_bind_group, &[uniform_offset]);
        pass.set_vertex_buffer(0, self.quad_buffer.slice(quad_offset..));
        pass.draw(0..6, 0..self.quads.len() as u32);
        self.quads.clear();
        Ok(())
    }

    fn fullscreen_pass(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        mode: u32,
        frame: u32,
        time: f32,
        amount: f32,
        current: &mut usize,
    ) {
        let uniform_offset = self.write_uniform(ScreenUniform {
            size: [self.internal_size.0 as f32, self.internal_size.1 as f32],
            mode,
            frame,
            time,
            amount,
            pad: [0.0; 2],
        });
        let next = 1 - *current;
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ordered full-screen effect"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.targets[next].view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&self.internal_pipeline);
        pass.set_bind_group(0, &self.sample_bind_groups[*current], &[uniform_offset]);
        pass.draw(0..3, 0..1);
        drop(pass);
        *current = next;
    }
}

fn create_render_target(device: &wgpu::Device, size: (u32, u32)) -> RenderTarget {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("persistent internal render target"),
        size: wgpu::Extent3d {
            width: size.0.max(1),
            height: size.1.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    RenderTarget { texture, view }
}

struct PipelineSpec<'a> {
    format: wgpu::TextureFormat,
    vertex_entry: &'a str,
    fragment_entry: &'a str,
    buffers: &'a [wgpu::VertexBufferLayout<'a>],
    label: &'a str,
}

fn create_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    spec: PipelineSpec<'_>,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(spec.label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(spec.vertex_entry),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: spec.buffers,
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(spec.fragment_entry),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: spec.format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
        cache: None,
    })
}

fn create_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    source: &wgpu::TextureView,
    screen: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("present bind group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(source),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: screen,
                    offset: 0,
                    size: wgpu::BufferSize::new(std::mem::size_of::<ScreenUniform>() as u64),
                }),
            },
        ],
    })
}
