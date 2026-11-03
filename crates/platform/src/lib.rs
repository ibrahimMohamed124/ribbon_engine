//! Windowing, OS events, keyboard/mouse state, and desktop runners.
//!
//! This crate supplies the platform boundary; rendering and audio remain separate plugins.

pub mod config;
pub mod cursor;
pub mod gamepad;
pub use gamepad::{inject_raw_gamepad_event, GamepadRumbleRequest};
pub use ribbon_input as input;
pub use ribbon_input::{
    key_name, parse_key, ActionMap, ActionState, BindingSource, BufferedInput, InputState, Key,
    KeyState, MouseButton, MouseInputState, MouseSource, NativeKeyCode, Rebinding, ALL_KEYS,
};
pub use ribbon_input::{GamepadAxis, GamepadButton, GamepadEvent, GamepadSet, GamepadSettings};
pub mod keyboard;
pub mod keys;
pub mod runner;
pub mod runtime;
pub mod timer;
pub mod window;

pub use config::{MouseButtons, MouseInput, WindowConfig};
pub use runner::WinitRunner;
pub use runtime::{
    DisplayResolution, DisplaySettings, Input, PlatformConfig, PlatformPlugin, TextInput,
    TextInputEvent, VSyncMode, VirtualInput, WindowFrame, WindowHandle, WindowMode,
};
pub use window::{engine_window_icon, set_window_icon, with_window_icon};
pub use winit::window::{Icon, Window};
