pub use engine_core::app::{Ctx, Flow};
use engine_core::gfx::Framebuffer;

use crate::material::MaterialTable;
use crate::scene::Scene;
use crate::EMPTY_MATERIAL_TABLE;

/// Application boundary consumed by a platform runner and a renderer backend.
pub trait App {
    fn update(&mut self, ctx: &mut Ctx<'_>) -> Flow;

    fn scene(&self, alpha: f32) -> Option<Scene<'_>>;

    fn overlay(&mut self, overlay: &mut Framebuffer, ctx: &Ctx<'_>);

    fn materials(&self) -> &'static MaterialTable {
        &EMPTY_MATERIAL_TABLE
    }
}
