//! wgpu pipeline that draws instanced, textured meshes with a depth buffer.

use crate::components::SunLight;
use crate::mesh::{self, Vertex};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3, Vec4};
use wgpu::util::DeviceExt;

/// Depth format used by the mesh pass.
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Handle to a mesh stored in a [`MeshRenderer`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MeshId(usize);

/// Handle to a material stored in a [`MeshRenderer`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MaterialId(usize);

/// RGBA8 pixels in sRGB encoding, row-major from the top-left.
#[derive(Clone, Copy, Debug)]
pub struct TextureData<'a> {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Tightly packed RGBA8 pixels.
    pub rgba8: &'a [u8],
}

/// Parameters and optional texture maps for a metallic-roughness material.
#[derive(Clone, Copy, Debug)]
pub struct PbrMaterialDesc<'a> {
    /// Linear RGBA base-color factor.
    pub base_color: [f32; 4],
    /// Metallic factor in the inclusive range 0..=1.
    pub metallic: f32,
    /// Roughness factor, clamped by the renderer to 0.04..=1.
    pub roughness: f32,
    /// Optional sRGB base-color texture.
    pub base_color_texture: Option<TextureData<'a>>,
    /// Optional linear glTF metallic-roughness texture (G=roughness, B=metallic).
    pub metallic_roughness_texture: Option<TextureData<'a>>,
}

/// One object to draw.
#[derive(Clone, Copy, Debug)]
pub struct Draw {
    /// Geometry to draw.
    pub mesh: MeshId,
    /// Surface material.
    pub material: MaterialId,
    /// Object-to-world matrix.
    pub model: Mat4,
    /// Linear RGB multiplier applied on top of the material.
    pub tint: [f32; 3],
}

/// Everything one render call needs besides the GPU handles.
pub struct Scene<'a> {
    /// Combined projection and view matrix.
    pub view_proj: Mat4,
    /// Camera world position used by view-dependent BRDF terms.
    pub camera_position: Vec3,
    /// Directional light and ambient term.
    pub sun: &'a SunLight,
    /// Objects to draw.
    pub draws: &'a [Draw],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct RawInstance {
    model: [[f32; 4]; 4],
    tint: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    light_view_proj: [[f32; 4]; 4],
    sun_dir: [f32; 4],
    sun_color: [f32; 4],
    ambient: [f32; 4],
    camera_pos: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ShadowGlobals {
    light_view_proj: [[f32; 4]; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MaterialUniform {
    base_color: [f32; 4],
    /// x: metallic, y: perceptual roughness, remaining channels reserved.
    surface: [f32; 4],
}

struct DepthTarget {
    view: wgpu::TextureView,
    size: (u32, u32),
}

struct ShadowTarget {
    // Keep the texture alive alongside its view.
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

struct GpuMesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    bounds_center: Vec3,
    bounds_radius: f32,
}

struct GpuMaterial {
    bind_group: wgpu::BindGroup,
    _uniform: wgpu::Buffer,
    _texture: wgpu::Texture,
    _metallic_roughness_texture: wgpu::Texture,
}

/// Owns the pipeline, uploaded meshes and materials, and the depth target.
pub struct MeshRenderer {
    pipeline: wgpu::RenderPipeline,
    shadow_pipeline: wgpu::RenderPipeline,
    globals: wgpu::Buffer,
    globals_group: wgpu::BindGroup,
    shadow_globals: wgpu::Buffer,
    shadow_globals_group: wgpu::BindGroup,
    shadow_target: ShadowTarget,
    _shadow_sampler: wgpu::Sampler,
    material_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    meshes: Vec<GpuMesh>,
    materials: Vec<GpuMaterial>,
    /// Compact, frustum-visible instances used by the main color pass.
    instances: wgpu::Buffer,
    /// All instances, including off-screen casters, used by the shadow pass.
    shadow_instances: wgpu::Buffer,
    instance_capacity: usize,
    depth: Option<DepthTarget>,
}

const VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 3] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2];
const INSTANCE_ATTRIBUTES: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
    3 => Float32x4, 4 => Float32x4, 5 => Float32x4, 6 => Float32x4, 7 => Float32x4
];

fn instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("mesh instances"),
        size: (capacity * std::mem::size_of::<RawInstance>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn srgb_to_linear(value: u8) -> f32 {
    let s = f32::from(value) / 255.0;
    if s <= 0.04045 {
        s / 12.92
    } else {
        ((s + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(value: f32) -> u8 {
    let v = value.clamp(0.0, 1.0);
    let s = if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    };
    (s * 255.0).round() as u8
}

/// Box-filter mip chain (level 0 first). Color is averaged in linear light.
pub fn build_mips(width: u32, height: u32, rgba8: &[u8]) -> Vec<(u32, u32, Vec<u8>)> {
    let mut levels = vec![(width, height, rgba8.to_vec())];
    while let Some((w, h, previous)) = levels.last() {
        if *w == 1 && *h == 1 {
            break;
        }
        let (w, h) = (*w, *h);
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let mut next = Vec::with_capacity((nw * nh * 4) as usize);
        for y in 0..nh {
            for x in 0..nw {
                let mut color = [0.0_f32; 3];
                let mut alpha = 0.0_f32;
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let sx = (x * 2 + dx).min(w - 1);
                    let sy = (y * 2 + dy).min(h - 1);
                    let at = ((sy * w + sx) * 4) as usize;
                    for (channel, sum) in color.iter_mut().enumerate() {
                        *sum += srgb_to_linear(previous[at + channel]);
                    }
                    alpha += f32::from(previous[at + 3]);
                }
                next.extend(color.map(|sum| linear_to_srgb(sum / 4.0)));
                next.push((alpha / 4.0).round() as u8);
            }
        }
        levels.push((nw, nh, next));
    }
    levels
}

/// Box-filter non-color data such as glTF metallic-roughness maps without sRGB conversion.
fn build_data_mips(width: u32, height: u32, rgba8: &[u8]) -> Vec<(u32, u32, Vec<u8>)> {
    let mut levels = vec![(width, height, rgba8.to_vec())];
    while let Some((w, h, previous)) = levels.last() {
        if *w == 1 && *h == 1 {
            break;
        }
        let (w, h) = (*w, *h);
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let mut next = Vec::with_capacity((nw * nh * 4) as usize);
        for y in 0..nh {
            for x in 0..nw {
                for channel in 0..4 {
                    let mut sum = 0_u32;
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let sx = (x * 2 + dx).min(w - 1);
                        let sy = (y * 2 + dy).min(h - 1);
                        let at = ((sy * w + sx) * 4 + channel) as usize;
                        sum += u32::from(previous[at]);
                    }
                    next.push(((sum + 2) / 4) as u8);
                }
            }
        }
        levels.push((nw, nh, next));
    }
    levels
}

fn upload_rgba_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    source: TextureData<'_>,
    label: &'static str,
    srgb: bool,
) -> (wgpu::Texture, wgpu::TextureView) {
    let levels = if srgb {
        build_mips(source.width, source.height, source.rgba8)
    } else {
        build_data_mips(source.width, source.height, source.rgba8)
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: source.width,
            height: source.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: levels.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: if srgb {
            wgpu::TextureFormat::Rgba8UnormSrgb
        } else {
            wgpu::TextureFormat::Rgba8Unorm
        },
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (level, (width, height, pixels)) in levels.iter().enumerate() {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: level as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(*height),
            },
            wgpu::Extent3d {
                width: *width,
                height: *height,
                depth_or_array_layers: 1,
            },
        );
    }
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

fn frustum_planes(view_proj: Mat4) -> [Vec4; 6] {
    let columns = view_proj.to_cols_array_2d();
    let row0 = Vec4::new(columns[0][0], columns[1][0], columns[2][0], columns[3][0]);
    let row1 = Vec4::new(columns[0][1], columns[1][1], columns[2][1], columns[3][1]);
    let row2 = Vec4::new(columns[0][2], columns[1][2], columns[2][2], columns[3][2]);
    let row3 = Vec4::new(columns[0][3], columns[1][3], columns[2][3], columns[3][3]);
    let normalize_plane = |plane: Vec4| {
        let length = plane.truncate().length();
        if length > 1.0e-6 {
            plane / length
        } else {
            plane
        }
    };
    [
        normalize_plane(row3 + row0),
        normalize_plane(row3 - row0),
        normalize_plane(row3 + row1),
        normalize_plane(row3 - row1),
        normalize_plane(row2),
        normalize_plane(row3 - row2),
    ]
}

fn sphere_visible(planes: &[Vec4; 6], center: Vec3, radius: f32) -> bool {
    planes
        .iter()
        .all(|plane| plane.truncate().dot(center) + plane.w >= -radius)
}

fn valid_texture<'a>(texture: Option<TextureData<'a>>, fallback: &'a [u8; 4]) -> TextureData<'a> {
    match texture {
        Some(texture)
            if texture.width > 0
                && texture.height > 0
                && texture.rgba8.len()
                    == (texture.width as usize) * (texture.height as usize) * 4 =>
        {
            texture
        }
        _ => TextureData {
            width: 1,
            height: 1,
            rgba8: fallback,
        },
    }
}

impl MeshRenderer {
    /// The built-in unit cube.
    pub const CUBE: MeshId = MeshId(0);
    /// Plain white material (no texture).
    pub const WHITE: MaterialId = MaterialId(0);

    /// Build the pipeline for render targets of `format`.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("mesh shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/mesh.wgsl").into()),
        });
        let shadow_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("directional shadow shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/shadow.wgsl").into()),
        });
        let shadow_size = 1024_u32;
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
            ],
        });
        let shadow_globals_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("shadow globals layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
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
                bind_group_layouts: &[&shadow_globals_layout],
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
            fragment: None,
            multiview: None,
            cache: None,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("mesh pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
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
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview: None,
            cache: None,
        });
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
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: shadow_globals.as_entire_binding(),
            }],
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
            pipeline,
            shadow_pipeline,
            globals,
            globals_group,
            shadow_globals,
            shadow_globals_group,
            shadow_target,
            _shadow_sampler: shadow_sampler,
            material_layout,
            sampler,
            meshes: Vec::new(),
            materials: Vec::new(),
            instances: instance_buffer(device, 16),
            shadow_instances: instance_buffer(device, 16),
            instance_capacity: 16,
            depth: None,
        };
        let cube = mesh::cube();
        let cube_id = renderer.upload_mesh(device, &cube.vertices, &cube.indices);
        debug_assert_eq!(cube_id, Self::CUBE);
        let white = renderer.upload_material(device, queue, [1.0; 4], None);
        debug_assert_eq!(white, Self::WHITE);
        renderer
    }

    /// Upload indexed triangle geometry.
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
        self.meshes.push(GpuMesh {
            vertices: vertex_buffer,
            indices: index_buffer,
            index_count: indices.len() as u32,
            bounds_center,
            bounds_radius,
        });
        MeshId(self.meshes.len() - 1)
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
            },
        )
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
        } = material;
        let white = [255_u8; 4];
        let base_source = valid_texture(base_color_texture, &white);
        let surface_source = valid_texture(metallic_roughness_texture, &white);
        let (gpu_texture, view) =
            upload_rgba_texture(device, queue, base_source, "material sRGB base color", true);
        // Metallic and roughness values are linear data, not sRGB colors.
        let (surface_texture, surface_view) = upload_rgba_texture(
            device,
            queue,
            surface_source,
            "material metallic roughness map",
            false,
        );
        let factors = MaterialUniform {
            base_color,
            surface: [
                metallic.clamp(0.0, 1.0),
                roughness.clamp(0.04, 1.0),
                0.0,
                0.0,
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
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&surface_view),
                },
            ],
        });
        self.materials.push(GpuMaterial {
            bind_group,
            _uniform: uniform,
            _texture: gpu_texture,
            _metallic_roughness_texture: surface_texture,
        });
        MaterialId(self.materials.len() - 1)
    }

    fn ensure_depth(&mut self, device: &wgpu::Device, size: (u32, u32)) {
        if self.depth.as_ref().is_some_and(|depth| depth.size == size) {
            return;
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("mesh depth"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        self.depth = Some(DepthTarget {
            view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
            size,
        });
    }

    fn directional_shadow_matrix(scene: &Scene<'_>) -> Mat4 {
        let direction = scene.sun.direction.normalize_or_zero();
        let direction = if direction.length_squared() > 0.5 {
            direction
        } else {
            Vec3::new(-0.4, -0.8, -0.45).normalize()
        };
        let focus = scene.camera_position;
        let eye = focus - direction * 32.0;
        let up = if direction.dot(Vec3::Y).abs() > 0.95 {
            Vec3::Z
        } else {
            Vec3::Y
        };
        Mat4::orthographic_rh(-24.0, 24.0, -24.0, 24.0, 0.1, 80.0)
            * Mat4::look_at_rh(eye, focus, up)
    }

    /// Draw `scene` over the existing contents of `view`. The directional shadow map
    /// is rendered first, then sampled by the physically based forward material pass.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        view: &wgpu::TextureView,
        size: (u32, u32),
        scene: &Scene<'_>,
    ) {
        if scene.draws.is_empty() || size.0 == 0 || size.1 == 0 {
            return;
        }
        self.ensure_depth(device, size);
        // Keep every valid draw for shadow casting, but only upload camera-visible instances
        // to the main pass. The sphere scale uses a Frobenius-norm bound so non-uniform or
        // sheared transforms remain conservative (possibly overestimating, never underestimating).
        let mut draws: Vec<Draw> = scene
            .draws
            .iter()
            .copied()
            .filter(|draw| {
                draw.mesh.0 < self.meshes.len() && draw.material.0 < self.materials.len()
            })
            .collect();
        draws.sort_by_key(|draw| (draw.mesh, draw.material));
        if draws.is_empty() {
            return;
        }
        let planes = frustum_planes(scene.view_proj);
        let visible_draws: Vec<Draw> = draws
            .iter()
            .copied()
            .filter(|draw| {
                let mesh = &self.meshes[draw.mesh.0];
                let center = draw.model.transform_point3(mesh.bounds_center);
                let x = draw.model.x_axis.truncate();
                let y = draw.model.y_axis.truncate();
                let z = draw.model.z_axis.truncate();
                let max_scale =
                    (x.length_squared() + y.length_squared() + z.length_squared()).sqrt();
                sphere_visible(&planes, center, mesh.bounds_radius * max_scale)
            })
            .collect();
        if visible_draws.is_empty() {
            return;
        }
        if draws.len() > self.instance_capacity {
            self.instance_capacity = draws.len().next_power_of_two();
            self.instances = instance_buffer(device, self.instance_capacity);
            self.shadow_instances = instance_buffer(device, self.instance_capacity);
        }
        let to_raw = |instances: &[Draw]| {
            instances
                .iter()
                .map(|draw| RawInstance {
                    model: draw.model.to_cols_array_2d(),
                    tint: [draw.tint[0], draw.tint[1], draw.tint[2], 1.0],
                })
                .collect::<Vec<_>>()
        };
        let shadow_raw = to_raw(&draws);
        let visible_raw = to_raw(&visible_draws);
        queue.write_buffer(&self.shadow_instances, 0, bytemuck::cast_slice(&shadow_raw));
        queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(&visible_raw));
        let light_view_proj = Self::directional_shadow_matrix(scene);
        let globals = Globals {
            view_proj: scene.view_proj.to_cols_array_2d(),
            light_view_proj: light_view_proj.to_cols_array_2d(),
            sun_dir: scene
                .sun
                .direction
                .normalize_or_zero()
                .extend(0.0)
                .to_array(),
            sun_color: scene.sun.color.extend(1.0).to_array(),
            ambient: scene.sun.ambient.extend(1.0).to_array(),
            camera_pos: scene.camera_position.extend(1.0).to_array(),
        };
        queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&globals));
        let shadow_globals = ShadowGlobals {
            light_view_proj: light_view_proj.to_cols_array_2d(),
        };
        queue.write_buffer(&self.shadow_globals, 0, bytemuck::bytes_of(&shadow_globals));
        let Some(depth) = self.depth.as_ref() else {
            return;
        };
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("mesh and directional shadow passes"),
        });
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
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.shadow_pipeline);
            pass.set_bind_group(0, &self.shadow_globals_group, &[]);
            pass.set_vertex_buffer(1, self.shadow_instances.slice(..));
            let mut start = 0;
            while start < draws.len() {
                let mesh_id = draws[start].mesh;
                let end = start
                    + draws[start..]
                        .iter()
                        .take_while(|draw| draw.mesh == mesh_id)
                        .count();
                let mesh = &self.meshes[mesh_id.0];
                pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..mesh.index_count, 0, start as u32..end as u32);
                start = end;
            }
        }
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
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.globals_group, &[]);
            pass.set_vertex_buffer(1, self.instances.slice(..));
            let mut start = 0;
            while start < visible_draws.len() {
                let key = (visible_draws[start].mesh, visible_draws[start].material);
                let end = start
                    + visible_draws[start..]
                        .iter()
                        .take_while(|d| (d.mesh, d.material) == key)
                        .count();
                let mesh = &self.meshes[key.0 .0];
                pass.set_bind_group(1, &self.materials[key.1 .0].bind_group, &[]);
                pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..mesh.index_count, 0, start as u32..end as u32);
                start = end;
            }
        }
        queue.submit(Some(encoder.finish()));
    }
}

#[cfg(test)]
mod tests {
    use super::{build_mips, Draw, MeshRenderer, Scene, TextureData};
    use crate::components::{Camera3D, SunLight, Transform3D};
    use engine_render::gpu::GpuContext;
    use engine_render::gpu_window::clear_view;
    use glam::{Mat4, Vec3};

    const SIZE: u32 = 64;

    struct Harness {
        context: GpuContext,
        renderer: MeshRenderer,
    }

    fn harness() -> Option<Harness> {
        let context = match pollster::block_on(GpuContext::new_offscreen(SIZE, SIZE)) {
            Ok(context) => context,
            Err(error) => {
                eprintln!("skipping GPU test: {}", error.0);
                return None;
            }
        };
        clear_view(
            context.device(),
            context.queue(),
            context.target().view(),
            [0.0, 0.0, 0.0, 1.0],
        );
        let renderer = MeshRenderer::new(
            context.device(),
            context.queue(),
            wgpu::TextureFormat::Rgba8Unorm,
        );
        Some(Harness { context, renderer })
    }

    impl Harness {
        fn draw(&mut self, draws: &[Draw]) -> Vec<u32> {
            let eye = Transform3D::looking_at(Vec3::new(0.0, 0.0, 4.0), Vec3::ZERO, Vec3::Y);
            let view = Mat4::from_rotation_translation(eye.rotation, eye.translation).inverse();
            self.renderer.render(
                self.context.device(),
                self.context.queue(),
                self.context.target().view(),
                (SIZE, SIZE),
                &Scene {
                    view_proj: Camera3D::default().projection(1.0) * view,
                    camera_position: eye.translation,
                    sun: &SunLight::default(),
                    draws,
                },
            );
            self.context.readback().expect("readback").pixels
        }
    }

    fn pixel(pixels: &[u32], x: u32, y: u32) -> (u32, u32, u32) {
        let p = pixels[(y * SIZE + x) as usize];
        ((p >> 16) & 0xff, (p >> 8) & 0xff, p & 0xff)
    }

    fn cube(model: Mat4, tint: [f32; 3]) -> Draw {
        Draw {
            mesh: MeshRenderer::CUBE,
            material: MeshRenderer::WHITE,
            model,
            tint,
        }
    }

    #[test]
    fn frustum_rejects_objects_behind_the_camera_and_keeps_visible_objects() {
        let projection = Camera3D::default().projection(1.0);
        let view = Mat4::IDENTITY;
        let planes = super::frustum_planes(projection * view);
        assert!(super::sphere_visible(
            &planes,
            Vec3::new(0.0, 0.0, -3.0),
            0.5
        ));
        assert!(!super::sphere_visible(
            &planes,
            Vec3::new(0.0, 0.0, 3.0),
            0.5
        ));
    }

    #[test]
    fn cube_is_drawn_in_the_center_and_background_is_untouched() {
        let Some(mut h) = harness() else { return };
        let pixels = h.draw(&[cube(Mat4::IDENTITY, [1.0, 0.0, 0.0])]);
        let (r, g, b) = pixel(&pixels, SIZE / 2, SIZE / 2);
        // PBR specular highlights can contribute a small amount to channels that
        // are zero in the base color, so test red dominance rather than exact RGB.
        assert!(
            r > 0 && r > 4 * g && r > 4 * b,
            "expected a red-dominant center pixel, got {r},{g},{b}"
        );
        assert_eq!(pixel(&pixels, 0, 0), (0, 0, 0));
    }

    #[test]
    fn depth_test_keeps_the_nearer_cube_in_front() {
        let Some(mut h) = harness() else { return };
        let far = Mat4::from_translation(Vec3::new(0.0, 0.0, -3.0));
        // The far cube is submitted last; without a depth test it would overwrite the red one.
        let pixels = h.draw(&[
            cube(Mat4::IDENTITY, [1.0, 0.0, 0.0]),
            cube(far, [0.0, 0.0, 1.0]),
        ]);
        let (r, _, b) = pixel(&pixels, SIZE / 2, SIZE / 2);
        assert!(
            r > 0 && r > 4 * b,
            "expected the near red cube to dominate, got r={r} b={b}"
        );
    }

    #[test]
    fn base_color_texture_shows_up_on_the_surface() {
        let Some(mut h) = harness() else { return };
        // 2x2 checker: red, green / blue, white.
        let texels: [u8; 16] = [
            255, 0, 0, 255, 0, 255, 0, 255, //
            0, 0, 255, 255, 255, 255, 255, 255,
        ];
        let material = h.renderer.upload_material(
            h.context.device(),
            h.context.queue(),
            [1.0; 4],
            Some(TextureData {
                width: 2,
                height: 2,
                rgba8: &texels,
            }),
        );
        let pixels = h.draw(&[Draw {
            mesh: MeshRenderer::CUBE,
            material,
            model: Mat4::IDENTITY,
            tint: [1.0; 3],
        }]);
        let mut colors = std::collections::HashSet::new();
        for y in 0..SIZE {
            for x in 0..SIZE {
                let p = pixel(&pixels, x, y);
                if p != (0, 0, 0) {
                    colors.insert(p);
                }
            }
        }
        assert!(
            colors.len() >= 3,
            "texture should produce several colors, saw {}",
            colors.len()
        );
    }

    #[test]
    fn mip_chain_halves_down_to_one_pixel_and_averages_in_linear_light() {
        let texels: [u8; 16] = [
            0, 0, 0, 255, 255, 255, 255, 255, //
            255, 255, 255, 255, 0, 0, 0, 255,
        ];
        let mips = build_mips(2, 2, &texels);
        assert_eq!(mips.len(), 2);
        assert_eq!((mips[1].0, mips[1].1), (1, 1));
        // Half black, half white is 0.5 linear, which encodes to about 188 in sRGB.
        assert!((187..=188).contains(&mips[1].2[0]), "{}", mips[1].2[0]);
        let odd = build_mips(5, 3, &[255; 5 * 3 * 4]);
        let sizes: Vec<_> = odd.iter().map(|(w, h, _)| (*w, *h)).collect();
        assert_eq!(sizes, vec![(5, 3), (2, 1), (1, 1)]);
    }
}
