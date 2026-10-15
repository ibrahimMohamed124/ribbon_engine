//! Desktop platform integration for core applications and scene renderers.

pub mod audio;
pub mod cursor;
pub mod keyboard;
pub mod keys;
pub mod present;
pub mod timer;
pub mod window;

mod runner;

pub use engine_core::app::WindowConfig;
pub use runner::run;
