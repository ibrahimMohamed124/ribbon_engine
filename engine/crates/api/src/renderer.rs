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

impl<R> Renderer for R
where
    for<'a> R: engine_render::Renderer<MaterialTable, Scene<'a> = Scene<'a>>,
{
    fn resize(&mut self, width: u32, height: u32) {
        engine_render::Renderer::<MaterialTable>::resize(self, width, height);
    }

    fn render_scene(&mut self, scene: Option<&Scene<'_>>, materials: &MaterialTable) {
        engine_render::Renderer::<MaterialTable>::render_scene(self, scene, materials);
    }

    fn render_frame(&mut self, plan: &FramePlan<'_>, materials: &MaterialTable) {
        engine_render::Renderer::<MaterialTable>::render_frame(self, plan, materials);
    }

    fn present(&mut self, out: &mut Framebuffer) {
        engine_render::Renderer::<MaterialTable>::present(self, out);
    }
}
