//! Window overlay render pass.
use ribbon_render::gpu_window::{FramePass, FrameTarget};
#[derive(Default)]
pub struct UiPass {
    pipeline: Option<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
    buffer: Option<wgpu::Buffer>,
    capacity: u64,
}
impl FramePass for UiPass {
    fn name(&self) -> &'static str {
        "ribbon ui"
    }
    fn on_device_lost(&mut self, _: &mut ribbon_app::World) {
        self.pipeline = None;
        self.buffer = None;
        self.capacity = 0;
    }
    fn render(&mut self, world: &mut ribbon_app::World, t: &FrameTarget<'_>) {
        let Some(ctx) = world.try_resource::<crate::UiContext>() else {
            return;
        };
        if ctx.draw.vertices.is_empty() {
            return;
        }
        if self.pipeline.as_ref().is_none_or(|(f, _)| *f != t.format) {
            let shader = t.device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("ribbon ui shader"),
                source: wgpu::ShaderSource::Wgsl(include_str!("ui.wgsl").into()),
            });
            let layout = t
                .device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: None,
                    bind_group_layouts: &[],
                    push_constant_ranges: &[],
                });
            let pipe = t
                .device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("ribbon ui"),
                    layout: Some(&layout),
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some("vs_main"),
                        compilation_options: Default::default(),
                        buffers: &[wgpu::VertexBufferLayout {
                            array_stride: 24,
                            step_mode: wgpu::VertexStepMode::Vertex,
                            attributes: &wgpu::vertex_attr_array![0=>Float32x2,1=>Float32x4],
                        }],
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some("fs_main"),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: t.format,
                            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    primitive: Default::default(),
                    depth_stencil: None,
                    multisample: Default::default(),
                    multiview: None,
                    cache: None,
                });
            self.pipeline = Some((t.format, pipe));
        }
        let bytes = bytemuck::cast_slice(&ctx.draw.vertices);
        let need = bytes.len() as u64;
        if need > self.capacity {
            self.capacity = need.next_power_of_two();
            self.buffer = Some(t.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("ui vertices"),
                size: self.capacity,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        let Some(buf) = self.buffer.as_ref() else {
            return;
        };
        t.queue.write_buffer(buf, 0, bytes);
        let mut enc = t.device.create_command_encoder(&Default::default());
        {
            let mut p = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ui overlay"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: t.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });
            let Some((_, pipe)) = self.pipeline.as_ref() else {
                return;
            };
            p.set_pipeline(pipe);
            p.set_vertex_buffer(0, buf.slice(..));
            p.draw(0..ctx.draw.vertices.len() as u32, 0..1);
        }
        t.queue.submit(Some(enc.finish()));
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn ui_shader_is_valid_wgsl() {
        let module =
            naga::front::wgsl::parse_str(include_str!("ui.wgsl")).expect("parse UI shader");
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("validate UI shader");
    }
}
