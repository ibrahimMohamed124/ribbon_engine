//! Presents wgpu frames into the platform window.
//!
//! [`GpuWindowPlugin`] waits for the runner's [`WindowHandle`], creates a
//! [`GpuContext`] bound to it, and clears the surface to [`ClearColor`] every
//! frame. Later renderer passes build on this surface and context.

use crate::gpu::{present_mode_label, GpuContext};
use crate::graph::{RenderContext, RenderError, RenderGraph, RenderNode};
use crate::profiling::{RenderDiagnostics, RenderGraphTiming};
use engine_app::{App, LogBuffer, LogLevel, Stage, World};
use engine_platform::WindowHandle;
use engine_plugins::Plugin;
use std::time::Instant;

/// Color the window is cleared to each frame, as linear RGBA in `0.0..=1.0`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClearColor(pub [f64; 4]);

impl Default for ClearColor {
    fn default() -> Self {
        Self([0.02, 0.02, 0.03, 1.0])
    }
}

/// Everything a [`FramePass`] needs to draw into the current window frame.
pub struct FrameTarget<'a> {
    /// Device that owns the frame's resources.
    pub device: &'a wgpu::Device,
    /// Queue used to submit work.
    pub queue: &'a wgpu::Queue,
    /// Color view of the swap-chain image, already cleared this frame.
    pub view: &'a wgpu::TextureView,
    /// Color format of [`FrameTarget::view`].
    pub format: wgpu::TextureFormat,
    /// Frame size in pixels.
    pub size: (u32, u32),
}

/// A GPU drawing step registered in the native-window render graph.
///
/// Passes execute in resource-dependency order. By default, every pass consumes
/// the previous surface version and produces the next one. More specialized nodes
/// can use [`RenderGraph`] directly when they need additional named resources.
pub trait FramePass: Send + Sync + 'static {
    /// Stable diagnostic label used in render-graph errors and traces.
    fn name(&self) -> &'static str {
        "frame pass"
    }

    /// Draw into `target`. GPU resources should be created lazily from its device.
    fn render(&mut self, world: &mut World, target: &FrameTarget<'_>);
}

const NO_RESOURCES: &[&str] = &[];

struct WindowClearNode;

impl RenderNode for WindowClearNode {
    fn name(&self) -> &str {
        "window clear"
    }

    fn reads(&self) -> &[&str] {
        NO_RESOURCES
    }

    fn writes(&self) -> &[&str] {
        NO_RESOURCES
    }

    fn write_resources(&self) -> Vec<String> {
        vec!["window.color.0".to_owned()]
    }

    fn run(&mut self, _context: &mut RenderContext) -> Result<(), RenderError> {
        Ok(())
    }

    fn run_frame(
        &mut self,
        _context: &mut RenderContext,
        world: &mut World,
        target: &FrameTarget<'_>,
    ) -> Result<(), RenderError> {
        let color = world
            .try_resource::<ClearColor>()
            .copied()
            .unwrap_or_default()
            .0;
        clear_view(target.device, target.queue, target.view, color);
        Ok(())
    }
}

struct FramePassNode {
    label: String,
    input: String,
    output: String,
    pass: Box<dyn FramePass>,
}

impl RenderNode for FramePassNode {
    fn name(&self) -> &str {
        &self.label
    }

    fn reads(&self) -> &[&str] {
        NO_RESOURCES
    }

    fn writes(&self) -> &[&str] {
        NO_RESOURCES
    }

    fn read_resources(&self) -> Vec<String> {
        vec![self.input.clone()]
    }

    fn write_resources(&self) -> Vec<String> {
        vec![self.output.clone()]
    }

    fn run(&mut self, _context: &mut RenderContext) -> Result<(), RenderError> {
        Ok(())
    }

    fn run_frame(
        &mut self,
        _context: &mut RenderContext,
        world: &mut World,
        target: &FrameTarget<'_>,
    ) -> Result<(), RenderError> {
        self.pass.render(world, target);
        Ok(())
    }
}

struct WindowPresentNode {
    input: std::sync::Arc<std::sync::Mutex<String>>,
}

impl RenderNode for WindowPresentNode {
    fn name(&self) -> &str {
        "window present"
    }

    fn reads(&self) -> &[&str] {
        NO_RESOURCES
    }

    fn writes(&self) -> &[&str] {
        NO_RESOURCES
    }

    fn read_resources(&self) -> Vec<String> {
        vec![self
            .input
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()]
    }

    fn write_resources(&self) -> Vec<String> {
        vec!["window.presented".to_owned()]
    }

    fn run(&mut self, _context: &mut RenderContext) -> Result<(), RenderError> {
        Ok(())
    }
}

/// GPU state owned by the window presenter. The render graph and its pass-owned GPU
/// resources are dropped before the context (device/surface/instance).
pub struct GpuWindow {
    graph: RenderGraph,
    next_pass: usize,
    last_output: String,
    present_input: std::sync::Arc<std::sync::Mutex<String>>,
    context: Option<GpuContext>,
    failed: bool,
}

impl Default for GpuWindow {
    fn default() -> Self {
        let mut graph = RenderGraph::default();
        graph.add_node(WindowClearNode);
        let present_input = std::sync::Arc::new(std::sync::Mutex::new("window.color.0".to_owned()));
        graph.add_node(WindowPresentNode {
            input: std::sync::Arc::clone(&present_input),
        });
        Self {
            graph,
            next_pass: 0,
            last_output: "window.color.0".to_owned(),
            present_input,
            context: None,
            failed: false,
        }
    }
}

impl GpuWindow {
    /// Borrow the context once the window surface has been created.
    pub fn context(&self) -> Option<&GpuContext> {
        self.context.as_ref()
    }

    /// Append a render-graph pass. Its output becomes the next pass's input, and
    /// the present node automatically depends on the latest surface version.
    pub fn add_pass(&mut self, pass: Box<dyn FramePass>) {
        let output = format!("window.color.{}", self.next_pass + 1);
        let node = FramePassNode {
            label: format!("{} [{}]", pass.name(), self.next_pass),
            input: self.last_output.clone(),
            output: output.clone(),
            pass,
        };
        self.graph.add_node(node);
        self.next_pass += 1;
        self.last_output = output.clone();
        let mut current = self
            .present_input
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *current = output;
    }
}

/// Record a pass that clears `view` to `color` and submit it.
pub fn clear_view(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    view: &wgpu::TextureView,
    color: [f64; 4],
) {
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("clear pass"),
    });
    {
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: color[0],
                        g: color[1],
                        b: color[2],
                        a: color[3],
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
    }
    queue.submit(Some(encoder.finish()));
}

fn log_error(world: &mut World, message: String) {
    if world.try_resource::<LogBuffer>().is_some() {
        world
            .resource_mut::<LogBuffer>()
            .push(LogLevel::Error, message);
    } else {
        eprintln!("{message}");
    }
}

/// Completed measurements and resource snapshots from one presented window frame.
/// Keeping named fields here avoids a fragile positional tuple as diagnostics evolve.
struct CompletedGpuWindowFrame {
    frame_wall_ns: u64,
    surface_acquire_ns: u64,
    graph_timing: RenderGraphTiming,
    present_ns: u64,
    context_target_bytes: u64,
    context_readback_bytes: u64,
    present_mode: &'static str,
    frame_latency: u32,
}

/// Create the context on first use, follow window resizes, and present one frame.
pub fn gpu_window_system(world: &mut World) {
    let Some(window) = world.try_resource::<WindowHandle>().map(|w| w.0.clone()) else {
        return;
    };
    let size = window.inner_size();
    if size.width == 0 || size.height == 0 {
        return;
    }
    let mut error = None;
    let mut completed_frame: Option<CompletedGpuWindowFrame> = None;
    let mut surface_timeout_count = 0_u64;
    let mut surface_outdated_count = 0_u64;
    let mut surface_lost_count = 0_u64;
    let mut surface_other_error_count = 0_u64;
    let mut render_graph_error_count = 0_u64;
    world.resource_scope::<GpuWindow, _>(|world, state| {
        let GpuWindow {
            context,
            failed,
            graph,
            ..
        } = state;
        if *failed {
            return;
        }
        if context.is_none() {
            match pollster::block_on(GpuContext::new_for_window(
                window.clone(),
                size.width,
                size.height,
            )) {
                Ok(created) => *context = Some(created),
                Err(failure) => {
                    *failed = true;
                    error = Some(format!("GPU window disabled: {}", failure.0));
                    return;
                }
            }
        }
        let Some(context) = context.as_mut() else {
            return;
        };
        let stale = context
            .surface_config()
            .is_some_and(|c| c.width != size.width || c.height != size.height);
        if stale {
            context.resize(size.width, size.height);
        }
        let Some((format, present_mode, frame_latency)) = context.surface_config().map(|config| {
            (
                config.format,
                present_mode_label(config.present_mode),
                config.desired_maximum_frame_latency,
            )
        }) else {
            return;
        };
        let Some(surface) = context.surface() else {
            return;
        };
        let frame_started = Instant::now();
        let acquire_started = Instant::now();
        let acquired_surface = surface.get_current_texture();
        let surface_acquire_ns = acquire_started
            .elapsed()
            .as_nanos()
            .min(u128::from(u64::MAX)) as u64;
        match acquired_surface {
            Ok(frame) => {
                let view = frame
                    .texture
                    .create_view(&wgpu::TextureViewDescriptor::default());
                let target = FrameTarget {
                    device: context.device(),
                    queue: context.queue(),
                    view: &view,
                    format,
                    size: (size.width, size.height),
                };
                let graph_timing =
                    match graph.run_frame_profiled(&mut RenderContext::default(), world, &target) {
                        Ok(timing) => timing,
                        Err(failure) => {
                            // Do not present or profile a partially rendered frame. Returning
                            // drops the view and SurfaceTexture, and the error is logged below.
                            error = Some(format!("GPU render graph failed: {failure}"));
                            render_graph_error_count = render_graph_error_count.saturating_add(1);
                            return;
                        }
                    };
                // The graph's final `window present` node acts as the resource-ordering
                // barrier. SurfaceTexture::present itself belongs to this presenter.
                drop(view);
                let present_started = Instant::now();
                frame.present();
                let present_ns = present_started
                    .elapsed()
                    .as_nanos()
                    .min(u128::from(u64::MAX)) as u64;
                completed_frame = Some(CompletedGpuWindowFrame {
                    frame_wall_ns: frame_started.elapsed().as_nanos().min(u128::from(u64::MAX))
                        as u64,
                    surface_acquire_ns,
                    graph_timing,
                    present_ns,
                    context_target_bytes: context.target_bytes_estimate(),
                    context_readback_bytes: context.readback_buffer_bytes_estimate(),
                    present_mode,
                    frame_latency,
                });
            }
            Err(wgpu::SurfaceError::Outdated) => {
                surface_outdated_count = surface_outdated_count.saturating_add(1);
                context.resize(size.width, size.height);
            }
            Err(wgpu::SurfaceError::Lost) => {
                surface_lost_count = surface_lost_count.saturating_add(1);
                context.resize(size.width, size.height);
            }
            Err(wgpu::SurfaceError::Timeout) => {
                surface_timeout_count = surface_timeout_count.saturating_add(1);
            }
            Err(other) => {
                surface_other_error_count = surface_other_error_count.saturating_add(1);
                *failed = true;
                error = Some(format!("GPU window disabled: {other}"));
            }
        }
    });
    if surface_timeout_count > 0
        || surface_outdated_count > 0
        || surface_lost_count > 0
        || surface_other_error_count > 0
        || render_graph_error_count > 0
    {
        if world.try_resource::<RenderDiagnostics>().is_none() {
            world.insert_resource(RenderDiagnostics::default());
        }
        let diagnostics = world.resource_mut::<RenderDiagnostics>();
        diagnostics.surface_timeouts = diagnostics
            .surface_timeouts
            .saturating_add(surface_timeout_count);
        diagnostics.surface_outdated = diagnostics
            .surface_outdated
            .saturating_add(surface_outdated_count);
        diagnostics.surface_lost = diagnostics.surface_lost.saturating_add(surface_lost_count);
        diagnostics.surface_other_errors = diagnostics
            .surface_other_errors
            .saturating_add(surface_other_error_count);
        diagnostics.render_graph_errors = diagnostics
            .render_graph_errors
            .saturating_add(render_graph_error_count);
    }
    if let Some(CompletedGpuWindowFrame {
        frame_wall_ns,
        surface_acquire_ns,
        graph_timing,
        present_ns,
        context_target_bytes,
        context_readback_bytes,
        present_mode,
        frame_latency,
    }) = completed_frame
    {
        if world.try_resource::<RenderDiagnostics>().is_none() {
            world.insert_resource(RenderDiagnostics::default());
        }
        let diagnostics = world.resource_mut::<RenderDiagnostics>();
        diagnostics.record_frame_phases(
            frame_wall_ns,
            surface_acquire_ns,
            present_ns,
            graph_timing,
        );
        diagnostics.record_surface_config(present_mode, frame_latency);
        diagnostics.add_context_target_bytes(context_target_bytes);
        diagnostics.add_context_readback_bytes(context_readback_bytes);
    }
    if let Some(message) = error {
        log_error(world, message);
    }
}

/// Installs [`ClearColor`], [`GpuWindow`] and the per-frame presenter.
pub struct GpuWindowPlugin;

impl Plugin for GpuWindowPlugin {
    fn name(&self) -> &'static str {
        "GpuWindowPlugin"
    }

    fn dependencies(&self) -> &'static [&'static str] {
        &["PlatformPlugin"]
    }

    fn build(&self, app: &mut App) {
        if app.world.try_resource::<ClearColor>().is_none() {
            app.insert_resource(ClearColor::default());
        }
        if app.world.try_resource::<RenderDiagnostics>().is_none() {
            app.insert_resource(RenderDiagnostics::default());
        }
        app.insert_resource(GpuWindow::default());
        app.add_system(Stage::Render, gpu_window_system);
    }
}

#[cfg(test)]
mod tests {
    use super::{clear_view, ClearColor};

    #[test]
    fn default_clear_color_is_opaque() {
        assert_eq!(ClearColor::default().0[3], 1.0);
    }

    #[test]
    fn clear_pass_fills_the_offscreen_target() {
        let context = match pollster::block_on(crate::gpu::GpuContext::new_offscreen(8, 4)) {
            Ok(context) => context,
            Err(error) => {
                eprintln!("skipping GPU test: {}", error.0);
                return;
            }
        };
        clear_view(
            context.device(),
            context.queue(),
            context.target().view(),
            [1.0, 0.0, 1.0, 1.0],
        );
        let frame = context.readback().expect("readback");
        assert_eq!((frame.width, frame.height), (8, 4));
        assert!(frame.pixels.iter().all(|p| p & 0x00ff_ffff == 0x00ff_00ff));
    }
}
