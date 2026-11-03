//! GPU upload and draw path for renderer-neutral particle batches.
use super::DEPTH_FORMAT;
use bytemuck::{Pod, Zeroable};
use glam::{Quat, Vec3};
use ribbon_render_contracts::{
    ParticleBatch, ParticleBillboard, ParticleBlendMode, ParticleInstance, ParticleShape,
};
use std::ops::Range;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuParticle {
    position: [f32; 3],
    rotation: f32,
    velocity: [f32; 3],
    emissive: f32,
    size: [f32; 2],
    shape_billboard: [f32; 2],
    color: [f32; 4],
}
const PARTICLE_ATTRIBUTES: [wgpu::VertexAttribute; 7] = wgpu::vertex_attr_array![0=>Float32x3,1=>Float32,2=>Float32x3,3=>Float32,4=>Float32x2,5=>Float32x2,6=>Float32x4];
struct ParticleDrawBatch {
    blend: ParticleBlendMode,
    instances: Range<u32>,
}

pub(super) struct ParticleGpu {
    buffer: wgpu::Buffer,
    capacity: usize,
    alpha: wgpu::RenderPipeline,
    additive: wgpu::RenderPipeline,
    camera_buffer: wgpu::Buffer,
    camera_group: wgpu::BindGroup,
    packed: Vec<GpuParticle>,
    batches: Vec<ParticleDrawBatch>,
}
impl ParticleGpu {
    pub(super) fn new(
        device: &wgpu::Device,
        globals_layout: &wgpu::BindGroupLayout,
        format: wgpu::TextureFormat,
        sample_count: u32,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("particle shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/particles.wgsl").into()),
        });
        let camera_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("particle camera basis layout"),
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
        let camera_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("particle camera basis"),
            size: 32,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let camera_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("particle camera basis group"),
            layout: &camera_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("particle pipeline layout"),
            bind_group_layouts: &[globals_layout, &camera_layout],
            push_constant_ranges: &[],
        });
        let create = |blend: wgpu::BlendState, label: &'static str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<GpuParticle>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &PARTICLE_ATTRIBUTES,
                    }],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: false,
                    depth_compare: wgpu::CompareFunction::LessEqual,
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: sample_count,
                    ..Default::default()
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(blend),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview: None,
                cache: None,
            })
        };
        let alpha = create(wgpu::BlendState::ALPHA_BLENDING, "alpha particle pipeline");
        let additive = create(
            wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::SrcAlpha,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent::OVER,
            },
            "additive particle pipeline",
        );
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("particle instance buffer"),
            size: 64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            buffer,
            capacity: 1,
            alpha,
            additive,
            camera_buffer,
            camera_group,
            packed: Vec::new(),
            batches: Vec::new(),
        }
    }
    pub(super) fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        batches: &[ParticleBatch],
        camera: Vec3,
        camera_rotation: Quat,
    ) -> (usize, usize) {
        // Derive an orthonormal world-space camera basis from the actual camera pose.
        // Inverting view-projection incorrectly mixes projection scale into billboard axes.
        let [right, up] = camera_basis(camera_rotation);
        // WGSL stores each basis vector as vec4 (16-byte alignment). Upload
        // explicit padding instead of tightly packed vec3 arrays (24 bytes).
        let basis = [right.extend(0.0).to_array(), up.extend(0.0).to_array()];
        queue.write_buffer(&self.camera_buffer, 0, bytemuck::cast_slice(&basis));
        self.packed.clear();
        self.batches.clear();
        for batch in batches {
            let start = self.packed.len();
            if batch.blend == ParticleBlendMode::Alpha
                && batch.sort_back_to_front
                && batch.instances.len() <= 20_000
            {
                let mut sorted = batch.instances.clone();
                sorted.sort_by(|a, b| distance_sq(b, camera).total_cmp(&distance_sq(a, camera)));
                self.packed.extend(
                    sorted
                        .iter()
                        .map(|p| pack(*p, batch.shape, batch.billboard)),
                );
            } else {
                self.packed.extend(
                    batch
                        .instances
                        .iter()
                        .map(|p| pack(*p, batch.shape, batch.billboard)),
                );
            }
            let end = self.packed.len();
            if end > start {
                self.batches.push(ParticleDrawBatch {
                    blend: batch.blend,
                    instances: start as u32..end as u32,
                });
            }
        }
        if self.packed.len() > self.capacity {
            self.capacity = self.packed.len().next_power_of_two().max(1);
            self.buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("particle instance buffer"),
                size: (self.capacity * 64) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        if !self.packed.is_empty() {
            queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(&self.packed));
        }
        (self.packed.len(), self.batches.len())
    }
    pub(super) fn draw<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        globals: &'a wgpu::BindGroup,
    ) {
        if self.packed.is_empty() {
            return;
        }
        pass.set_bind_group(0, globals, &[]);
        pass.set_bind_group(1, &self.camera_group, &[]);
        pass.set_vertex_buffer(0, self.buffer.slice(..));
        for batch in &self.batches {
            pass.set_pipeline(match batch.blend {
                ParticleBlendMode::Alpha => &self.alpha,
                ParticleBlendMode::Additive => &self.additive,
            });
            pass.draw(0..6, batch.instances.clone());
        }
    }
}

/// Returns an orthonormal world-space billboard basis from the camera's world rotation.
fn camera_basis(camera_rotation: Quat) -> [Vec3; 2] {
    let rotation = if camera_rotation.is_finite() && camera_rotation.length_squared() > 1.0e-12 {
        camera_rotation.normalize()
    } else {
        Quat::IDENTITY
    };
    [
        (rotation * Vec3::X).normalize_or_zero(),
        (rotation * Vec3::Y).normalize_or_zero(),
    ]
}

/// CPU mirror of the WGSL formula, used to test all projected velocity directions.
#[cfg(test)]
fn velocity_stretch_rotation(projected_velocity: Vec3, right: Vec3, up: Vec3) -> Option<f32> {
    let x = projected_velocity.dot(right);
    let y = projected_velocity.dot(up);
    (x.is_finite() && y.is_finite() && x * x + y * y > 1.0e-10).then(|| (-x).atan2(y))
}

fn distance_sq(p: &ParticleInstance, camera: Vec3) -> f32 {
    let p = Vec3::from_array(p.position);
    (p - camera).length_squared()
}
fn pack(i: ParticleInstance, shape: ParticleShape, billboard: ParticleBillboard) -> GpuParticle {
    let shape = match shape {
        ParticleShape::SoftDisc => 0.0,
        ParticleShape::Streak => 1.0,
        ParticleShape::Square => 2.0,
    };
    let stretch = match billboard {
        ParticleBillboard::Camera => -1.0,
        ParticleBillboard::VelocityStretched { stretch } => stretch.max(0.0),
    };
    GpuParticle {
        position: i.position,
        rotation: i.rotation,
        velocity: i.velocity,
        emissive: i.emissive.max(0.0),
        size: i.size,
        shape_billboard: [shape, stretch],
        color: i.color,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn packed_particle_is_64_bytes() {
        assert_eq!(std::mem::size_of::<GpuParticle>(), 64)
    }
    #[test]
    fn camera_uniform_matches_wgsl_vec4_alignment() {
        let [right, up] = camera_basis(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2));
        let uniform = [right.extend(0.0).to_array(), up.extend(0.0).to_array()];
        assert_eq!(std::mem::size_of_val(&uniform), 32);
        assert_eq!(uniform[0][3], 0.0);
        assert_eq!(uniform[1][3], 0.0);
        assert!(uniform[0][0].abs() < 1.0e-6);
        assert!((uniform[1][0] + 1.0).abs() < 1.0e-6);
    }

    #[test]
    fn camera_billboard_basis_uses_camera_rotation_not_projection() {
        let rotation = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        let [right, up] = camera_basis(rotation);
        assert!(right.abs_diff_eq(Vec3::NEG_Z, 1.0e-6));
        assert!(up.abs_diff_eq(Vec3::Y, 1.0e-6));
        assert!((right.length() - 1.0).abs() < 1.0e-6);
        assert!((up.length() - 1.0).abs() < 1.0e-6);
        assert!(right.dot(up).abs() < 1.0e-6);
    }

    #[test]
    fn velocity_stretched_rotation_aligns_the_long_axis_with_screen_velocity() {
        let right = Vec3::X;
        let up = Vec3::Y;
        let cases = [
            (Vec3::X, -std::f32::consts::FRAC_PI_2),
            (Vec3::NEG_X, std::f32::consts::FRAC_PI_2),
            (Vec3::Y, 0.0),
            (Vec3::NEG_Y, std::f32::consts::PI),
        ];
        for (velocity, expected_angle) in cases {
            let angle = velocity_stretch_rotation(velocity, right, up)
                .expect("nonzero projected velocity has a stretch rotation");
            let long_axis = up * angle.cos() - right * angle.sin();
            assert!(long_axis.dot(velocity.normalize()) > 1.0 - 1.0e-6);
            assert!(
                (angle - expected_angle).abs() < 1.0e-6
                    || (angle - expected_angle).abs() > std::f32::consts::TAU - 1.0e-6
            );
        }
        assert_eq!(velocity_stretch_rotation(Vec3::Z, right, up), None);
    }

    #[test]
    fn invalid_camera_rotation_uses_finite_identity_basis() {
        let [right, up] = camera_basis(Quat::from_xyzw(f32::NAN, 0.0, 0.0, 1.0));
        assert_eq!(right, Vec3::X);
        assert_eq!(up, Vec3::Y);
    }

    #[test]
    fn alpha_sort_is_back_to_front() {
        let camera = Vec3::ZERO;
        let near = ParticleInstance {
            position: [1.0, 0.0, 0.0],
            ..Default::default()
        };
        let far = ParticleInstance {
            position: [5.0, 0.0, 0.0],
            ..Default::default()
        };
        let batch = ParticleBatch {
            blend: ParticleBlendMode::Alpha,
            shape: ParticleShape::Square,
            billboard: ParticleBillboard::Camera,
            sort_back_to_front: true,
            instances: vec![near, far],
        };
        let mut packed = [
            pack(batch.instances[0], batch.shape, batch.billboard),
            pack(batch.instances[1], batch.shape, batch.billboard),
        ];
        packed.sort_by(|a, b| {
            let aa = (Vec3::from_array(a.position) - camera).length_squared();
            let bb = (Vec3::from_array(b.position) - camera).length_squared();
            bb.total_cmp(&aa)
        });
        assert_eq!(packed[0].position, far.position);
    }
}
