//! Minimal application contract and per-frame input/request context.

use crate::gfx::Framebuffer;
use crate::input::{Key, KeyState};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Quit,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct MouseButtons {
    pub left: bool,
    pub right: bool,
    pub middle: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct WindowRequest {
    pub width: usize,
    pub height: usize,
    pub scale: u32,
}

#[derive(Clone, Debug)]
pub struct WindowConfig {
    pub title: String,
    pub width: usize,
    pub height: usize,
    pub scale: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorRequest {
    #[default]
    Release,
    Capture,
}

#[derive(Default)]
pub struct Requests {
    pub title: Option<String>,
    pub window: Option<WindowRequest>,
    pub cursor: CursorRequest,
}

pub struct Ctx<'a> {
    pub dt: f32,
    pub frame: u32,
    pub fps: f32,
    pub keys: &'a dyn KeyState,
    pub pressed_keys: &'a [Key],
    pub mouse_delta: (f32, f32),
    pub mouse_position: Option<(f32, f32)>,
    pub mouse_scroll: f32,
    pub mouse_buttons: MouseButtons,
    pub window_active: bool,
    pub requests: Requests,
}

impl<'a> Ctx<'a> {
    pub fn new(
        dt: f32,
        frame: u32,
        fps: f32,
        keys: &'a dyn KeyState,
        pressed_keys: &'a [Key],
        mouse_delta: (f32, f32),
        mouse_position: Option<(f32, f32)>,
        mouse_scroll: f32,
        mouse_buttons: MouseButtons,
        window_active: bool,
    ) -> Self {
        Self {
            dt,
            frame,
            fps,
            keys,
            pressed_keys,
            mouse_delta,
            mouse_position,
            mouse_scroll,
            mouse_buttons,
            window_active,
            requests: Requests::default(),
        }
    }

    pub fn request_title(&mut self, title: impl Into<String>) {
        self.requests.title = Some(title.into());
    }

    pub fn request_window(&mut self, window: WindowRequest) {
        self.requests.window = Some(window);
    }

    pub fn capture_cursor(&mut self) {
        self.requests.cursor = CursorRequest::Capture;
    }

    pub fn release_cursor(&mut self) {
        self.requests.cursor = CursorRequest::Release;
    }
}

pub trait App {
    fn update(&mut self, ctx: &mut Ctx<'_>) -> Flow;
    fn render(&mut self, framebuffer: &mut Framebuffer, ctx: &Ctx<'_>);
}
