//! Windowing, OS events, keyboard/mouse state, and desktop runners.
//!
//! This crate supplies the platform boundary; rendering and audio remain separate plugins.

pub mod config;
pub mod cursor;
pub mod input;
pub mod keyboard;
pub mod keys;
pub mod runner;
pub mod runtime;
pub mod timer;
pub mod window;

pub use config::{MouseButtons, MouseInput, WindowConfig};
pub use runner::WinitRunner;
pub use runtime::{Input, PlatformConfig, PlatformPlugin, VirtualInput};
