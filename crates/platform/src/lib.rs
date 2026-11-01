//! Windowing, OS events, keyboard/mouse state, and desktop runners.
//!
//! This crate supplies the platform boundary; rendering and audio remain separate plugins.

pub mod config;
pub mod cursor;
pub use ribbon_input as input;
pub use ribbon_input::{
    key_name, parse_key, ActionMap, ActionState, BindingSource, BufferedInput, InputState, Key,
    KeyState, MouseButton, MouseInputState, MouseSource, Rebinding, ALL_KEYS,
};
pub mod keyboard;
pub mod keys;
pub mod runner;
pub mod runtime;
pub mod timer;
pub mod window;

pub use config::{MouseButtons, MouseInput, WindowConfig};
pub use runner::WinitRunner;
pub use runtime::{Input, PlatformConfig, PlatformPlugin, VirtualInput, WindowFrame, WindowHandle};
pub use window::{engine_window_icon, set_window_icon, with_window_icon};
pub use winit::window::{Icon, Window};
