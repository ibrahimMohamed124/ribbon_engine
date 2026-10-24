//! Window and pointer value types shared with the temporary legacy API bridge.

/// Mouse button state for the three buttons handled by the engine input layer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MouseButtons {
    /// Left button is held.
    pub left: bool,
    /// Right button is held.
    pub right: bool,
    /// Middle button is held.
    pub middle: bool,
}

/// Pointer state captured for one frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct MouseInput {
    /// Relative movement in physical pixels.
    pub delta: (f32, f32),
    /// Position in the framebuffer, when inside the displayed image.
    pub position: Option<(f32, f32)>,
    /// Accumulated scroll amount for the frame.
    pub scroll: f32,
    /// Buttons currently held.
    pub buttons: MouseButtons,
}

/// Dimensions and title used to create a desktop window.
#[derive(Clone, Debug)]
pub struct WindowConfig {
    /// Window title.
    pub title: String,
    /// Logical framebuffer width.
    pub width: usize,
    /// Logical framebuffer height.
    pub height: usize,
    /// Initial integer scale factor.
    pub scale: u32,
    /// Legacy fixed simulation frequency, kept for the old runner.
    pub fixed_hz: u32,
    /// Legacy maximum number of fixed steps per frame.
    pub max_steps: u32,
}
