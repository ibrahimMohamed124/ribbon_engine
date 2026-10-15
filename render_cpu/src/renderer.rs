use engine_core::gfx::Framebuffer;
use engine_scene::material::MaterialTable;
use engine_scene::renderer::Renderer;
use engine_scene::scene::Scene;

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

    fn render_world(&mut self, scene: &Scene<'_>, materials: &MaterialTable) {
        render_with_materials(&mut self.framebuffer, scene, &self.shader, materials);
    }

    fn draw_overlay(&mut self, overlay: &Framebuffer) {
        if self.framebuffer.width != overlay.width || self.framebuffer.height != overlay.height {
            self.resize(overlay.width as u32, overlay.height as u32);
        }
        self.framebuffer.pixels.copy_from_slice(&overlay.pixels);
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

#[cfg(test)]
mod tests {
    use super::CpuRenderer;
    use engine_core::gfx::Framebuffer;
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
        let overlay = Framebuffer {
            width: 2,
            height: 1,
            pixels: vec![0x0012_3456, 0],
        };
        renderer.draw_overlay(&overlay);
        let mut output = Framebuffer::new(2, 1);
        renderer.present(&mut output);
        assert_eq!(output.pixels, overlay.pixels);
    }
}
