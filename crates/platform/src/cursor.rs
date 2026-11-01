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

/// Capture using the preferred mode first, then try the other supported mode.
pub fn capture_preferred(window: &Window, preferred: GrabMode) -> Option<GrabMode> {
    let fallback = match preferred {
        GrabMode::Locked => CursorGrabMode::Confined,
        GrabMode::Confined => CursorGrabMode::Locked,
    };
    let preferred_mode = match preferred {
        GrabMode::Locked => CursorGrabMode::Locked,
        GrabMode::Confined => CursorGrabMode::Confined,
    };
    let mode = if window.set_cursor_grab(preferred_mode).is_ok() {
        Some(preferred)
    } else if window.set_cursor_grab(fallback).is_ok() {
        Some(match preferred {
            GrabMode::Locked => GrabMode::Confined,
            GrabMode::Confined => GrabMode::Locked,
        })
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
