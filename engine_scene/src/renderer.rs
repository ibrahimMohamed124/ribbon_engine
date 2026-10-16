use crate::material::MaterialTable;
use crate::scene::Scene;
use engine_core::gfx::Framebuffer;

/// Rendering contract shared by the CPU reference renderer and future backends.
pub trait Renderer {
    fn resize(&mut self, width: u32, height: u32);
    fn render_world(&mut self, scene: &Scene<'_>, materials: &MaterialTable);
    fn draw_overlay(&mut self, overlay: &Framebuffer);
    fn present(&mut self, out: &mut Framebuffer);
}
