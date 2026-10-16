//! Shared contracts used by games, the platform runner, and rendering backends.

pub mod app;
pub mod font;
pub mod gfx;
pub mod renderer;

pub use app::{
    App, Ctx, CursorRequest, Flow, FrameInfo, FramePlan, MouseButtons, MouseInput, Requests,
    WindowConfig, WindowRequest,
};
pub use gfx::{DrawCommand, DrawList, Effect, Framebuffer};
pub use renderer::Renderer;
