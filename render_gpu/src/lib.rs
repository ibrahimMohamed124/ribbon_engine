//! GPU presentation and rendering backend.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use engine_core::gfx::{DrawCommand, DrawList, Effect, Framebuffer};
use engine_core::math::{forward_vec, Vect};
use engine_scene::app::FramePlan;
use engine_scene::entities::PAGE;
use engine_scene::material::{BaseAlbedoRule, MaterialTable, TextureKind};
use engine_scene::scene::Scene;
use engine_scene::style::LEVEL_COUNT;
use engine_scene::texture::{SpriteAtlas, Texture};
use engine_scene::RENDER_DIST;
use winit::window::Window;

const PRESENT_SHADER: &str = include_str!("../shaders/present.wgsl");
const QUAD_SHADER: &str = include_str!("../shaders/ui.wgsl");
const WORLD_SHADER: &str = include_str!("../shaders/world.wgsl");
const PBR_WORD_CAPACITY: u64 = 4 * 4 * 87_381;

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

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct WorldUniform {
    dims: [u32; 4],
    eye: [f32; 4],
    forward: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
    projection: [f32; 4],
    lighting: [f32; 4],
    style: [f32; 4],
    frame: [u32; 4],
    sprite: [u32; 4],
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuMaterial {
    info: [u32; 4],
    params: [f32; 4],
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuEntity {
    pos_yaw: [f32; 4],
    params: [f32; 4],
    bounds_min: [f32; 4],
    bounds_max: [f32; 4],
    range: [u32; 4],
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuEntityBox {
    min_part: [f32; 4],
    max_sway: [f32; 4],
}

struct GpuWorldResources {
    uniform: wgpu::Buffer,
    voxels: wgpu::Buffer,
    materials: wgpu::Buffer,
    entities: wgpu::Buffer,
    boxes: wgpu::Buffer,
    tone: wgpu::Buffer,
    gamma: wgpu::Buffer,
    palette: wgpu::Buffer,
    pbr_texels: wgpu::Buffer,
    _brick_texture: wgpu::Texture,
    brick_view: wgpu::TextureView,
    _noise_texture: wgpu::Texture,
    noise_view: wgpu::TextureView,
    layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    world_key: Option<(usize, usize)>,
    packed: Vec<u32>,
    cached_blocks: Vec<u8>,
    pbr_key: Option<(usize, usize)>,
    sprite_key: Option<(usize, usize)>,
}

struct RenderTarget {
    _texture: wgpu::Texture,
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
    world_pipeline: wgpu::RenderPipeline,
    world: GpuWorldResources,
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
    frame_index: u32,
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
        let world_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("GPU world raycaster"),
            source: wgpu::ShaderSource::Wgsl(WORLD_SHADER.into()),
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
        let world = create_world_resources(&device, &queue);
        let world_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("world pipeline layout"),
            bind_group_layouts: &[&world.layout],
            push_constant_ranges: &[],
        });
        let world_pipeline = create_pipeline(
            &device,
            &world_shader,
            &world_layout,
            PipelineSpec {
                format: wgpu::TextureFormat::Rgba8Unorm,
                vertex_entry: "vs_main",
                fragment_entry: "fs_world",
                buffers: &[],
                label: "GPU world raycaster",
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
            world_pipeline,
            world,
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
            frame_index: 0,
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
        materials: &MaterialTable,
    ) -> Result<(), String> {
        if framebuffer.width == 0 || framebuffer.height == 0 {
            return Ok(());
        }
        let size = (framebuffer.width as u32, framebuffer.height as u32);
        if size != self.internal_size {
            self.resize_internal(size);
        }
        if let Some(scene) = plan.scene.as_ref() {
            update_world(
                &self.device,
                &self.queue,
                &mut self.world,
                scene,
                materials,
                self.internal_size,
            )?;
        }
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("single frame encoder"),
            });
        self.uniform_cursor = 0;
        self.quad_cursor = 0;
        let mut current = 0usize;
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("GPU world pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.targets[0].view,
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
            if plan.scene.is_some() {
                pass.set_pipeline(&self.world_pipeline);
                pass.set_bind_group(0, &self.world.bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
        }
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
        self.frame_index = self.frame_index.wrapping_add(1);
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

fn create_world_resources(device: &wgpu::Device, queue: &wgpu::Queue) -> GpuWorldResources {
    let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty,
        count: None,
    };
    let mut layout_entries = vec![entry(
        0,
        wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<WorldUniform>() as u64),
        },
    )];
    for binding in 1..=7 {
        layout_entries.push(entry(
            binding,
            wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
        ));
    }
    layout_entries.push(texture_entry(8));
    layout_entries.push(texture_entry(9));
    layout_entries.push(entry(
        10,
        wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
    ));
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("world bindings"),
        entries: &layout_entries,
    });
    let uniform = make_buffer(
        device,
        "world uniform",
        std::mem::size_of::<WorldUniform>() as u64,
        wgpu::BufferUsages::UNIFORM,
    );
    let voxels = make_buffer(device, "packed voxels", 4, wgpu::BufferUsages::STORAGE);
    let materials = make_buffer(device, "materials", 256 * 32, wgpu::BufferUsages::STORAGE);
    let entities = make_buffer(device, "entities", 16 * 80, wgpu::BufferUsages::STORAGE);
    let boxes = make_buffer(
        device,
        "entity boxes",
        1024 * 32,
        wgpu::BufferUsages::STORAGE,
    );
    let tone = make_buffer(device, "tone LUT", 1024 * 4, wgpu::BufferUsages::STORAGE);
    let gamma = make_buffer(device, "gamma LUT", 1024 * 4, wgpu::BufferUsages::STORAGE);
    let palette = make_buffer(
        device,
        "palette",
        LEVEL_COUNT as u64 * 4,
        wgpu::BufferUsages::STORAGE,
    );
    let pbr_texels = make_buffer(
        device,
        "PBR mip data and baked monster sprites",
        (PBR_WORD_CAPACITY + (768 * 3840 * 3) as u64) * 4,
        wgpu::BufferUsages::STORAGE,
    );
    let (bt, bv) = make_texture(device, queue, "bricks", &Texture::bricks(64, 32, 17));
    let (nt, nv) = make_texture(device, queue, "noise", &Texture::noise(64, 64, 41));
    let bind_group = world_bind(
        device,
        &layout,
        &uniform,
        &voxels,
        &materials,
        &entities,
        &boxes,
        &tone,
        &gamma,
        &palette,
        &bv,
        &nv,
        &pbr_texels,
    );
    GpuWorldResources {
        uniform,
        voxels,
        materials,
        entities,
        boxes,
        tone,
        gamma,
        palette,
        pbr_texels,
        _brick_texture: bt,
        brick_view: bv,
        _noise_texture: nt,
        noise_view: nv,
        layout,
        bind_group,
        world_key: None,
        packed: Vec::new(),
        cached_blocks: Vec::new(),
        pbr_key: None,
        sprite_key: None,
    }
}
fn make_buffer(
    device: &wgpu::Device,
    label: &str,
    size: u64,
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: size.max(4),
        usage: usage | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}
fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}
#[allow(clippy::too_many_arguments)]
fn world_bind(
    d: &wgpu::Device,
    l: &wgpu::BindGroupLayout,
    u: &wgpu::Buffer,
    v: &wgpu::Buffer,
    m: &wgpu::Buffer,
    e: &wgpu::Buffer,
    b: &wgpu::Buffer,
    t: &wgpu::Buffer,
    g: &wgpu::Buffer,
    p: &wgpu::Buffer,
    bt: &wgpu::TextureView,
    nt: &wgpu::TextureView,
    pbr: &wgpu::Buffer,
) -> wgpu::BindGroup {
    let bs = [u, v, m, e, b, t, g, p];
    let mut entries: Vec<_> = bs
        .iter()
        .enumerate()
        .map(|(i, x)| wgpu::BindGroupEntry {
            binding: i as u32,
            resource: x.as_entire_binding(),
        })
        .collect();
    entries.push(wgpu::BindGroupEntry {
        binding: 8,
        resource: wgpu::BindingResource::TextureView(bt),
    });
    entries.push(wgpu::BindGroupEntry {
        binding: 9,
        resource: wgpu::BindingResource::TextureView(nt),
    });
    entries.push(wgpu::BindGroupEntry {
        binding: 10,
        resource: pbr.as_entire_binding(),
    });
    d.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("world resource group"),
        layout: l,
        entries: &entries,
    })
}
fn make_texture(
    d: &wgpu::Device,
    q: &wgpu::Queue,
    label: &str,
    src: &Texture,
) -> (wgpu::Texture, wgpu::TextureView) {
    let t = d.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: src.w as u32,
            height: src.h as u32,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    q.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &t,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        bytemuck::cast_slice(&src.texels),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(src.w as u32 * 4),
            rows_per_image: Some(src.h as u32),
        },
        wgpu::Extent3d {
            width: src.w as u32,
            height: src.h as u32,
            depth_or_array_layers: 1,
        },
    );
    let v = t.create_view(&wgpu::TextureViewDescriptor::default());
    (t, v)
}
fn update_world(
    d: &wgpu::Device,
    q: &wgpu::Queue,
    g: &mut GpuWorldResources,
    s: &Scene<'_>,
    mt: &MaterialTable,
    size: (u32, u32),
) -> Result<(), String> {
    let dims = s.world.dimensions();
    let key = (s.world.blocks().as_ptr() as usize, s.world.blocks().len());
    let need = (s.world.blocks().len().div_ceil(4) * 4).max(4) as u64;
    let mut changed = false;
    if g.voxels.size() < need {
        g.voxels = make_buffer(d, "packed voxels", need, wgpu::BufferUsages::STORAGE);
        changed = true;
    }
    if let Some(atlas) = mt.sprite_atlas.filter(|atlas| atlas.is_valid()) {
        let required = (atlas.width * atlas.height * 3 * 4) as u64;
        if g.pbr_texels.size() < PBR_WORD_CAPACITY * 4 + required {
            g.pbr_texels = make_buffer(
                d,
                "PBR mip data and baked monster sprites",
                PBR_WORD_CAPACITY * 4 + required,
                wgpu::BufferUsages::STORAGE,
            );
            changed = true;
        }
    }
    if g.world_key != Some(key) || g.cached_blocks.as_slice() != s.world.blocks() {
        g.packed = pack_voxels(s.world.blocks());
        q.write_buffer(&g.voxels, 0, bytemuck::cast_slice(&g.packed));
        g.world_key = Some(key);
        g.cached_blocks.clear();
        g.cached_blocks.extend_from_slice(s.world.blocks());
    }
    let mut es = Vec::<GpuEntity>::new();
    let mut bs = Vec::<GpuEntityBox>::new();
    for ent in s.entities.iter().take(16) {
        let def = s
            .entity_registry
            .get(ent.kind)
            .ok_or_else(|| format!("unknown entity kind {}", ent.kind.0))?;
        let start = bs.len();
        if def.boxes.len() > 64 {
            return Err(format!(
                "entity kind {} exceeds the 64-box GPU entity limit",
                ent.kind.0
            ));
        }
        for bx in def.boxes {
            bs.push(GpuEntityBox {
                min_part: [bx.min[0], bx.min[1], bx.min[2], bx.part as f32],
                max_sway: [bx.max[0], bx.max[1], bx.max[2], bx.phase_sway],
            });
        }
        let (min, max, has) = def.bounds.map_or(([0.; 3], [0.; 3], 0), |(a, b)| (a, b, 1));
        let mut pos = [ent.pos.x, ent.pos.y, ent.pos.z];
        let mut yaw = ent.yaw;
        if ent.kind == PAGE {
            pos[2] += (ent.params[0] * 2. + ent.params[1]).sin() * 0.08;
            yaw = ent.params[0] * 1.2 + ent.params[1];
        }
        es.push(GpuEntity {
            pos_yaw: [pos[0], pos[1], pos[2], yaw],
            params: ent.params,
            bounds_min: [min[0], min[1], min[2], 0.],
            bounds_max: [max[0], max[1], max[2], 0.],
            range: [start as u32, def.boxes.len() as u32, has, ent.kind.0],
        });
    }
    if s.entities.len() > 16 || bs.len() > 1024 {
        return Err("GPU supports up to 16 entities and 1024 entity boxes".into());
    }
    if !es.is_empty() {
        q.write_buffer(&g.entities, 0, bytemuck::cast_slice(&es));
    }
    if !bs.is_empty() {
        q.write_buffer(&g.boxes, 0, bytemuck::cast_slice(&bs));
    }
    let mut mats = vec![GpuMaterial::zeroed(); 256];
    for m in mt.materials {
        let (tex, seed) = match m.texture {
            TextureKind::None => (0, 0),
            TextureKind::Bricks { seed } => (1, seed as u32),
            TextureKind::Noise { seed } => (2, seed as u32),
        };
        mats[m.block_id as usize] = GpuMaterial {
            info: [
                u32::from(m.base_rule == BaseAlbedoRule::StudsClassic),
                tex,
                seed,
                m.pbr_texture.map_or(0, |id| id.0),
            ],
            params: [
                m.constant_albedo,
                m.uv_scale[0],
                m.uv_scale[1],
                f32::from(m.cell_variation),
            ],
        };
    }
    q.write_buffer(&g.materials, 0, bytemuck::cast_slice(&mats));
    let pbr_key = (mt.pbr_textures.as_ptr() as usize, mt.pbr_textures.len());
    if g.pbr_key != Some(pbr_key) {
        let packed = pack_pbr_textures(mt);
        q.write_buffer(&g.pbr_texels, 0, bytemuck::cast_slice(&packed));
        g.pbr_key = Some(pbr_key);
    }
    let sprite_key = mt
        .sprite_atlas
        .filter(|atlas| atlas.is_valid())
        .map(|atlas| (atlas.albedo.as_ptr() as usize, atlas.albedo.len()));
    if g.sprite_key != sprite_key {
        if let Some(atlas) = mt.sprite_atlas.filter(|atlas| atlas.is_valid()) {
            let packed = pack_sprite_atlas(atlas);
            q.write_buffer(
                &g.pbr_texels,
                PBR_WORD_CAPACITY * 4,
                bytemuck::cast_slice(&packed),
            );
        }
        g.sprite_key = sprite_key;
    }
    let sty = s.lighting.style;
    let lut = sty.level_lut();
    let tones: Vec<f32> = (0..1024)
        .map(|i| lut.tone(&sty, i as f32 * 2. / 1023.))
        .collect();
    let gammas: Vec<f32> = (0..1024).map(|i| lut.level(i as f32 / 1023.)).collect();
    q.write_buffer(&g.tone, 0, bytemuck::cast_slice(&tones));
    q.write_buffer(&g.gamma, 0, bytemuck::cast_slice(&gammas));
    q.write_buffer(&g.palette, 0, bytemuck::cast_slice(sty.palette.colors()));
    if changed {
        g.bind_group = world_bind(
            d,
            &g.layout,
            &g.uniform,
            &g.voxels,
            &g.materials,
            &g.entities,
            &g.boxes,
            &g.tone,
            &g.gamma,
            &g.palette,
            &g.brick_view,
            &g.noise_view,
            &g.pbr_texels,
        );
    }
    let c = s.camera;
    let f = forward_vec(c.view);
    let r = Vect::new(-c.view.phi.sin(), c.view.phi.cos(), 0.);
    let up = Vect::new(
        -c.view.psi.sin() * c.view.phi.cos(),
        -c.view.psi.sin() * c.view.phi.sin(),
        c.view.psi.cos(),
    );
    let un = WorldUniform {
        dims: [dims.0 as u32, dims.1 as u32, dims.2 as u32, size.0],
        eye: [c.eye.x, c.eye.y, c.eye.z, 0.],
        forward: [f.x, f.y, f.z, 0.],
        right: [r.x, r.y, r.z, 0.],
        up: [up.x, up.y, up.z, 0.],
        projection: [
            c.hfov,
            s.lighting.intensity,
            RENDER_DIST,
            f32::from(sty.flashlight_shadows),
        ],
        lighting: [
            s.lighting.illumination,
            sty.ambient,
            sty.lamp,
            sty.fog_distance,
        ],
        style: [
            sty.ao_strength,
            sty.texture_strength,
            f32::from(sty.pbr_enabled),
            f32::from(sty.realistic_assets),
        ],
        frame: [
            s.lighting.frame,
            es.len() as u32,
            size.1,
            s.empty_voxel as u32,
        ],
        sprite: mt
            .sprite_atlas
            .filter(|atlas| atlas.is_valid())
            .map_or([0; 4], |atlas| {
                [
                    atlas.width as u32,
                    atlas.height as u32,
                    atlas.cell_width as u32,
                    atlas.cell_height as u32,
                ]
            }),
    };
    q.write_buffer(&g.uniform, 0, bytemuck::bytes_of(&un));
    Ok(())
}

fn pack_voxels(blocks: &[u8]) -> Vec<u32> {
    let mut packed = vec![0; blocks.len().div_ceil(4).max(1)];
    for (i, &voxel) in blocks.iter().enumerate() {
        packed[i / 4] |= u32::from(voxel) << ((i % 4) * 8);
    }
    packed
}

fn pack_pbr_textures(materials: &MaterialTable) -> Vec<u32> {
    const MIP_TEXELS: usize = 87_381;
    const MIP_LEVELS: [usize; 9] = [65_536, 16_384, 4_096, 1_024, 256, 64, 16, 4, 1];
    let mut packed = vec![0u32; 4 * 4 * MIP_TEXELS];
    for set in materials.pbr_textures {
        if !(1..=4).contains(&set.id.0) {
            continue;
        }
        let set_index = set.id.0 as usize - 1;
        for (map_index, chain) in [
            &set.albedo,
            &set.normal,
            &set.roughness,
            &set.ambient_occlusion,
        ]
        .into_iter()
        .enumerate()
        {
            let mut mip_offset = 0;
            for (level_index, level) in chain.levels.iter().take(9).enumerate() {
                let map_base = (set_index * 4 + map_index) * MIP_TEXELS + mip_offset;
                for (texel_index, texel) in level
                    .texels
                    .iter()
                    .take(MIP_LEVELS[level_index])
                    .enumerate()
                {
                    packed[map_base + texel_index] = u32::from_le_bytes(*texel);
                }
                mip_offset += MIP_LEVELS[level_index];
            }
        }
    }
    packed
}

fn pack_sprite_atlas(atlas: &SpriteAtlas) -> Vec<u32> {
    let mut packed = Vec::with_capacity(atlas.width * atlas.height * 3);
    packed.extend(atlas.albedo.iter().map(|pixel| u32::from_le_bytes(*pixel)));
    packed.extend(atlas.normal.iter().map(|pixel| u32::from_le_bytes(*pixel)));
    packed.extend(
        atlas
            .emissive
            .iter()
            .map(|value| u32::from(*value) | 0xff00_0000),
    );
    packed
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
    RenderTarget {
        _texture: texture,
        view,
    }
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

#[cfg(test)]
mod world_tests {
    use super::pack_voxels;

    #[test]
    fn gpu_voxel_packing_preserves_linear_ids_across_words() {
        assert_eq!(
            pack_voxels(&[1, 2, 3, 4, 5]),
            vec![0x0403_0201, 0x0000_0005]
        );
        assert_eq!(pack_voxels(&[]), vec![0]);
    }
}
