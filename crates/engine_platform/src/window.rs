//! Winit window attributes and framebuffer pointer conversion.

use crate::config::WindowConfig;
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
    let viewport = viewport((window_size.width, window_size.height), framebuffer_size);
    window_to_framebuffer(
        (position.x as f32, position.y as f32),
        &viewport,
        framebuffer_size,
    )
}

/// Aspect-preserving centered viewport for a logical framebuffer.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Viewport {
    /// Left edge in window pixels.
    pub x: f32,
    /// Top edge in window pixels.
    pub y: f32,
    /// Scale from framebuffer pixels to window pixels.
    pub scale: f32,
}

/// Compute the largest centered fit of the framebuffer inside a window.
pub fn viewport(window_size: (u32, u32), fb_size: (usize, usize)) -> Viewport {
    let (window_width, window_height) = window_size;
    let (fb_width, fb_height) = fb_size;
    if window_width == 0 || window_height == 0 || fb_width == 0 || fb_height == 0 {
        return Viewport::default();
    }
    let scale =
        (window_width as f32 / fb_width as f32).min(window_height as f32 / fb_height as f32);
    Viewport {
        x: (window_width as f32 - fb_width as f32 * scale) * 0.5,
        y: (window_height as f32 - fb_height as f32 * scale) * 0.5,
        scale,
    }
}

/// Convert a physical pointer position into the visible framebuffer rectangle.
pub fn window_to_framebuffer(
    position: (f32, f32),
    viewport: &Viewport,
    framebuffer_size: (usize, usize),
) -> Option<(f32, f32)> {
    let (width, height) = framebuffer_size;
    let (x, y) = position;
    if width == 0
        || height == 0
        || !viewport.scale.is_finite()
        || viewport.scale <= 0.0
        || !x.is_finite()
        || !y.is_finite()
    {
        return None;
    }
    let x = (x - viewport.x) / viewport.scale;
    let y = (y - viewport.y) / viewport.scale;
    (x >= 0.0 && y >= 0.0 && x < width as f32 && y < height as f32).then_some((x, y))
}
