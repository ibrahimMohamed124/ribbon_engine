//! Cursor grab helpers with a Locked-to-Confined fallback.

use winit::dpi::PhysicalPosition;
use winit::window::{CursorGrabMode, Window};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrabMode {
    Locked,
    Confined,
}

pub fn capture(window: &Window) -> Option<GrabMode> {
    let mode = if window.set_cursor_grab(CursorGrabMode::Locked).is_ok() {
        Some(GrabMode::Locked)
    } else if window.set_cursor_grab(CursorGrabMode::Confined).is_ok() {
        Some(GrabMode::Confined)
    } else {
        None
    };
    if mode.is_some() {
        window.set_cursor_visible(false);
    }
    mode
}

pub fn recenter_confined(window: &Window, position: PhysicalPosition<f64>) {
    let size = window.inner_size();
    let center = PhysicalPosition::new(size.width as f64 * 0.5, size.height as f64 * 0.5);
    if (position.x - center.x).abs() > 1.0 || (position.y - center.y).abs() > 1.0 {
        let _ = window.set_cursor_position(center);
    }
}

pub fn release(window: &Window) {
    let _ = window.set_cursor_grab(CursorGrabMode::None);
    window.set_cursor_visible(true);
}
