//! minifb window setup and coordinate conversion helpers.

use engine_core::app::WindowConfig;
use engine_core::input::Key;
use minifb::{KeyRepeat, MouseButton, MouseMode, Scale, ScaleMode, Window, WindowOptions};

pub fn scale_from(factor: u32) -> Scale {
    match factor {
        1 => Scale::X1,
        2 => Scale::X2,
        _ => Scale::X4,
    }
}

pub fn create_window(config: &WindowConfig) -> Result<Window, minifb::Error> {
    Window::new(
        &config.title,
        config.width,
        config.height,
        WindowOptions {
            scale: scale_from(config.scale),
            scale_mode: ScaleMode::AspectRatioStretch,
            resize: true,
            ..WindowOptions::default()
        },
    )
}

pub fn mouse_position_in_framebuffer(
    window: &Window,
    framebuffer_size: (usize, usize),
) -> Option<(f32, f32)> {
    let (x, y) = window.get_unscaled_mouse_pos(MouseMode::Discard)?;
    let (window_width, window_height) = window.get_size();
    let (framebuffer_width, framebuffer_height) = framebuffer_size;
    if window_width == 0 || window_height == 0 || framebuffer_width == 0 || framebuffer_height == 0
    {
        return None;
    }

    let scale = (window_width as f32 / framebuffer_width as f32)
        .min(window_height as f32 / framebuffer_height as f32);
    let offset_x = (window_width as f32 - framebuffer_width as f32 * scale) * 0.5;
    let offset_y = (window_height as f32 - framebuffer_height as f32 * scale) * 0.5;
    let x = (x - offset_x) / scale;
    let y = (y - offset_y) / scale;
    (x >= 0.0 && y >= 0.0 && x < framebuffer_width as f32 && y < framebuffer_height as f32)
        .then_some((x, y))
}

pub fn pressed_keys(window: &mut Window) -> Vec<Key> {
    window
        .get_keys_pressed(KeyRepeat::No)
        .into_iter()
        .filter_map(super::keys::from_minifb)
        .collect()
}

pub fn left_mouse_down(window: &Window) -> bool {
    window.get_mouse_down(MouseButton::Left)
}

pub fn mouse_buttons(window: &Window) -> (bool, bool, bool) {
    (
        window.get_mouse_down(MouseButton::Left),
        window.get_mouse_down(MouseButton::Right),
        window.get_mouse_down(MouseButton::Middle),
    )
}

pub fn scroll_y(window: &Window) -> f32 {
    window.get_scroll_wheel().map_or(0.0, |(_, y)| y)
}
