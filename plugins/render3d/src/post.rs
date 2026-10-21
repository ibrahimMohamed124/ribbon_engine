//! HDR scene target and fullscreen tone-mapping pass.

/// Linear HDR color format used between the mesh renderer and presentation.
pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Window-sized offscreen color target. The owning texture is retained with its view.
pub struct HdrTarget {
    _texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub size: (u32, u32),
}

impl HdrTarget {
    /// Allocate or resize the HDR color target.
    pub fn new(device: &wgpu::Device, size: (u32, u32)) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("render3d HDR scene color"),
            size: wgpu::Extent3d {
                width: size.0.max(1),
                height: size.1.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            _texture: texture,
            view,
            size,
        }
    }
}

/// Fullscreen Reinhard tone mapper from the linear HDR target to the window surface.
pub struct ToneMapper {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    bloom_pipeline: wgpu::RenderPipeline,
    bloom_bind_group: wgpu::BindGroup,
    _bloom_texture: wgpu::Texture,
    bloom_view: wgpu::TextureView,
    _sampler: wgpu::Sampler,
}

impl ToneMapper {
    /// Build a mapper for an HDR source view and the current surface format.
    pub fn new(
        device: &wgpu::Device,
        source: &wgpu::TextureView,
        surface_format: wgpu::TextureFormat,
        size: (u32, u32),
    ) -> Self {
        Self::new_with_shader(
            device,
            source,
            surface_format,
            size,
            include_str!("../shaders/tonemap.wgsl"),
        )
    }

    /// Build a mapper for a linear UNORM output target. Unlike an sRGB surface, an
    /// `Rgba8Unorm` texture does not encode output values automatically, so this pass
    /// performs the linear-to-sRGB transfer after tone mapping.
    pub fn new_offscreen_unorm(
        device: &wgpu::Device,
        source: &wgpu::TextureView,
        size: (u32, u32),
    ) -> Self {
        Self::new_with_shader(
            device,
            source,
            wgpu::TextureFormat::Rgba8Unorm,
            size,
            include_str!("../shaders/tonemap_unorm.wgsl"),
        )
    }

    fn new_with_shader(
        device: &wgpu::Device,
        source: &wgpu::TextureView,
        surface_format: wgpu::TextureFormat,
        size: (u32, u32),
        shader_source: &str,
    ) -> Self {
        let bloom_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("render3d quarter-resolution bloom texture"),
            size: wgpu::Extent3d {
                width: (size.0 / 4).max(1),
                height: (size.1 / 4).max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let bloom_view = bloom_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bloom_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bloom extraction layout"),
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
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tone mapper layout"),
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
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("tone mapper sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..wgpu::SamplerDescriptor::default()
        });
        let bloom_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bright pass bind group"),
            layout: &bloom_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("tone mapper bind group"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&bloom_view),
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tone mapper shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("tone mapper pipeline layout"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("tone mapper pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..wgpu::PrimitiveState::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview: None,
            cache: None,
        });
        let bloom_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bright-pass bloom shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/bloom.wgsl").into()),
        });
        let bloom_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("bright-pass bloom pipeline layout"),
                bind_group_layouts: &[&bloom_layout],
                push_constant_ranges: &[],
            });
        let bloom_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("bright-pass bloom pipeline"),
            layout: Some(&bloom_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &bloom_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..wgpu::PrimitiveState::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &bloom_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview: None,
            cache: None,
        });
        Self {
            pipeline,
            bind_group,
            bloom_pipeline,
            bloom_bind_group,
            _bloom_texture: bloom_texture,
            bloom_view,
            _sampler: sampler,
        }
    }

    /// Draw the fullscreen tone-mapping triangle into the acquired window image.
    pub fn render(&self, device: &wgpu::Device, queue: &wgpu::Queue, target: &wgpu::TextureView) {
        let mut bloom_encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("bloom bright-pass encoder"),
        });
        {
            let mut pass = bloom_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("quarter-resolution bloom extraction and blur"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.bloom_view,
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
            pass.set_pipeline(&self.bloom_pipeline);
            pass.set_bind_group(0, &self.bloom_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        queue.submit(Some(bloom_encoder.finish()));
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("tone mapping pass"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("tone mapping"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
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
        queue.submit(Some(encoder.finish()));
    }
}
