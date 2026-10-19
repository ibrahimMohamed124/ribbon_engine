//! wgpu pipeline that draws instanced, textured meshes with a depth buffer.

use crate::components::SunLight;
use crate::mesh::{self, Vertex};
use bytemuck::{Pod, Zeroable};
use glam::Mat4;
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
    sun_dir: [f32; 4],
    sun_color: [f32; 4],
    ambient: [f32; 4],
}

struct DepthTarget {
    view: wgpu::TextureView,
    size: (u32, u32),
}

struct GpuMesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
}

struct GpuMaterial {
    bind_group: wgpu::BindGroup,
    _uniform: wgpu::Buffer,
    _texture: wgpu::Texture,
}

/// Owns the pipeline, uploaded meshes and materials, and the depth target.
pub struct MeshRenderer {
    pipeline: wgpu::RenderPipeline,
    globals: wgpu::Buffer,
    globals_group: wgpu::BindGroup,
    material_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    meshes: Vec<GpuMesh>,
    materials: Vec<GpuMaterial>,
    instances: wgpu::Buffer,
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
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mesh globals layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
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
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mesh pipeline layout"),
            bind_group_layouts: &[&globals_layout, &material_layout],
            push_constant_ranges: &[],
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
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
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
            globals,
            globals_group,
            material_layout,
            sampler,
            meshes: Vec::new(),
            materials: Vec::new(),
            instances: instance_buffer(device, 16),
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
        self.meshes.push(GpuMesh {
            vertices: vertex_buffer,
            indices: index_buffer,
            index_count: indices.len() as u32,
        });
        MeshId(self.meshes.len() - 1)
    }

    /// Upload a material: a linear color factor and an optional sRGB base color texture.
    pub fn upload_material(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        base_color: [f32; 4],
        texture: Option<TextureData<'_>>,
    ) -> MaterialId {
        let white = [255_u8; 4];
        let source = match texture {
            Some(t)
                if t.width > 0
                    && t.height > 0
                    && t.rgba8.len() == (t.width * t.height * 4) as usize =>
            {
                t
            }
            _ => TextureData {
                width: 1,
                height: 1,
                rgba8: &white,
            },
        };
        let mips = build_mips(source.width, source.height, source.rgba8);
        let gpu_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("material base color"),
            size: wgpu::Extent3d {
                width: source.width,
                height: source.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: mips.len() as u32,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (level, (w, h, pixels)) in mips.iter().enumerate() {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &gpu_texture,
                    mip_level: level as u32,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(w * 4),
                    rows_per_image: Some(*h),
                },
                wgpu::Extent3d {
                    width: *w,
                    height: *h,
                    depth_or_array_layers: 1,
                },
            );
        }
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("material factors"),
            contents: bytemuck::bytes_of(&base_color),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let view = gpu_texture.create_view(&wgpu::TextureViewDescriptor::default());
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
            ],
        });
        self.materials.push(GpuMaterial {
            bind_group,
            _uniform: uniform,
            _texture: gpu_texture,
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

    /// Draw `scene` over the existing contents of `view`.
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
        let mut draws: Vec<Draw> = scene
            .draws
            .iter()
            .copied()
            .filter(|d| d.mesh.0 < self.meshes.len() && d.material.0 < self.materials.len())
            .collect();
        draws.sort_by_key(|d| (d.mesh, d.material));
        if draws.is_empty() {
            return;
        }
        if draws.len() > self.instance_capacity {
            self.instance_capacity = draws.len().next_power_of_two();
            self.instances = instance_buffer(device, self.instance_capacity);
        }
        let raw: Vec<RawInstance> = draws
            .iter()
            .map(|d| RawInstance {
                model: d.model.to_cols_array_2d(),
                tint: [d.tint[0], d.tint[1], d.tint[2], 1.0],
            })
            .collect();
        queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(&raw));
        let globals = Globals {
            view_proj: scene.view_proj.to_cols_array_2d(),
            sun_dir: scene
                .sun
                .direction
                .normalize_or_zero()
                .extend(0.0)
                .to_array(),
            sun_color: scene.sun.color.extend(1.0).to_array(),
            ambient: scene.sun.ambient.extend(1.0).to_array(),
        };
        queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&globals));
        let Some(depth) = self.depth.as_ref() else {
            return;
        };
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("mesh pass"),
        });
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
            while start < draws.len() {
                let key = (draws[start].mesh, draws[start].material);
                let end = start
                    + draws[start..]
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
    fn cube_is_drawn_in_the_center_and_background_is_untouched() {
        let Some(mut h) = harness() else { return };
        let pixels = h.draw(&[cube(Mat4::IDENTITY, [1.0, 0.0, 0.0])]);
        let (r, g, b) = pixel(&pixels, SIZE / 2, SIZE / 2);
        assert!(r > 0 && g == 0 && b == 0, "center pixel was {r},{g},{b}");
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
            r > 0 && b == 0,
            "expected the near red cube, got r={r} b={b}"
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
