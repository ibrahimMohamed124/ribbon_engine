//! Desktop platform integration for `blocks_engine`.

pub mod audio;
pub mod cursor;
pub mod keyboard;
pub mod keys;
pub mod present;
pub mod timer;
pub mod window;

mod runner;

pub use blocks_engine::app::WindowConfig;
pub use runner::run;
