pub use engine_core::app::{Ctx, Flow};
use engine_core::gfx::DrawList;

use crate::material::MaterialTable;
use crate::scene::Scene;
use crate::EMPTY_MATERIAL_TABLE;

/// Ordered inputs for rendering one complete frame.
pub struct FramePlan<'a> {
    pub scene: Option<Scene<'a>>,
    pub world_layer: DrawList,
    pub brightness: f32,
    pub ui_layer: DrawList,
}

/// Application boundary consumed by a platform runner and a renderer backend.
pub trait App {
    fn update(&mut self, ctx: &mut Ctx<'_>) -> Flow;

    /// Called when automatic presentation falls back to the CPU renderer.
    fn on_cpu_fallback(&mut self) {}

    fn frame_plan(&self, ctx: &Ctx<'_>) -> FramePlan<'_>;

    fn materials(&self) -> &'static MaterialTable {
        &EMPTY_MATERIAL_TABLE
    }
}
