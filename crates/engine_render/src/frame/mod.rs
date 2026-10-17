//! Backend-neutral draw data and renderer-facing frame contracts.

mod font;
mod gfx;

pub use font::glyph;
pub use gfx::{
    DrawCommand, DrawList, Effect, Framebuffer, MAX_DRAW_QUADS_PER_FRAME,
    MAX_RENDER_PASSES_PER_FRAME,
};

/// Ordered inputs for rendering one complete frame.
pub struct FramePlan<S> {
    /// Optional backend-specific scene data.
    pub scene: Option<S>,
    /// World draw commands, preserving their insertion order.
    pub world_layer: DrawList,
    /// World brightness applied before the UI layer.
    pub brightness: f32,
    /// UI draw commands, preserving their insertion order.
    pub ui_layer: DrawList,
}

/// Generic renderer contract; scene and material types are supplied by the rendering plugin.
pub trait Renderer<M> {
    /// Scene value supplied by the rendering plugin for each borrowed frame.
    type Scene<'a>;

    /// Resize backend-owned render targets.
    fn resize(&mut self, width: u32, height: u32);
    /// Render scene data into the current frame.
    fn render_scene<'a>(&mut self, scene: Option<&Self::Scene<'a>>, materials: &M);
    /// Render the ordered frame plan.
    fn render_frame<'a>(&mut self, plan: &FramePlan<Self::Scene<'a>>, materials: &M);
    /// Copy or present the resulting pixels into a framebuffer.
    fn present(&mut self, output: &mut Framebuffer);
}
