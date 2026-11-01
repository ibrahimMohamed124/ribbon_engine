//! Winit window attributes and framebuffer pointer conversion.

use crate::config::WindowConfig;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::window::{Icon, Window, WindowAttributes};

/// Build the Ribbon Engine window icon from the checked-in RGBA asset.
///
/// The raw RGBA representation avoids a runtime image-decoder dependency and keeps
/// the icon available in every application using this platform crate.
pub fn engine_window_icon() -> Option<Icon> {
    const SIZE: u32 = 64;
    const RGBA: &[u8] = include_bytes!("../../../assets/engine_icon.rgba");
    Icon::from_rgba(RGBA.to_vec(), SIZE, SIZE).ok()
}

/// Apply the engine icon to both the titlebar and (on Windows) the taskbar.
///
/// Winit's generic `with_window_icon` sets the small Windows window icon (`ICON_SMALL`),
/// which appears in the titlebar. Windows taskbar buttons use the large icon (`ICON_BIG`),
/// configured with the Windows-specific `with_taskbar_icon` extension.
pub fn with_engine_icons(attributes: WindowAttributes) -> WindowAttributes {
    #[cfg(target_os = "windows")]
    {
        let attributes = attributes.with_window_icon(engine_window_icon());
        use winit::platform::windows::WindowAttributesExtWindows;
        attributes.with_taskbar_icon(engine_taskbar_icon().or_else(engine_window_icon))
    }

    #[cfg(not(target_os = "windows"))]
    {
        with_window_icon(attributes, engine_window_icon())
    }
}

/// Apply a custom icon to the window and, on Windows, its taskbar button.
///
/// Use this when creating a custom window outside [`crate::WinitRunner`]. For a
/// running engine window, use [`crate::WindowHandle::set_icon`].
pub fn with_window_icon(attributes: WindowAttributes, icon: Option<Icon>) -> WindowAttributes {
    let attributes = attributes.with_window_icon(icon.clone());

    #[cfg(target_os = "windows")]
    {
        use winit::platform::windows::WindowAttributesExtWindows;
        attributes.with_taskbar_icon(icon)
    }

    #[cfg(not(target_os = "windows"))]
    {
        attributes
    }
}

/// Set a custom icon on an existing window and its Windows taskbar button.
pub fn set_window_icon(window: &Window, icon: Option<Icon>) {
    window.set_window_icon(icon.clone());

    #[cfg(target_os = "windows")]
    {
        use winit::platform::windows::WindowExtWindows;
        window.set_taskbar_icon(icon);
    }
}

/// Load the Windows taskbar icon at the native large-icon size from the executable's
/// multi-resolution icon resource. Keep the RGBA asset as a fallback for downstream
/// applications that do not embed the workspace's PE icon resource.
#[cfg(target_os = "windows")]
fn engine_taskbar_icon() -> Option<Icon> {
    use winit::dpi::PhysicalSize;
    use winit::platform::windows::IconExtWindows;

    Icon::from_resource(101, Some(PhysicalSize::new(32, 32)))
        .ok()
        .or_else(engine_window_icon)
}

pub fn attributes(config: &WindowConfig) -> WindowAttributes {
    with_engine_icons(
        Window::default_attributes()
            .with_title(config.title.clone())
            .with_inner_size(physical_size(config))
            .with_resizable(true),
    )
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
