//! Presents wgpu frames into the platform window.
//!
//! [`GpuWindowPlugin`] waits for the runner's [`WindowHandle`], creates a
//! [`GpuContext`] bound to it, and clears the surface to [`ClearColor`] every
//! frame. Later renderer passes build on this surface and context.

use crate::gpu::GpuContext;
use engine_app::{App, LogBuffer, LogLevel, Stage, World};
use engine_platform::WindowHandle;
use engine_plugins::Plugin;

/// Color the window is cleared to each frame, as linear RGBA in `0.0..=1.0`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClearColor(pub [f64; 4]);

impl Default for ClearColor {
    fn default() -> Self {
        Self([0.02, 0.02, 0.03, 1.0])
    }
}

/// GPU state owned by the window presenter.
#[derive(Default)]
pub struct GpuWindow {
    context: Option<GpuContext>,
    failed: bool,
}

impl GpuWindow {
    /// Borrow the context once the window surface has been created.
    pub fn context(&self) -> Option<&GpuContext> {
        self.context.as_ref()
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

/// Create the context on first use, follow window resizes, and present one frame.
pub fn gpu_window_system(world: &mut World) {
    let Some(window) = world.try_resource::<WindowHandle>().map(|w| w.0.clone()) else {
        return;
    };
    let size = window.inner_size();
    if size.width == 0 || size.height == 0 {
        return;
    }
    let color = world
        .try_resource::<ClearColor>()
        .copied()
        .unwrap_or_default()
        .0;
    let mut error = None;
    {
        let state = world.resource_mut::<GpuWindow>();
        if state.failed {
            return;
        }
        if state.context.is_none() {
            match pollster::block_on(GpuContext::new_for_window(
                window.clone(),
                size.width,
                size.height,
            )) {
                Ok(context) => state.context = Some(context),
                Err(failure) => {
                    state.failed = true;
                    error = Some(format!("GPU window disabled: {}", failure.0));
                }
            }
        }
        if let Some(context) = state.context.as_mut() {
            let stale = context
                .surface_config()
                .is_some_and(|c| c.width != size.width || c.height != size.height);
            if stale {
                context.resize(size.width, size.height);
            }
            if let Some(surface) = context.surface() {
                match surface.get_current_texture() {
                    Ok(frame) => {
                        let view = frame
                            .texture
                            .create_view(&wgpu::TextureViewDescriptor::default());
                        clear_view(context.device(), context.queue(), &view, color);
                        frame.present();
                    }
                    Err(wgpu::SurfaceError::Outdated | wgpu::SurfaceError::Lost) => {
                        context.resize(size.width, size.height);
                    }
                    Err(wgpu::SurfaceError::Timeout) => {}
                    Err(other) => {
                        state.failed = true;
                        error = Some(format!("GPU window disabled: {other}"));
                    }
                }
            }
        }
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
