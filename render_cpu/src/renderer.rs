use engine_core::gfx::post::adjust_brightness;
use engine_core::gfx::Framebuffer;
use engine_core::gfx::{Canvas, DrawCommand, DrawList};
use engine_scene::app::FramePlan;
use engine_scene::material::MaterialTable;
use engine_scene::renderer::Renderer;

use crate::render::lighting::SurfaceShader;
use crate::render::render_with_materials;

/// CPU reference renderer. The shader stays renderer-side and never enters `Scene`.
pub struct CpuRenderer<S> {
    framebuffer: Framebuffer,
    shader: S,
}

impl<S> CpuRenderer<S> {
    pub fn new(width: u32, height: u32, shader: S) -> Self {
        Self {
            framebuffer: Framebuffer::new(width as usize, height as usize),
            shader,
        }
    }
}

impl<S: SurfaceShader + Sync> Renderer for CpuRenderer<S> {
    fn resize(&mut self, width: u32, height: u32) {
        self.framebuffer = Framebuffer::new(width as usize, height as usize);
    }

    fn render_frame(&mut self, plan: &FramePlan<'_>, materials: &MaterialTable) {
        if self.framebuffer.width != plan.world_layer.width
            || self.framebuffer.height != plan.world_layer.height
        {
            self.resize(
                plan.world_layer.width as u32,
                plan.world_layer.height as u32,
            );
        }
        self.framebuffer.pixels.fill(0);
        if let Some(scene) = &plan.scene {
            render_with_materials(&mut self.framebuffer, scene, &self.shader, materials);
        }
        apply_draw_list(&mut self.framebuffer, &plan.world_layer);
        adjust_brightness(&mut self.framebuffer, plan.brightness);
        apply_draw_list(&mut self.framebuffer, &plan.ui_layer);
    }

    fn present(&mut self, out: &mut Framebuffer) {
        if out.width != self.framebuffer.width || out.height != self.framebuffer.height {
            out.width = self.framebuffer.width;
            out.height = self.framebuffer.height;
            out.pixels.resize(self.framebuffer.pixels.len(), 0);
        }
        out.pixels.copy_from_slice(&self.framebuffer.pixels);
    }
}

pub fn apply_draw_list(framebuffer: &mut Framebuffer, draw_list: &DrawList) {
    for command in &draw_list.commands {
        match command {
            DrawCommand::Rect { x, y, w, h, color } => framebuffer.rect(*x, *y, *w, *h, *color),
            DrawCommand::Text {
                value,
                x,
                y,
                scale,
                color,
            } => framebuffer.text(value, *x, *y, *scale, *color),
            DrawCommand::Dim(amount) => framebuffer.dim(*amount),
            DrawCommand::Effect(effect) => framebuffer.effect(*effect),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CpuRenderer, FramePlan};
    use engine_core::gfx::{DrawCommand, DrawList, Framebuffer};
    use engine_scene::renderer::Renderer;

    struct NoopShader;

    impl crate::render::lighting::SurfaceShader for NoopShader {
        fn albedo(
            &self,
            _block: u8,
            _hit: &engine_core::voxel::Hit,
            _direction: engine_core::math::Vect,
            _style: &engine_scene::RenderStyle,
        ) -> f32 {
            0.0
        }
    }

    #[test]
    fn flattened_overlay_is_copied_byte_for_byte() {
        let mut renderer = CpuRenderer::new(2, 1, NoopShader);
        let mut overlay = DrawList::new(2, 1);
        overlay.commands.push(DrawCommand::Rect {
            x: 0,
            y: 0,
            w: 1,
            h: 1,
            color: 0x0012_3456,
        });
        let plan = FramePlan {
            scene: None,
            world_layer: overlay,
            brightness: 1.0,
            ui_layer: DrawList::new(2, 1),
        };
        renderer.render_frame(&plan, &engine_scene::EMPTY_MATERIAL_TABLE);
        let mut output = Framebuffer::new(2, 1);
        renderer.present(&mut output);
        assert_eq!(output.pixels, vec![0x0012_3456, 0]);
    }
}
