use crate::canvas::Canvas;
use crate::material::MaterialTable;
use crate::scene::Scene as RenderScene;
use ribbon_render::{DrawCommand, DrawList, FramePlan, Framebuffer, Renderer};

use crate::render::lighting::SurfaceShader;
use crate::render::post::adjust_brightness;
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

impl<S: SurfaceShader + Sync> Renderer<MaterialTable> for CpuRenderer<S> {
    type Scene<'a> = RenderScene<'a>;

    fn resize(&mut self, width: u32, height: u32) {
        self.framebuffer = Framebuffer::new(width as usize, height as usize);
    }

    fn render_scene<'a>(&mut self, scene: Option<&RenderScene<'a>>, materials: &MaterialTable) {
        self.framebuffer.pixels.fill(0);
        if let Some(scene) = scene {
            render_with_materials(&mut self.framebuffer, scene, &self.shader, materials);
        }
    }

    fn render_frame<'a>(&mut self, plan: &FramePlan<RenderScene<'a>>, materials: &MaterialTable) {
        if self.framebuffer.width != plan.world_layer.width
            || self.framebuffer.height != plan.world_layer.height
        {
            self.resize(
                plan.world_layer.width as u32,
                plan.world_layer.height as u32,
            );
        }
        self.render_scene(plan.scene.as_ref(), materials);
        composite_frame_layers(
            &mut self.framebuffer,
            &plan.world_layer,
            plan.brightness,
            &plan.ui_layer,
        );
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

/// Apply a frame's world overlay, brightness adjustment, and UI commands to a rendered
/// framebuffer. Shared by reference CPU rendering and offscreen GPU-backed applications.
pub fn composite_frame_layers(
    framebuffer: &mut Framebuffer,
    world_layer: &DrawList,
    brightness: f32,
    ui_layer: &DrawList,
) {
    apply_draw_list(framebuffer, world_layer);
    adjust_brightness(framebuffer, brightness);
    apply_draw_list(framebuffer, ui_layer);
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
    use ribbon_render::Renderer;
    use ribbon_render::{DrawCommand, DrawList, Framebuffer};

    struct NoopShader;

    impl crate::render::lighting::SurfaceShader for NoopShader {
        fn albedo(
            &self,
            _block: u8,
            _hit: &ribbon_plugin_voxel::Hit,
            _direction: ribbon_math::Vect,
            _style: &crate::RenderStyle,
        ) -> f32 {
            0.0
        }
    }

    #[test]
    fn frame_plan_composites_world_brightness_then_ui_in_order() {
        let mut renderer = CpuRenderer::new(2, 1, NoopShader);
        let mut world_layer = DrawList::new(2, 1);
        world_layer.commands.push(DrawCommand::Rect {
            x: 0,
            y: 0,
            w: 2,
            h: 1,
            color: 0x00c8_c8c8,
        });
        let mut ui_layer = DrawList::new(2, 1);
        ui_layer.commands.push(DrawCommand::Rect {
            x: 0,
            y: 0,
            w: 1,
            h: 1,
            color: 0x00ff_ffff,
        });
        let plan = FramePlan {
            scene: None,
            world_layer,
            brightness: 0.5,
            ui_layer,
        };
        renderer.render_frame(&plan, &crate::EMPTY_MATERIAL_TABLE);
        let mut output = Framebuffer::new(2, 1);
        renderer.present(&mut output);
        assert_eq!(output.pixels, vec![0x00ff_ffff, 0x0064_6464]);
    }
}
