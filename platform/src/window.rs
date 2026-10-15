//! Winit window attributes and framebuffer pointer conversion.

use engine_core::app::WindowConfig;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::window::{Window, WindowAttributes};

pub fn attributes(config: &WindowConfig) -> WindowAttributes {
    Window::default_attributes()
        .with_title(config.title.clone())
        .with_inner_size(physical_size(config))
        .with_resizable(true)
}

pub fn physical_size(config: &WindowConfig) -> PhysicalSize<u32> {
    PhysicalSize::new(
        (config.width as u32).saturating_mul(config.scale.max(1)),
        (config.height as u32).saturating_mul(config.scale.max(1)),
    )
}

pub fn mouse_position_in_framebuffer(
    position: PhysicalPosition<f64>,
    window_size: PhysicalSize<u32>,
    framebuffer_size: (usize, usize),
) -> Option<(f32, f32)> {
    let viewport =
        super::present::viewport((window_size.width, window_size.height), framebuffer_size);
    super::present::window_to_framebuffer(
        (position.x as f32, position.y as f32),
        &viewport,
        framebuffer_size,
    )
}
