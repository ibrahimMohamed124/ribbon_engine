use super::*;

impl MeshRenderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        Self::new_with_sample_count(device, queue, format, 1)
    }

    pub(crate) fn new_with_sample_count(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        sample_count: u32,
    ) -> Self {
        let sample_count = match sample_count {
            2 | 4 | 8 => sample_count,
            _ => 1,
        };
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("mesh shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../../shaders/mesh.wgsl").into()),
        });
        let shadow_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("directional shadow shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../../shaders/shadow.wgsl").into()),
        });
        let shadow_size = super::shadows::DIRECTIONAL_SHADOW_RESOLUTION;
        let shadow_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("directional shadow map"),
            size: wgpu::Extent3d {
                width: shadow_size,
                height: shadow_size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let shadow_view = shadow_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let spot_shadow_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("spotlight perspective shadow map"),
            size: wgpu::Extent3d {
                width: shadow_size,
                height: shadow_size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let spot_shadow_view =
            spot_shadow_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("directional shadow comparison sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..wgpu::SamplerDescriptor::default()
        });
        let shadow_target = ShadowTarget {
            _texture: shadow_texture,
            view: shadow_view,
        };
        let spot_shadow_target = ShadowTarget {
            _texture: spot_shadow_texture,
            view: spot_shadow_view,
        };
        let skin_matrices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("per-frame skeletal joint matrices"),
            size: (MAX_SKIN_MATRICES_PER_FRAME * std::mem::size_of::<[[f32; 4]; 4]>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mesh globals layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let shadow_globals_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("shadow globals layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });
        let material_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mesh material layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
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
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mesh pipeline layout"),
            bind_group_layouts: &[&globals_layout, &material_layout],
            push_constant_ranges: &[],
        });
        let shadow_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("directional shadow pipeline layout"),
                bind_group_layouts: &[&shadow_globals_layout, &material_layout],
                push_constant_ranges: &[],
            });
        let shadow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("directional shadow pipeline"),
            layout: Some(&shadow_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shadow_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[
                    wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Vertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &VERTEX_ATTRIBUTES,
                    },
                    wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<RawInstance>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &INSTANCE_ATTRIBUTES,
                    },
                ],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                ..wgpu::PrimitiveState::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState {
                    constant: 2,
                    slope_scale: 2.0,
                    clamp: 0.0,
                },
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shadow_shader,
                entry_point: Some("fs_shadow"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[],
            }),
            multiview: None,
            cache: None,
        });
        // Double-sided materials get a no-cull directional shadow variant without
        // disabling back-face culling for ordinary materials.
        let shadow_double_sided_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("directional double-sided shadow pipeline"),
                layout: Some(&shadow_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shadow_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[
                        wgpu::VertexBufferLayout {
                            array_stride: std::mem::size_of::<Vertex>() as u64,
                            step_mode: wgpu::VertexStepMode::Vertex,
                            attributes: &VERTEX_ATTRIBUTES,
                        },
                        wgpu::VertexBufferLayout {
                            array_stride: std::mem::size_of::<RawInstance>() as u64,
                            step_mode: wgpu::VertexStepMode::Instance,
                            attributes: &INSTANCE_ATTRIBUTES,
                        },
                    ],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: None,
                    ..wgpu::PrimitiveState::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: true,
                    depth_compare: wgpu::CompareFunction::Less,
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState {
                        constant: 2,
                        slope_scale: 2.0,
                        clamp: 0.0,
                    },
                }),
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shadow_shader,
                    entry_point: Some("fs_shadow"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[],
                }),
                multiview: None,
                cache: None,
            });
        // A spotlight commonly sits inside closed room geometry. Render both
        // sides into its depth map so inward-facing corridor surfaces still write
        // depth when viewed from inside the room. Keep its projection and bias independent.
        let spot_shadow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("spotlight shadow pipeline"),
            layout: Some(&shadow_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shadow_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[
                    wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Vertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &VERTEX_ATTRIBUTES,
                    },
                    wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<RawInstance>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &INSTANCE_ATTRIBUTES,
                    },
                ],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                ..wgpu::PrimitiveState::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shadow_shader,
                entry_point: Some("fs_shadow"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[],
            }),
            multiview: None,
            cache: None,
        });
        let pipeline = create_color_pipeline(
            device,
            &shader,
            &pipeline_layout,
            ColorPipelineOptions {
                format,
                label: "opaque mesh pipeline",
                blended: false,
                double_sided: false,
                sample_count,
            },
        );
        let double_sided_pipeline = create_color_pipeline(
            device,
            &shader,
            &pipeline_layout,
            ColorPipelineOptions {
                format,
                label: "opaque double-sided mesh pipeline",
                blended: false,
                double_sided: true,
                sample_count,
            },
        );
        let blend_pipeline = create_color_pipeline(
            device,
            &shader,
            &pipeline_layout,
            ColorPipelineOptions {
                format,
                label: "transparent mesh pipeline",
                blended: true,
                double_sided: false,
                sample_count,
            },
        );
        let blend_double_sided_pipeline = create_color_pipeline(
            device,
            &shader,
            &pipeline_layout,
            ColorPipelineOptions {
                format,
                label: "transparent double-sided mesh pipeline",
                blended: true,
                double_sided: true,
                sample_count,
            },
        );
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mesh globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mesh globals bind group"),
            layout: &globals_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: globals.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&shadow_target.view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&shadow_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&spot_shadow_target.view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&shadow_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: skin_matrices.as_entire_binding(),
                },
            ],
        });
        let shadow_globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("directional shadow globals"),
            size: std::mem::size_of::<ShadowGlobals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let shadow_globals_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("directional shadow globals bind group"),
            layout: &shadow_globals_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: shadow_globals.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: skin_matrices.as_entire_binding(),
                },
            ],
        });
        let spot_shadow_globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("spotlight shadow globals"),
            size: std::mem::size_of::<ShadowGlobals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let spot_shadow_globals_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("spotlight shadow globals bind group"),
            layout: &shadow_globals_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: spot_shadow_globals.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: skin_matrices.as_entire_binding(),
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("mesh sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            anisotropy_clamp: 8,
            ..wgpu::SamplerDescriptor::default()
        });
        let mut renderer = Self {
            sample_count,
            pipeline,
            double_sided_pipeline,
            blend_pipeline,
            blend_double_sided_pipeline,
            shadow_pipeline,
            shadow_double_sided_pipeline,
            spot_shadow_pipeline,
            globals,
            globals_group,
            shadow_globals,
            shadow_globals_group,
            skin_matrices,
            spot_shadow_globals,
            spot_shadow_globals_group,
            shadow_target,
            spot_shadow_target,
            _shadow_sampler: shadow_sampler,
            material_layout,
            sampler,
            meshes: Vec::new(),
            free_mesh_slots: Vec::new(),
            materials: Vec::new(),
            free_material_slots: Vec::new(),
            textures: Vec::new(),
            free_texture_slots: Vec::new(),
            samplers: Vec::new(),
            free_sampler_slots: Vec::new(),
            texture_cache: HashMap::new(),
            texture_cache_hits: 0,
            texture_cache_misses: 0,
            gpu_timing: GpuTimestampProfiler::new(device),
            peak_resource_bytes_estimate: 0,
            last_visible_mesh_instances: 0,
            last_visible_draw_calls: 0,
            last_shadow_draw_calls: 0,
            last_skinned_draws_after_palette_budget: 0,
            last_skinned_draws_rejected_palette_capacity: 0,
            last_skin_palette_matrices_staged: 0,
            instances: instance_buffer(device, 16),
            shadow_instances: instance_buffer(device, 16),
            instance_capacity: MIN_INSTANCE_CAPACITY,
            underused_instance_frames: 0,
            instance_buffer_grows_cumulative: 0,
            instance_buffer_shrinks_cumulative: 0,
            depth: None,
        };
        let cube = mesh::cube();
        let cube_id = renderer.upload_mesh(device, &cube.vertices, &cube.indices);
        debug_assert_eq!(cube_id, Self::CUBE);
        let white = renderer.upload_material(device, queue, [1.0; 4], None);
        debug_assert_eq!(white, Self::WHITE);
        renderer
    }
}

#[derive(Clone, Copy)]
struct ColorPipelineOptions {
    format: wgpu::TextureFormat,
    label: &'static str,
    blended: bool,
    double_sided: bool,
    sample_count: u32,
}

fn create_color_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    options: ColorPipelineOptions,
) -> wgpu::RenderPipeline {
    let ColorPipelineOptions {
        format,
        label,
        blended,
        double_sided,
        sample_count,
    } = options;
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[
                wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &VERTEX_ATTRIBUTES,
                },
                wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<RawInstance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &INSTANCE_ATTRIBUTES,
                },
            ],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: if double_sided {
                None
            } else {
                Some(wgpu::Face::Back)
            },
            ..wgpu::PrimitiveState::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: !blended,
            depth_compare: wgpu::CompareFunction::Less,
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState {
            count: sample_count,
            mask: !0,
            alpha_to_coverage_enabled: false,
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: if blended {
                    Some(wgpu::BlendState::ALPHA_BLENDING)
                } else {
                    None
                },
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview: None,
        cache: None,
    })
}
