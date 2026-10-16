//! Compatibility re-exports for the shared ray/object intersection API.
pub use crate::entities::ENEMY_BOUNDS as ENEMY_BBOX;
pub use crate::surfaces::{enemy_view, page_view};
pub use engine_scene::entities::{Bounds3, Box3, EntityInstance, EntityKind};
pub use render_cpu::render::entities::{ray_box, EntHit, EntView};
