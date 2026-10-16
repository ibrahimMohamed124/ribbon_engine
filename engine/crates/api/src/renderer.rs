use crate::app::FramePlan;
use crate::gfx::Framebuffer;
use voxelworks_scene::material::MaterialTable;
use voxelworks_scene::scene::Scene;

/// Rendering contract shared by the CPU reference renderer and future backends.
pub trait Renderer {
    fn resize(&mut self, width: u32, height: u32);
    fn render_scene(&mut self, scene: Option<&Scene<'_>>, materials: &MaterialTable);
    fn render_frame(&mut self, plan: &FramePlan<'_>, materials: &MaterialTable);
    fn present(&mut self, out: &mut Framebuffer);
}
