//! GPU presentation and rendering backend.

use std::sync::Arc;

use engine_core::gfx::Framebuffer;
use wgpu::util::DeviceExt;
use winit::window::Window;

const PRESENT_SHADER: &str = r#"
struct Screen { size: vec2<f32> };
@group(0) @binding(0) var source_image: texture_2d<f32>;
@group(0) @binding(1) var<uniform> screen: Screen;

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    return vec4<f32>(positions[index], 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let source_size = vec2<f32>(textureDimensions(source_image));
    let scale = min(screen.size.x / source_size.x, screen.size.y / source_size.y);
    let offset = (screen.size - source_size * scale) * 0.5;
    let local = position.xy - offset;
    if (local.x < 0.0 || local.y < 0.0 || local.x >= source_size.x * scale || local.y >= source_size.y * scale) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    let pixel = vec2<i32>(floor(local / scale));
    return vec4<f32>(textureLoad(source_image, pixel, 0).rgb, 1.0);
}
"#;

/// Persistent nearest-neighbor surface presenter for an internal CPU image.
///
/// This is the Phase 2 bridge; Phase 3 and 4 replace the upload source with GPU
/// overlay and world passes while retaining the same surface and present pass.
pub struct GpuPresenter {
    _instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface_config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    screen_buffer: wgpu::Buffer,
    source_texture: wgpu::Texture,
    source_view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
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
            .unwrap_or(capabilities.formats[0]);
        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: capabilities.alpha_modes[0],
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
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let screen_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("surface dimensions"),
            contents: &[0; 8],
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("nearest upscale shader"),
            source: wgpu::ShaderSource::Wgsl(PRESENT_SHADER.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("present pipeline layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("nearest upscale pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        let source_texture = create_source_texture(&device, internal_size);
        let source_view = source_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group =
            create_bind_group(&device, &bind_group_layout, &source_view, &screen_buffer);

        Ok(Self {
            _instance: instance,
            surface,
            device,
            queue,
            surface_config,
            pipeline,
            bind_group_layout,
            screen_buffer,
            source_texture,
            source_view,
            bind_group,
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

    pub fn present(&mut self, framebuffer: &Framebuffer) -> Result<(), String> {
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
                texture: &self.source_texture,
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
        let mut uniform = [0u8; 8];
        uniform[..4].copy_from_slice(&self.surface_config.width.to_ne_bytes());
        uniform[4..].copy_from_slice(&self.surface_config.height.to_ne_bytes());
        self.queue.write_buffer(&self.screen_buffer, 0, &uniform);

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
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("present encoder"),
            });
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
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit(Some(encoder.finish()));
        frame.present();
        Ok(())
    }

    fn resize_internal(&mut self, size: (u32, u32)) {
        self.internal_size = size;
        self.source_texture = create_source_texture(&self.device, size);
        self.source_view = self
            .source_texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.bind_group = create_bind_group(
            &self.device,
            &self.bind_group_layout,
            &self.source_view,
            &self.screen_buffer,
        );
        self.upload_bytes
            .resize(size.0 as usize * size.1 as usize * 4, 0);
    }
}

fn create_source_texture(device: &wgpu::Device, size: (u32, u32)) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("internal CPU framebuffer upload"),
        size: wgpu::Extent3d {
            width: size.0.max(1),
            height: size.1.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
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
                resource: screen.as_entire_binding(),
            },
        ],
    })
}
