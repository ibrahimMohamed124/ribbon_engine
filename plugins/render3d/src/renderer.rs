//! wgpu pipeline that draws instanced meshes with a depth buffer.

use crate::components::SunLight;
use crate::mesh::{self, Vertex};
use bytemuck::{Pod, Zeroable};
use glam::Mat4;
use wgpu::util::DeviceExt;

/// Depth format used by the mesh pass.
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// One object to draw: model matrix and linear RGB color.
#[derive(Clone, Copy, Debug)]
pub struct InstanceData {
    /// Object-to-world matrix.
    pub model: Mat4,
    /// Linear RGB base color.
    pub color: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct RawInstance {
    model: [[f32; 4]; 4],
    color: [f32; 4],
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

/// Everything one draw call needs besides the GPU handles.
pub struct Scene<'a> {
    /// Combined projection and view matrix.
    pub view_proj: Mat4,
    /// Directional light and ambient term.
    pub sun: &'a SunLight,
    /// Objects to draw.
    pub instances: &'a [InstanceData],
}

/// Owns the pipeline, mesh buffers and depth target for one color format.
pub struct MeshRenderer {
    pipeline: wgpu::RenderPipeline,
    globals: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    instances: wgpu::Buffer,
    instance_capacity: usize,
    depth: Option<DepthTarget>,
}

const VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 2] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];
const INSTANCE_ATTRIBUTES: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
    2 => Float32x4, 3 => Float32x4, 4 => Float32x4, 5 => Float32x4, 6 => Float32x4
];

fn instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("mesh instances"),
        size: (capacity * std::mem::size_of::<RawInstance>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

impl MeshRenderer {
    /// Build the pipeline for render targets of `format`.
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("mesh shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/mesh.wgsl").into()),
        });
        let layout_entry = wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mesh globals layout"),
            entries: &[layout_entry],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mesh pipeline layout"),
            bind_group_layouts: &[&bind_layout],
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
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mesh globals bind group"),
            layout: &bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            }],
        });
        let cube = mesh::cube();
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("cube vertices"),
            contents: bytemuck::cast_slice(&cube.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("cube indices"),
            contents: bytemuck::cast_slice(&cube.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        Self {
            pipeline,
            globals,
            bind_group,
            vertices,
            indices,
            index_count: cube.indices.len() as u32,
            instances: instance_buffer(device, 16),
            instance_capacity: 16,
            depth: None,
        }
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

    /// Draw `instances` (all cubes for now) over the existing contents of `view`.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        view: &wgpu::TextureView,
        size: (u32, u32),
        scene: &Scene<'_>,
    ) {
        let Scene {
            view_proj,
            sun,
            instances,
        } = *scene;
        if instances.is_empty() || size.0 == 0 || size.1 == 0 {
            return;
        }
        self.ensure_depth(device, size);
        if instances.len() > self.instance_capacity {
            self.instance_capacity = instances.len().next_power_of_two();
            self.instances = instance_buffer(device, self.instance_capacity);
        }
        let raw: Vec<RawInstance> = instances
            .iter()
            .map(|instance| RawInstance {
                model: instance.model.to_cols_array_2d(),
                color: [instance.color[0], instance.color[1], instance.color[2], 1.0],
            })
            .collect();
        queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(&raw));
        let globals = Globals {
            view_proj: view_proj.to_cols_array_2d(),
            sun_dir: sun.direction.normalize_or_zero().extend(0.0).to_array(),
            sun_color: sun.color.extend(1.0).to_array(),
            ambient: sun.ambient.extend(1.0).to_array(),
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
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, self.vertices.slice(..));
            pass.set_vertex_buffer(1, self.instances.slice(..));
            pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint16);
            pass.draw_indexed(0..self.index_count, 0, 0..instances.len() as u32);
        }
        queue.submit(Some(encoder.finish()));
    }
}

#[cfg(test)]
mod tests {
    use super::{InstanceData, MeshRenderer, Scene};
    use crate::components::{Camera3D, SunLight, Transform3D};
    use engine_render::gpu::GpuContext;
    use engine_render::gpu_window::clear_view;
    use glam::{Mat4, Vec3};

    const SIZE: u32 = 64;

    fn render_scene(instances: &[InstanceData]) -> Option<Vec<u32>> {
        let context = match pollster::block_on(GpuContext::new_offscreen(SIZE, SIZE)) {
            Ok(context) => context,
            Err(error) => {
                eprintln!("skipping GPU test: {}", error.0);
                return None;
            }
        };
        let eye = Transform3D::looking_at(Vec3::new(0.0, 0.0, 4.0), Vec3::ZERO, Vec3::Y);
        let view = Mat4::from_rotation_translation(eye.rotation, eye.translation).inverse();
        let view_proj = Camera3D::default().projection(1.0) * view;
        clear_view(
            context.device(),
            context.queue(),
            context.target().view(),
            [0.0, 0.0, 0.0, 1.0],
        );
        let mut renderer = MeshRenderer::new(context.device(), wgpu::TextureFormat::Rgba8Unorm);
        renderer.render(
            context.device(),
            context.queue(),
            context.target().view(),
            (SIZE, SIZE),
            &Scene {
                view_proj,
                sun: &SunLight::default(),
                instances,
            },
        );
        Some(context.readback().expect("readback").pixels)
    }

    fn pixel(pixels: &[u32], x: u32, y: u32) -> (u32, u32, u32) {
        let p = pixels[(y * SIZE + x) as usize];
        ((p >> 16) & 0xff, (p >> 8) & 0xff, p & 0xff)
    }

    #[test]
    fn cube_is_drawn_in_the_center_and_background_is_untouched() {
        let Some(pixels) = render_scene(&[InstanceData {
            model: Mat4::IDENTITY,
            color: [1.0, 0.0, 0.0],
        }]) else {
            return;
        };
        let (r, g, b) = pixel(&pixels, SIZE / 2, SIZE / 2);
        assert!(r > 0 && g == 0 && b == 0, "center pixel was {r},{g},{b}");
        assert_eq!(pixel(&pixels, 0, 0), (0, 0, 0));
    }

    #[test]
    fn depth_test_keeps_the_nearer_cube_in_front() {
        let near_red = InstanceData {
            model: Mat4::IDENTITY,
            color: [1.0, 0.0, 0.0],
        };
        let far_blue = InstanceData {
            model: Mat4::from_translation(Vec3::new(0.0, 0.0, -3.0)),
            color: [0.0, 0.0, 1.0],
        };
        // The far cube is submitted last; without a depth test it would overwrite the red one.
        let Some(pixels) = render_scene(&[near_red, far_blue]) else {
            return;
        };
        let (r, _, b) = pixel(&pixels, SIZE / 2, SIZE / 2);
        assert!(
            r > 0 && b == 0,
            "expected the near red cube, got r={r} b={b}"
        );
    }
}
