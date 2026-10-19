//! Minimal application contract and per-frame input/request context.

pub use engine_platform::config::{MouseButtons, MouseInput, WindowConfig};
use voxelworks_core::input::{Key, KeyState};
use voxelworks_scene::material::MaterialTable;
use voxelworks_scene::scene::Scene;
use voxelworks_scene::EMPTY_MATERIAL_TABLE;

/// Compatibility alias for the legacy scene-specific frame plan.
pub type FramePlan<'a> = engine_render::frame::FramePlan<Scene<'a>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Quit,
}

#[derive(Clone, Copy, Debug)]
pub struct WindowRequest {
    pub width: usize,
    pub height: usize,
    pub scale: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct FrameInfo {
    pub dt: f32,
    pub frame: u32,
    pub fps: f32,
    pub fixed_dt: f32,
    pub fixed_steps: u32,
    pub alpha: f32,
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
    pub fixed_dt: f32,
    pub fixed_steps: u32,
    pub alpha: f32,
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
        frame_info: FrameInfo,
        keys: &'a dyn KeyState,
        pressed_keys: &'a [Key],
        mouse: MouseInput,
        window_active: bool,
    ) -> Self {
        Self {
            dt: frame_info.dt,
            frame: frame_info.frame,
            fps: frame_info.fps,
            fixed_dt: frame_info.fixed_dt,
            fixed_steps: frame_info.fixed_steps,
            alpha: frame_info.alpha,
            keys,
            pressed_keys,
            mouse_delta: mouse.delta,
            mouse_position: mouse.position,
            mouse_scroll: mouse.scroll,
            mouse_buttons: mouse.buttons,
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

/// Application boundary consumed by a platform runner and renderer backend.
pub trait App {
    fn update(&mut self, ctx: &mut Ctx<'_>) -> Flow;

    /// Called when automatic presentation falls back to the CPU renderer.
    fn on_cpu_fallback(&mut self) {}

    fn frame_plan(&self, ctx: &Ctx<'_>) -> FramePlan<'_>;

    fn materials(&self) -> &'static MaterialTable {
        &EMPTY_MATERIAL_TABLE
    }
}
