//! Lightweight, pixel-positioned text overlays for native GPU windows.

use super::{FramePass, FrameTarget};
use ribbon_app::World;

/// Small, pixel-positioned text panel drawn after the scene by [`WindowTextOverlayPass`].
///
/// This deliberately small API supports diagnostic panels and control guides without introducing
/// a full UI framework. Text uses the engine's shared 5x7 bitmap font and is clipped to the
/// current surface size. Lines are rendered in ASCII; unsupported characters use the replacement
/// glyph from the shared font.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WindowTextOverlay {
    /// Lines rendered in order, starting with a highlighted heading.
    pub lines: Vec<String>,
    /// When false, the panel is not drawn.
    pub visible: bool,
}

/// Render [`WindowTextOverlay`] over the current window image.
///
/// Add this pass after the scene renderer has registered its pass. The panel is alpha blended,
/// preserves the preceding color attachment, and recreates its pipeline if the surface format
/// changes.
#[derive(Default)]
pub struct WindowTextOverlayPass {
    pipeline: Option<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
    vertex_buffer: Option<wgpu::Buffer>,
    vertex_capacity: u64,
}

const VERTEX_STRIDE: u64 = 24;
const TEXT_SCALE: i32 = 2;
const GLYPH_ADVANCE: i32 = 6 * TEXT_SCALE;
const LINE_HEIGHT: i32 = 9 * TEXT_SCALE;

fn push_vertex(bytes: &mut Vec<u8>, x: f32, y: f32, color: [f32; 4], size: (u32, u32)) {
    let clip_x = (x / size.0 as f32) * 2.0 - 1.0;
    let clip_y = 1.0 - (y / size.1 as f32) * 2.0;
    for value in [clip_x, clip_y, color[0], color[1], color[2], color[3]] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
}

fn push_rect(bytes: &mut Vec<u8>, rect: (i32, i32, i32, i32), color: [f32; 4], size: (u32, u32)) {
    let (width, height) = (size.0 as i32, size.1 as i32);
    if width <= 0 || height <= 0 {
        return;
    }
    let (x, y, w, h) = rect;
    let x0 = x.clamp(0, width);
    let y0 = y.clamp(0, height);
    let x1 = x.saturating_add(w).clamp(0, width);
    let y1 = y.saturating_add(h).clamp(0, height);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    for (px, py) in [
        (x0 as f32, y0 as f32),
        (x1 as f32, y0 as f32),
        (x1 as f32, y1 as f32),
        (x0 as f32, y0 as f32),
        (x1 as f32, y1 as f32),
        (x0 as f32, y1 as f32),
    ] {
        push_vertex(bytes, px, py, color, size);
    }
}

fn build_vertices(lines: &[String], size: (u32, u32)) -> Vec<u8> {
    if lines.is_empty() || size.0 == 0 || size.1 == 0 {
        return Vec::new();
    }

    let max_columns = ((size.0.saturating_sub(32) as usize) / GLYPH_ADVANCE as usize).clamp(1, 56);
    let max_lines = ((size.1.saturating_sub(24) / LINE_HEIGHT as u32).max(1) as usize)
        .min(12)
        .min(lines.len());
    let displayed = &lines[..max_lines];
    let text_columns = displayed
        .iter()
        .map(|line| line.chars().take(max_columns).count())
        .max()
        .unwrap_or(0)
        .max(1);
    let requested_width = text_columns
        .saturating_mul(GLYPH_ADVANCE as usize)
        .saturating_add(20)
        .min(u32::MAX as usize) as u32;
    let panel_x = size.0.min(8);
    let panel_y = size.1.min(8);
    let panel_width = requested_width.min(size.0.saturating_sub(panel_x)).max(1);
    let requested_height = (max_lines as u32)
        .saturating_mul(LINE_HEIGHT as u32)
        .saturating_add(16);
    let panel_height = requested_height.min(size.1.saturating_sub(panel_y)).max(1);

    let mut bytes = Vec::new();
    push_rect(
        &mut bytes,
        (
            panel_x as i32,
            panel_y as i32,
            panel_width as i32,
            panel_height as i32,
        ),
        [0.015, 0.024, 0.040, 0.90],
        size,
    );
    push_rect(
        &mut bytes,
        (panel_x as i32, panel_y as i32, 3, panel_height as i32),
        [0.18, 0.78, 0.95, 1.0],
        size,
    );

    let text_x = panel_x.saturating_add(10) as i32;
    let mut text_y = panel_y.saturating_add(8) as i32;
    for (line_index, line) in displayed.iter().enumerate() {
        let color = if line_index == 0 {
            [0.42, 0.88, 1.0, 1.0]
        } else {
            [0.91, 0.94, 0.98, 1.0]
        };
        let mut x = text_x;
        for ch in line.chars().take(max_columns) {
            for (row, bits) in crate::frame::glyph(ch.to_ascii_uppercase())
                .into_iter()
                .enumerate()
            {
                for column in 0..5 {
                    if bits & (1 << (4 - column)) != 0 {
                        push_rect(
                            &mut bytes,
                            (
                                x + column * TEXT_SCALE,
                                text_y + row as i32 * TEXT_SCALE,
                                TEXT_SCALE,
                                TEXT_SCALE,
                            ),
                            color,
                            size,
                        );
                    }
                }
            }
            x += GLYPH_ADVANCE;
        }
        text_y = text_y.saturating_add(LINE_HEIGHT);
    }
    bytes
}

impl FramePass for WindowTextOverlayPass {
    fn name(&self) -> &'static str {
        "window text overlay"
    }

    fn render(&mut self, world: &mut World, target: &FrameTarget<'_>) {
        let Some(overlay) = world.try_resource::<WindowTextOverlay>() else {
            return;
        };
        if !overlay.visible || overlay.lines.is_empty() || target.size.0 == 0 || target.size.1 == 0
        {
            return;
        }
        let bytes = build_vertices(&overlay.lines, target.size);
        if bytes.is_empty() {
            return;
        }

        if self
            .pipeline
            .as_ref()
            .is_none_or(|(format, _)| *format != target.format)
        {
            let shader = target
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("window text overlay shader"),
                    source: wgpu::ShaderSource::Wgsl(
                        r#"
struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
};
@vertex
fn vs_main(@location(0) position: vec2<f32>, @location(1) color: vec4<f32>) -> VertexOut {
    var out: VertexOut;
    out.position = vec4<f32>(position, 0.0, 1.0);
    out.color = color;
    return out;
}
@fragment
fn fs_main(input: VertexOut) -> @location(0) vec4<f32> {
    return input.color;
}
"#
                        .into(),
                    ),
                });
            let attributes = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4];
            let pipeline = target
                .device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("window text overlay pipeline"),
                    layout: None,
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some("vs_main"),
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                        buffers: &[wgpu::VertexBufferLayout {
                            array_stride: VERTEX_STRIDE,
                            step_mode: wgpu::VertexStepMode::Vertex,
                            attributes: &attributes,
                        }],
                    },
                    primitive: wgpu::PrimitiveState::default(),
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some("fs_main"),
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: target.format,
                            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    multiview: None,
                    cache: None,
                });
            self.pipeline = Some((target.format, pipeline));
        }

        let required = bytes.len() as u64;
        if self.vertex_buffer.is_none() || self.vertex_capacity < required {
            self.vertex_capacity = required.max(VERTEX_STRIDE).next_power_of_two();
            self.vertex_buffer = Some(target.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("window text overlay vertices"),
                size: self.vertex_capacity,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        let Some(buffer) = self.vertex_buffer.as_ref() else {
            return;
        };
        target.queue.write_buffer(buffer, 0, &bytes);
        let Some((_, pipeline)) = self.pipeline.as_ref() else {
            return;
        };
        let mut encoder = target
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("window text overlay encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("window text overlay pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target.view,
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
            pass.set_pipeline(pipeline);
            pass.set_vertex_buffer(0, buffer.slice(0..required));
            pass.draw(0..(required / VERTEX_STRIDE) as u32, 0..1);
        }
        target.queue.submit(Some(encoder.finish()));
    }
}

#[cfg(test)]
mod tests {
    use super::{build_vertices, WindowTextOverlay};

    #[test]
    fn empty_and_zero_sized_overlays_produce_no_geometry() {
        assert!(build_vertices(&[], (320, 200)).is_empty());
        assert!(build_vertices(&["TITLE".to_owned()], (0, 200)).is_empty());
        assert!(build_vertices(&["TITLE".to_owned()], (320, 0)).is_empty());
        assert!(!WindowTextOverlay::default().visible);
    }

    #[test]
    fn text_overlay_emits_complete_vertices_and_clips_small_surfaces() {
        let lines = vec!["GRAPHICS SHOWCASE".to_owned(), "MOVE THE CAMERA".to_owned()];
        let normal = build_vertices(&lines, (320, 200));
        assert!(!normal.is_empty());
        assert_eq!(normal.len() % 24, 0);
        let small = build_vertices(&lines, (40, 24));
        assert!(!small.is_empty());
        assert_eq!(small.len() % 24, 0);
        assert!(small.len() < normal.len());
    }
}
