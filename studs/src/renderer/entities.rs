//! Compatibility re-exports for the shared ray/object intersection API.
pub use crate::surfaces::{enemy_view, page_view, ENEMY_BOUNDS as ENEMY_BBOX};
pub use blocks_engine::render::entities::{ray_box, Bounds3, Box3, EntHit, EntView, SurfaceFn};
