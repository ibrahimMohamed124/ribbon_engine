//! wgpu pipeline that draws instanced, textured meshes with a depth buffer.

use crate::components::{FogSettings, PointLight, SpotLight, SunLight};
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
    /// Optional tangent-space normal texture (linear RGB).
    pub normal_texture: Option<TextureData<'a>>,
    /// Optional red-channel ambient occlusion texture (linear).
    pub occlusion_texture: Option<TextureData<'a>>,
    /// Multiplier for the occlusion texture (0..=1).
    pub occlusion_strength: f32,
    /// Optional emissive texture, sampled in sRGB.
    pub emissive_texture: Option<TextureData<'a>>,
    /// Linear emissive color factor.
    pub emissive_factor: [f32; 3],
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
    /// World-space spot lights. The renderer uses at most one shadow-casting light.
    pub spot_lights: &'a [SpotLight],
    /// Non-shadowed point lights; only the first eight are consumed.
    pub point_lights: &'a [PointLight],
    /// Exponential height/distance fog.
    pub fog: FogSettings,
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
    spot_view_proj: [[f32; 4]; 4],
    sun_dir: [f32; 4],
    sun_color: [f32; 4],
    ambient: [f32; 4],
    camera_pos: [f32; 4],
    spot_position_range: [f32; 4],
    spot_direction_outer: [f32; 4],
    spot_color_intensity: [f32; 4],
    spot_params: [f32; 4],
    point_positions_range: [[f32; 4]; 8],
    point_colors_intensity: [[f32; 4]; 8],
    point_count: [f32; 4],
    fog_color_density: [f32; 4],
    fog_params: [f32; 4],
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
    /// x: metallic, y: perceptual roughness, z: AO strength.
    surface: [f32; 4],
    emissive: [f32; 4],
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
    _normal_texture: wgpu::Texture,
    _occlusion_texture: wgpu::Texture,
    _emissive_texture: wgpu::Texture,
}

/// Owns the pipeline, uploaded meshes and materials, and the depth target.
pub struct MeshRenderer {
    pipeline: wgpu::RenderPipeline,
    shadow_pipeline: wgpu::RenderPipeline,
    spot_shadow_pipeline: wgpu::RenderPipeline,
    globals: wgpu::Buffer,
    globals_group: wgpu::BindGroup,
    shadow_globals: wgpu::Buffer,
    shadow_globals_group: wgpu::BindGroup,
    spot_shadow_globals: wgpu::Buffer,
    spot_shadow_globals_group: wgpu::BindGroup,
    shadow_target: ShadowTarget,
    spot_shadow_target: ShadowTarget,
    _shadow_sampler: wgpu::Sampler,
    material_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    meshes: Vec<Option<GpuMesh>>,
    free_mesh_slots: Vec<usize>,
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

fn pack_point_lights(lights: &[PointLight]) -> ([[f32; 4]; 8], [[f32; 4]; 8]) {
    let mut positions_range = [[0.0; 4]; 8];
    let mut colors_intensity = [[0.0; 4]; 8];
    for (index, light) in lights.iter().take(8).enumerate() {
        positions_range[index] = [
            light.position.x,
            light.position.y,
            light.position.z,
            light.range.max(0.0),
        ];
        colors_intensity[index] = [
            light.color.x,
            light.color.y,
            light.color.z,
            light.intensity.max(0.0),
        ];
    }
    (positions_range, colors_intensity)
}

fn select_spot_lights(lights: &[SpotLight]) -> (Option<SpotLight>, Option<SpotLight>) {
    let shadow = lights.iter().copied().find(|light| light.casts_shadows);
    let active = shadow.or_else(|| lights.first().copied());
    (active, shadow)
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
        // A spotlight commonly sits inside closed room geometry. Render both
        // sides into its depth map so inward-facing corridor surfaces still write
        // depth when viewed from inside the room. Keep the directional pipeline's
        // back-face culling and its tuned bias independent.
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
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&spot_shadow_target.view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
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
        let spot_shadow_globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("spotlight shadow globals"),
            size: std::mem::size_of::<ShadowGlobals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let spot_shadow_globals_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("spotlight shadow globals bind group"),
            layout: &shadow_globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: spot_shadow_globals.as_entire_binding(),
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
            spot_shadow_pipeline,
            globals,
            globals_group,
            shadow_globals,
            shadow_globals_group,
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
        let mesh = GpuMesh {
            vertices: vertex_buffer,
            indices: index_buffer,
            index_count: indices.len() as u32,
            bounds_center,
            bounds_radius,
        };
        if let Some(slot) = self.free_mesh_slots.pop() {
            self.meshes[slot] = Some(mesh);
            MeshId(slot)
        } else {
            self.meshes.push(Some(mesh));
            MeshId(self.meshes.len() - 1)
        }
    }

    /// Release a previously uploaded dynamic mesh. Built-in meshes are permanent, and
    /// released slots are reused by later uploads so repeated chunk edits do not grow the
    /// GPU mesh table without bound. A stale `MeshId` becomes invalid until that slot is reused.
    pub fn release_mesh(&mut self, mesh: MeshId) -> bool {
        if mesh == Self::CUBE {
            return false;
        }
        let Some(slot) = self.meshes.get_mut(mesh.0) else {
            return false;
        };
        if slot.take().is_some() {
            self.free_mesh_slots.push(mesh.0);
            true
        } else {
            false
        }
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
            normal_texture,
            occlusion_texture,
            occlusion_strength,
            emissive_texture,
            emissive_factor,
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
        let flat_normal = [128_u8, 128, 255, 255];
        let white_occlusion = [255_u8; 4];
        // With no emissive map, white leaves the glTF emissiveFactor unmodified.
        let white_emissive = [255_u8; 4];
        let normal_source = valid_texture(normal_texture, &flat_normal);
        let occlusion_source = valid_texture(occlusion_texture, &white_occlusion);
        let emissive_source = valid_texture(emissive_texture, &white_emissive);
        let (normal_gpu_texture, normal_view) =
            upload_rgba_texture(device, queue, normal_source, "material normal map", false);
        let (occlusion_gpu_texture, occlusion_view) = upload_rgba_texture(
            device,
            queue,
            occlusion_source,
            "material ambient occlusion map",
            false,
        );
        let (emissive_gpu_texture, emissive_view) = upload_rgba_texture(
            device,
            queue,
            emissive_source,
            "material emissive map",
            true,
        );
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
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&normal_view),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&occlusion_view),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(&emissive_view),
                },
            ],
        });
        self.materials.push(GpuMaterial {
            bind_group,
            _uniform: uniform,
            _texture: gpu_texture,
            _metallic_roughness_texture: surface_texture,
            _normal_texture: normal_gpu_texture,
            _occlusion_texture: occlusion_gpu_texture,
            _emissive_texture: emissive_gpu_texture,
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

    fn spot_shadow_matrix(light: SpotLight) -> Mat4 {
        let direction = light.direction.normalize_or_zero();
        let direction = if direction.length_squared() > 0.5 {
            direction
        } else {
            Vec3::NEG_Z
        };
        let up = if direction.dot(Vec3::Y).abs() > 0.95 {
            Vec3::Z
        } else {
            Vec3::Y
        };
        let half_angle = light.outer_cone_angle.clamp(0.05, 1.48);
        Mat4::perspective_rh(half_angle * 2.0, 1.0, 0.05, light.range.max(0.1))
            * Mat4::look_at_rh(light.position, light.position + direction, up)
    }

    /// Draw `scene` over the existing contents of `view`. Directional and (optional)
    /// spotlight shadow maps are rendered before the PBR color pass.
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
                self.meshes.get(draw.mesh.0).is_some_and(Option::is_some)
                    && draw.material.0 < self.materials.len()
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
                let Some(mesh) = self.meshes[draw.mesh.0].as_ref() else {
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
        let (active_spot, shadow_spot) = select_spot_lights(scene.spot_lights);
        let spot_view_proj = shadow_spot
            .map(Self::spot_shadow_matrix)
            .unwrap_or(Mat4::IDENTITY);
        let (point_positions_range, point_colors_intensity) = pack_point_lights(scene.point_lights);
        let spot = active_spot.unwrap_or_default();
        // max/min also maps NaN to the lower bound.
        #[allow(clippy::manual_clamp)]
        let outer_cone_angle = spot.outer_cone_angle.max(0.01).min(1.55);
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
                outer_cone_angle.cos(),
            ],
            spot_color_intensity: [
                spot.color.x,
                spot.color.y,
                spot.color.z,
                spot.intensity.max(0.0),
            ],
            spot_params: [
                spot.inner_cone_angle.max(0.0).min(outer_cone_angle).cos(),
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
                let mesh = self.meshes[mesh_id.0]
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
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.spot_shadow_pipeline);
            pass.set_bind_group(0, &self.spot_shadow_globals_group, &[]);
            pass.set_vertex_buffer(1, self.shadow_instances.slice(..));
            let mut start = 0;
            while start < draws.len() {
                let mesh_id = draws[start].mesh;
                let end = start
                    + draws[start..]
                        .iter()
                        .take_while(|draw| draw.mesh == mesh_id)
                        .count();
                let mesh = self.meshes[mesh_id.0]
                    .as_ref()
                    .expect("draw list was validated before recording spotlight shadows");
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
                let mesh = self.meshes[key.0 .0]
                    .as_ref()
                    .expect("draw list was validated before recording the mesh pass");
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
    use super::{
        build_mips, pack_point_lights, select_spot_lights, Draw, MeshRenderer, Scene, TextureData,
    };
    use crate::components::{Camera3D, FogSettings, PointLight, SpotLight, SunLight, Transform3D};
    use engine_render::gpu::GpuContext;
    use engine_render::gpu_window::clear_view;
    use glam::{Mat4, Vec3};
    use std::fs::File;
    use std::path::PathBuf;

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
                    spot_lights: &[],
                    point_lights: &[],
                    fog: FogSettings::default(),
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
    fn lighting_post_and_shadow_wgsl_shaders_validate_without_a_gpu_adapter() {
        let shaders = [
            include_str!("../shaders/mesh.wgsl"),
            include_str!("../shaders/shadow.wgsl"),
            include_str!("../shaders/bloom.wgsl"),
            include_str!("../shaders/tonemap.wgsl"),
            include_str!("../shaders/tonemap_unorm.wgsl"),
        ];
        for (index, source) in shaders.iter().enumerate() {
            let module = naga::front::wgsl::parse_str(source)
                .unwrap_or_else(|error| panic!("WGSL shader {index} failed to parse: {error:?}"));
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap_or_else(|error| panic!("WGSL shader {index} failed validation: {error:?}"));
        }
    }

    #[test]
    fn spotlight_selection_prefers_the_shadow_casting_light() {
        let non_shadow = SpotLight {
            position: Vec3::new(1.0, 2.0, 3.0),
            casts_shadows: false,
            ..SpotLight::default()
        };
        let shadow = SpotLight {
            position: Vec3::new(4.0, 5.0, 6.0),
            casts_shadows: true,
            ..SpotLight::default()
        };
        let (active, caster) = select_spot_lights(&[non_shadow, shadow]);
        assert_eq!(active, Some(shadow));
        assert_eq!(caster, Some(shadow));
        let (active, caster) = select_spot_lights(&[non_shadow]);
        assert_eq!(active, Some(non_shadow));
        assert_eq!(caster, None);
    }

    #[test]
    fn point_light_packing_caps_at_eight_and_clamps_negative_ranges() {
        let lights: Vec<_> = (0..10)
            .map(|index| PointLight {
                position: Vec3::new(index as f32, 2.0, 3.0),
                color: Vec3::new(0.1 * index as f32, 0.25, 0.75),
                intensity: if index == 3 { -4.0 } else { index as f32 },
                range: if index == 4 {
                    -2.0
                } else {
                    10.0 + index as f32
                },
            })
            .collect();
        let (positions, colors) = pack_point_lights(&lights);
        assert_eq!(positions.len(), 8);
        assert_eq!(positions[0], [0.0, 2.0, 3.0, 10.0]);
        assert_eq!(positions[4][3], 0.0);
        assert_eq!(positions[7][0], 7.0);
        assert_eq!(colors[3][3], 0.0);
        assert_eq!(colors[7][3], 7.0);
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
    fn spotlight_only_corridor_writes_a_headless_png() {
        const WIDTH: u32 = 160;
        const HEIGHT: u32 = 90;
        let context = match pollster::block_on(GpuContext::new_offscreen(WIDTH, HEIGHT)) {
            Ok(context) => context,
            Err(error) => {
                eprintln!(
                    "skipping spotlight screenshot test: no usable GPU adapter/device: {}",
                    error.0
                );
                return;
            }
        };
        let (device, queue) = (context.device(), context.queue());
        let mut renderer = MeshRenderer::new(device, queue, wgpu::TextureFormat::Rgba8Unorm);
        let cube_model = |position: Vec3, scale: Vec3| {
            Mat4::from_scale_rotation_translation(scale, glam::Quat::IDENTITY, position)
        };
        let draws = [
            cube(
                cube_model(Vec3::new(0.0, -0.12, -0.6), Vec3::new(8.0, 0.24, 14.0)),
                [0.66, 0.69, 0.65],
            ),
            cube(
                cube_model(Vec3::new(-3.9, 1.5, -0.6), Vec3::new(0.2, 3.0, 14.0)),
                [0.34, 0.40, 0.35],
            ),
            cube(
                cube_model(Vec3::new(3.9, 1.5, -0.6), Vec3::new(0.2, 3.0, 14.0)),
                [0.34, 0.40, 0.35],
            ),
            cube(
                cube_model(Vec3::new(0.0, 1.5, -7.3), Vec3::new(8.0, 3.0, 0.2)),
                [0.30, 0.35, 0.31],
            ),
            cube(
                // Move the occluder off the camera centerline so its floor shadow
                // remains visible instead of being hidden behind the blocking cube.
                cube_model(Vec3::new(-1.2, 0.75, -2.0), Vec3::new(1.1, 1.5, 0.9)),
                [0.08, 0.10, 0.08],
            ),
        ];
        let camera = Transform3D::looking_at(
            Vec3::new(0.0, 2.05, 4.2),
            Vec3::new(0.0, 0.65, -2.3),
            Vec3::Y,
        );
        let view = Mat4::from_rotation_translation(camera.rotation, camera.translation).inverse();
        let spotlight = SpotLight {
            // Place the test light close enough to the central blocker that its
            // floor shadow receives enough linear-light energy to survive RGBA8
            // quantization in this headless regression target.
            position: Vec3::new(0.0, 2.0, 1.5),
            // Aim the cone through the off-center blocker so its floor shadow is
            // illuminated strongly enough to be measured by the regression test.
            direction: Vec3::new(-0.31, -0.32, -0.91).normalize(),
            color: Vec3::new(0.82, 0.93, 0.80),
            // Keep the offscreen test signal above RGBA8 quantization and cover
            // the visible floor-shadow region on the side of the occluder.
            intensity: 320.0,
            range: 13.0,
            inner_cone_angle: 16.0_f32.to_radians(),
            outer_cone_angle: 32.0_f32.to_radians(),
            casts_shadows: true,
        };
        let render_corridor = |renderer: &mut MeshRenderer, spot: SpotLight| {
            clear_view(
                device,
                queue,
                context.target().view(),
                [0.002, 0.003, 0.003, 1.0],
            );
            renderer.render(
                device,
                queue,
                context.target().view(),
                (WIDTH, HEIGHT),
                &Scene {
                    view_proj: Camera3D {
                        fov_y: 70.0_f32.to_radians(),
                        near: 0.05,
                        far: 24.0,
                    }
                    .projection(WIDTH as f32 / HEIGHT as f32)
                        * view,
                    camera_position: camera.translation,
                    sun: &SunLight {
                        direction: Vec3::NEG_Y,
                        color: Vec3::ZERO,
                        ambient: Vec3::splat(0.0005),
                    },
                    spot_lights: std::slice::from_ref(&spot),
                    point_lights: &[],
                    fog: FogSettings {
                        color: Vec3::new(0.015, 0.023, 0.020),
                        density: 0.035,
                        base_height: 0.0,
                        height_falloff: 0.08,
                    },
                    draws: &draws,
                },
            );
            context
                .readback()
                .expect("read back spotlight corridor")
                .pixels
        };
        let framebuffer_pixels = render_corridor(&mut renderer, spotlight);
        // A second frame with the exact same light and geometry but disabled shadows
        // makes this a regression check for the shadow map, not merely a dark-corner check.
        let unshadowed_pixels = render_corridor(
            &mut renderer,
            SpotLight {
                casts_shadows: false,
                ..spotlight
            },
        );
        let mut shadow_changed_pixels = 0usize;
        let mut shadow_difference_energy = 0_u64;
        for (shadowed, unshadowed) in framebuffer_pixels.iter().zip(&unshadowed_pixels) {
            let shadowed = *shadowed;
            let unshadowed = *unshadowed;
            let shadowed_rgb = (
                (shadowed >> 16) & 0xff,
                (shadowed >> 8) & 0xff,
                shadowed & 0xff,
            );
            let unshadowed_rgb = (
                (unshadowed >> 16) & 0xff,
                (unshadowed >> 8) & 0xff,
                unshadowed & 0xff,
            );
            let difference = shadowed_rgb.0.abs_diff(unshadowed_rgb.0)
                + shadowed_rgb.1.abs_diff(unshadowed_rgb.1)
                + shadowed_rgb.2.abs_diff(unshadowed_rgb.2);
            shadow_difference_energy += u64::from(difference);
            if difference > 12 {
                shadow_changed_pixels += 1;
            }
        }
        let mut bright_pixels = 0usize;
        let mut dark_pixels = 0usize;
        let mut rgba = Vec::with_capacity(framebuffer_pixels.len() * 4);
        for packed_pixel in framebuffer_pixels {
            let r = ((packed_pixel >> 16) & 0xff) as u8;
            let g = ((packed_pixel >> 8) & 0xff) as u8;
            let b = (packed_pixel & 0xff) as u8;
            let luminance = (u32::from(r) * 3 + u32::from(g) * 6 + u32::from(b)) / 10;
            if luminance > 24 {
                bright_pixels += 1;
            }
            if luminance < 6 {
                dark_pixels += 1;
            }
            rgba.extend_from_slice(&[r, g, b, 255]);
        }
        let output =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/spotlight_corridor.png");
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent).expect("create target directory for screenshot");
        }
        let file = File::create(&output).expect("create spotlight screenshot PNG");
        let mut encoder = png::Encoder::new(file, WIDTH, HEIGHT);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("write PNG header");
        writer
            .write_image_data(&rgba)
            .expect("write screenshot image data");
        eprintln!("spotlight corridor screenshot: {}", output.display());

        // Persist the screenshot before asserting so a failing GPU regression still leaves
        // an inspectable artifact in target/ on the developer machine.
        assert!(
            shadow_changed_pixels > 16 && shadow_difference_energy > 800,
            "spotlight shadow map should darken a measurable corridor region; changed pixels={shadow_changed_pixels}, difference energy={shadow_difference_energy}; inspect {}",
            output.display()
        );
        assert!(
            bright_pixels > 80,
            "spotlight-only corridor should contain a lit region; bright pixels={bright_pixels}; inspect {}",
            output.display()
        );
        assert!(
            dark_pixels > 80,
            "spotlight-only corridor should retain deep shadow; dark pixels={dark_pixels}; inspect {}",
            output.display()
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
