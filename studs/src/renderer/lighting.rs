//! Shared falloff helpers; the game's material shader lives in `surfaces`.
pub use render_cpu::render::lighting::cone_factor as spot_factor;
pub use render_cpu::render::lighting::{light, SurfaceShader};
