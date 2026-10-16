use crate::app::FramePlan;
use engine_core::gfx::Framebuffer;

/// Rendering contract shared by the CPU reference renderer and future backends.
pub trait Renderer {
    fn resize(&mut self, width: u32, height: u32);
    fn render_scene(
        &mut self,
        scene: Option<&crate::scene::Scene<'_>>,
        materials: &crate::material::MaterialTable,
    );
    fn render_frame(&mut self, plan: &FramePlan<'_>, materials: &crate::material::MaterialTable);
    fn present(&mut self, out: &mut Framebuffer);
}
