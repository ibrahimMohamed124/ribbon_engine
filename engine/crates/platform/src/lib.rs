//! Desktop platform integration for core applications and scene renderers.

#[cfg(feature = "audio")]
pub mod audio;
pub mod cursor;
pub mod keyboard;
pub mod keys;
pub mod mixer;
pub mod present;
pub mod timer;
pub mod window;

pub mod runner;

pub use runner::{run, run_with_mode, RendererMode};
pub use voxelworks_api::WindowConfig;
