//! Platform input state, scripted input, and typed events shared by desktop runners.

use crate::config::MouseButtons;
pub use crate::input::MouseButton as InputMouseButton;
use crate::input::{
    ActionState, GamepadAxis, GamepadButton, GamepadSet, InputState, Key, KeyState, MouseInputState,
};
use ribbon_app::{App, AppExit};
use ribbon_events::WorldEventsExt;
use ribbon_plugins::Plugin;

/// A keyboard edge or state change from the operating system or a virtual source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyboardEvent {
    /// Physical engine key.
    pub key: Key,
    /// Whether the key is now pressed.
    pub pressed: bool,
    /// Whether this press was a key repeat.
    pub repeat: bool,
}

/// Committed Unicode text and IME composition updates from the operating system.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextInputEvent {
    /// A committed text fragment; it may contain multiple Unicode scalar values.
    Commit(String),
    /// Current pre-edit composition text and byte-indexed selection/caret range.
    Preedit {
        text: String,
        cursor: Option<(usize, usize)>,
    },
    /// The operating system enabled IME composition.
    Enabled,
    /// IME composition ended or was cancelled.
    Disabled,
}

/// Frame-local text input snapshot plus the active IME pre-edit string.
#[derive(Clone, Debug, Default)]
pub struct TextInput {
    committed: Vec<String>,
    preedit: Option<String>,
    preedit_cursor: Option<(usize, usize)>,
    ime_enabled: bool,
}

impl TextInput {
    /// Text fragments committed during the current frame, preserving event order.
    pub fn committed(&self) -> &[String] {
        &self.committed
    }

    /// Current IME pre-edit string, if composition is active and non-empty.
    pub fn preedit(&self) -> Option<&str> {
        self.preedit.as_deref()
    }

    /// Whether the platform currently reports an active IME session.
    pub fn ime_enabled(&self) -> bool {
        self.ime_enabled
    }

    /// Byte-indexed IME cursor/selection range within [`Self::preedit`].
    pub fn preedit_cursor(&self) -> Option<(usize, usize)> {
        self.preedit_cursor
    }

    fn begin_frame(&mut self) {
        self.committed.clear();
    }

    fn apply(&mut self, event: &TextInputEvent) {
        match event {
            TextInputEvent::Commit(text) => {
                if !text.is_empty() {
                    self.committed.push(text.clone());
                }
                self.preedit = None;
                self.preedit_cursor = None;
            }
            TextInputEvent::Preedit { text, cursor } => {
                if text.is_empty() {
                    self.preedit = None;
                    self.preedit_cursor = None;
                } else {
                    self.preedit = Some(text.clone());
                    self.preedit_cursor = cursor.map(|(start, end)| {
                        let clamp_boundary = |offset: usize| {
                            let mut offset = offset.min(text.len());
                            while !text.is_char_boundary(offset) {
                                offset -= 1;
                            }
                            offset
                        };
                        (clamp_boundary(start), clamp_boundary(end))
                    });
                }
            }
            TextInputEvent::Enabled => self.ime_enabled = true,
            TextInputEvent::Disabled => {
                self.ime_enabled = false;
                self.preedit = None;
                self.preedit_cursor = None;
            }
        }
    }
}

/// Pointer movement in physical pixels and its latest window position, if available.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MouseMotion {
    /// Relative pointer delta.
    pub delta: (f32, f32),
    /// Current pointer position in physical window coordinates.
    pub position: Option<(f32, f32)>,
    /// Scroll amount reported with the pointer event.
    pub scroll: f32,
}

/// A mouse button state change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MouseButtonEvent {
    /// Button that changed.
    pub button: InputMouseButton,
    /// Whether the button is now pressed.
    pub pressed: bool,
}

/// Physical window size changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowResized {
    /// New width in physical pixels.
    pub width: u32,
    /// New height in physical pixels.
    pub height: u32,
}

/// The window's focus state changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Focused(pub bool);

/// Shared handle to the desktop window, inserted by the runner once it exists.
///
/// Renderers use it to create a surface; it is absent in headless runs.
#[derive(Clone)]
pub struct WindowHandle(pub std::sync::Arc<winit::window::Window>);

impl WindowHandle {
    /// Return the native window's OS scaling factor.
    pub fn scale_factor(&self) -> f64 {
        self.0.scale_factor()
    }
    /// Change the window icon and, on Windows, its taskbar icon.
    pub fn set_icon(&self, icon: winit::window::Icon) {
        crate::window::set_window_icon(&self.0, Some(icon));
    }

    /// Remove a custom icon from the window and its Windows taskbar button.
    pub fn clear_icon(&self) {
        crate::window::set_window_icon(&self.0, None);
    }

    /// Position the operating-system IME candidate/caret window relative to a text widget.
    pub fn set_ime_cursor_area(
        &self,
        position: winit::dpi::PhysicalPosition<i32>,
        size: winit::dpi::PhysicalSize<u32>,
    ) {
        self.0.set_ime_cursor_area(position, size);
    }

    /// Set the native IME candidate area using raw physical-pixel tuples.
    /// This keeps downstream engine crates independent from the `winit` API.
    pub fn set_ime_cursor_area_pixels(&self, position: (i32, i32), size: (u32, u32)) {
        self.0.set_ime_cursor_area(
            winit::dpi::PhysicalPosition::new(position.0, position.1),
            winit::dpi::PhysicalSize::new(size.0, size.1),
        );
    }

    /// Enable or disable native input-method events for the application window.
    pub fn set_ime_allowed(&self, allowed: bool) {
        self.0.set_ime_allowed(allowed);
    }
}

/// Renderer-neutral CPU frame submitted for presentation by the platform runner.
///
/// Pixel values use `0x00RRGGBB`; zero dimensions mean that no frame is ready.
#[derive(Clone, Debug, Default)]
pub struct WindowFrame {
    /// Frame width in pixels.
    pub width: usize,
    /// Frame height in pixels.
    pub height: usize,
    /// Row-major pixels in `0x00RRGGBB` format.
    pub pixels: Vec<u32>,
}

impl WindowFrame {
    /// Replace the current frame while reusing the pixel allocation where possible.
    pub fn set_pixels(&mut self, width: usize, height: usize, pixels: &[u32]) {
        let expected = width.saturating_mul(height);
        if width == 0 || height == 0 || pixels.len() < expected {
            self.width = 0;
            self.height = 0;
            self.pixels.clear();
            return;
        }
        self.width = width;
        self.height = height;
        self.pixels.clear();
        self.pixels.extend_from_slice(&pixels[..expected]);
    }
}

/// The desktop window requested to close.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CloseRequested;

/// Platform-neutral input event queued by a runner or a scripted source.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PlatformEvent {
    /// Keyboard state update.
    Keyboard(KeyboardEvent),
    /// Pointer movement.
    MouseMotion(MouseMotion),
    /// Mouse button state update.
    MouseButton(MouseButtonEvent),
    /// Window resized.
    WindowResized(WindowResized),
    /// Window close requested.
    CloseRequested,
    /// Window focus changed.
    Focused(bool),
}

/// Cursor capture state as known by the platform layer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorGrabState {
    /// Cursor is visible and unconstrained.
    #[default]
    Released,
    /// Cursor is locked to the window.
    Locked,
    /// Cursor is confined to the window.
    Confined,
}

/// Engine input snapshot, updated at the beginning of each frame.
///
/// Physical key/button state and per-frame edges are owned by `ribbon_input`; the platform keeps
/// the legacy public fields as a stable snapshot for callers that already consume them.
#[derive(Default)]
pub struct Input {
    keyboard: InputState,
    mouse: MouseInputState,
    gamepads: GamepadSet,
    /// Current physical pointer position in window coordinates.
    pub mouse_position: Option<(f32, f32)>,
    /// Pointer delta accumulated this frame.
    pub mouse_delta: (f32, f32),
    /// Scroll amount accumulated this frame.
    pub mouse_scroll: f32,
    /// Common mouse buttons held this frame. Other mouse buttons are retained by `ribbon_input`.
    pub mouse_buttons: MouseButtons,
    /// Current focus state.
    pub focused: bool,
    /// Cursor capture state.
    pub cursor_grab: CursorGrabState,
}

impl Input {
    /// Read processed gamepad button and axis state.
    pub fn gamepads(&self) -> &GamepadSet {
        &self.gamepads
    }
    pub(crate) fn gamepads_mut(&mut self) -> &mut GamepadSet {
        &mut self.gamepads
    }
    /// Check whether a physical key is held.
    pub fn is_down(&self, key: Key) -> bool {
        self.keyboard.is_down(key)
    }

    /// Check whether a key became held during the current frame.
    pub fn just_pressed(&self, key: Key) -> bool {
        self.keyboard.is_just_pressed(key)
    }

    /// Check whether a key became released during the current frame.
    pub fn just_released(&self, key: Key) -> bool {
        self.keyboard.is_just_released(key)
    }

    /// Check whether a mouse button is held, including additional OS buttons.
    pub fn mouse_button_down(&self, button: InputMouseButton) -> bool {
        self.mouse.is_down(button)
    }

    /// Check whether a mouse button became held during this frame.
    pub fn mouse_button_just_pressed(&self, button: InputMouseButton) -> bool {
        self.mouse.just_pressed(button)
    }

    /// Check whether a mouse button became released during this frame.
    pub fn mouse_button_just_released(&self, button: InputMouseButton) -> bool {
        self.mouse.just_released(button)
    }

    fn begin_frame(&mut self) {
        self.keyboard.begin_frame();
        self.mouse.begin_frame();
        self.mouse_delta = (0.0, 0.0);
        self.mouse_scroll = 0.0;
    }

    fn apply(&mut self, event: PlatformEvent) {
        match event {
            PlatformEvent::Keyboard(event) if event.pressed => {
                self.keyboard.press_key(event.key, event.repeat);
            }
            PlatformEvent::Keyboard(event) => {
                self.keyboard.release_key(event.key);
            }
            PlatformEvent::MouseMotion(event) => {
                self.mouse
                    .apply_motion(event.delta, event.position, event.scroll);
                self.mouse_delta.0 += event.delta.0;
                self.mouse_delta.1 += event.delta.1;
                self.mouse_scroll += event.scroll;
                if event.position.is_some() {
                    self.mouse_position = event.position;
                }
            }
            PlatformEvent::MouseButton(event) => {
                if event.pressed {
                    self.mouse.press_button(event.button);
                } else {
                    self.mouse.release_button(event.button);
                }
                match event.button {
                    InputMouseButton::Left => self.mouse_buttons.left = event.pressed,
                    InputMouseButton::Right => self.mouse_buttons.right = event.pressed,
                    InputMouseButton::Middle => self.mouse_buttons.middle = event.pressed,
                    InputMouseButton::Other(_) => {}
                }
            }
            PlatformEvent::Focused(focused) => {
                self.focused = focused;
                if !focused {
                    self.keyboard.focus_lost();
                    self.mouse.focus_lost();
                    self.mouse_buttons = MouseButtons::default();
                    self.mouse_delta = (0.0, 0.0);
                    self.mouse_scroll = 0.0;
                    self.mouse_position = None;
                }
            }
            PlatformEvent::WindowResized(_) | PlatformEvent::CloseRequested => {}
        }
    }
}

impl KeyState for Input {
    fn is_down(&self, key: Key) -> bool {
        self.is_down(key)
    }
}

impl ActionState for Input {
    fn is_mouse_button_down(&self, button: InputMouseButton) -> bool {
        self.mouse_button_down(button)
    }
    fn is_gamepad_button_down(&self, pad: u8, button: u16) -> bool {
        GamepadButton::from_id(button).is_some_and(|button| self.gamepads.button_down(pad, button))
    }
    fn gamepad_axis_value(&self, pad: u8, axis: u8) -> f32 {
        GamepadAxis::from_id(axis).map_or(0.0, |axis| self.gamepads.axis(pad, axis))
    }
}

/// Scripted events consumed at the start of the next frame. The queue implementation lives in
/// the platform-independent `ribbon_input` crate, while the event enum remains platform-owned.
pub type VirtualInput = crate::input::VirtualInput<PlatformEvent>;

#[derive(Default)]
pub(crate) struct PendingInput {
    pub events: Vec<PlatformEvent>,
    pub text_events: Vec<TextInputEvent>,
}

/// Requested native window mode. Exclusive fullscreen gracefully falls back to borderless
/// when the monitor does not expose the requested video mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WindowMode {
    /// Resizable decorated window.
    #[default]
    Windowed,
    /// Borderless fullscreen on the current monitor.
    BorderlessFullscreen,
    /// Exclusive fullscreen using a matching monitor video mode when available.
    ExclusiveFullscreen,
}

/// User-facing vertical-sync policy, independent from wgpu's concrete present mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VSyncMode {
    /// FIFO presentation; waits for the next refresh.
    On,
    /// Prefer FIFO Relaxed and fall back to FIFO.
    Adaptive,
    /// Prefer Immediate (no synchronization), then Mailbox, with FIFO as the universal fallback.
    #[default]
    Off,
}

/// A requested drawable resolution in physical pixels. Zero-sized modes are rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayResolution {
    pub width: u32,
    pub height: u32,
}

impl DisplayResolution {
    pub const fn new(width: u32, height: u32) -> Option<Self> {
        if width == 0 || height == 0 {
            None
        } else {
            Some(Self { width, height })
        }
    }
}

/// Runtime display preferences. This is a separate resource so existing `PlatformConfig`
/// struct literals and serialized/game-specific window configuration remain source-compatible.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DisplaySettings {
    /// Requested native window mode.
    pub window_mode: WindowMode,
    /// Optional physical drawable resolution. Borderless mode uses the monitor's native size.
    /// `None` follows the legacy `PlatformConfig` dimensions.
    pub resolution: Option<DisplayResolution>,
    /// User-facing VSync policy.
    /// `None` preserves the existing environment/configuration-selected backend mode.
    pub vsync: Option<VSyncMode>,
}

impl DisplaySettings {
    /// Request a physical drawable resolution. Invalid zero dimensions leave the setting intact.
    pub fn set_resolution(&mut self, width: u32, height: u32) -> bool {
        let Some(resolution) = DisplayResolution::new(width, height) else {
            return false;
        };
        self.resolution = Some(resolution);
        true
    }

    /// Follow the legacy PlatformConfig dimensions again.
    pub fn clear_resolution_override(&mut self) {
        self.resolution = None;
    }

    /// Restore windowed mode, the legacy configured resolution, and backend/environment VSync.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Window creation configuration inserted by [`PlatformPlugin`].
#[derive(Clone, Debug)]
pub struct PlatformConfig {
    /// Window title.
    pub title: String,
    /// Initial physical width.
    pub width: u32,
    /// Initial physical height.
    pub height: u32,
}

impl Default for PlatformConfig {
    fn default() -> Self {
        Self {
            title: "Ribbon".to_owned(),
            width: 1280,
            height: 720,
        }
    }
}

/// Registers desktop input events and selects the winit runner.
#[derive(Default)]
pub struct PlatformPlugin;

impl Plugin for PlatformPlugin {
    fn name(&self) -> &'static str {
        "PlatformPlugin"
    }

    fn build(&self, app: &mut App) {
        if app.world.try_resource::<PlatformConfig>().is_none() {
            app.insert_resource(PlatformConfig::default());
        }
        app.insert_resource(Input::default())
            .insert_resource(VirtualInput::default())
            .insert_resource(PendingInput::default())
            .insert_resource(WindowFrame::default())
            .insert_resource(TextInput::default());
        if app.world.try_resource::<DisplaySettings>().is_none() {
            app.insert_resource(DisplaySettings::default());
        }
        app.add_event::<KeyboardEvent>()
            .add_event::<MouseMotion>()
            .add_event::<MouseButtonEvent>()
            .add_event::<WindowResized>()
            .add_event::<CloseRequested>()
            .add_event::<Focused>()
            .add_event::<TextInputEvent>();
        crate::gamepad::install(app);
        app.add_system(ribbon_app::Stage::First, update_input);
        app.set_runner(crate::WinitRunner);
    }
}

fn update_input(world: &mut ribbon_app::World) {
    let mut pending = std::mem::take(&mut world.resource_mut::<PendingInput>().events);
    pending.extend(world.resource_mut::<VirtualInput>().drain());
    let pending_text = std::mem::take(&mut world.resource_mut::<PendingInput>().text_events);
    world.resource_scope::<Input, _>(|world, input| {
        input.begin_frame();
        for event in pending {
            input.apply(event);
            match event {
                PlatformEvent::Keyboard(event) => world.send_event(event),
                PlatformEvent::MouseMotion(event) => world.send_event(event),
                PlatformEvent::MouseButton(event) => world.send_event(event),
                PlatformEvent::WindowResized(event) => world.send_event(event),
                PlatformEvent::Focused(focused) => world.send_event(Focused(focused)),
                PlatformEvent::CloseRequested => {
                    world.send_event(CloseRequested);
                    world.send_event(AppExit);
                }
            }
        }
    });
    world.resource_scope::<TextInput, _>(|world, text_input| {
        text_input.begin_frame();
        for event in pending_text {
            text_input.apply(&event);
            world.send_event(event);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{
        Input, KeyboardEvent, MouseButtonEvent, MouseMotion, PlatformEvent, PlatformPlugin,
        VirtualInput,
    };
    use crate::input::{Key, MouseButton};
    use ribbon_app::App;
    use ribbon_plugins::AppPluginExt;

    #[test]
    fn focus_loss_releases_held_keyboard_and_mouse_state() {
        let mut app = App::new();
        app.add_plugin(PlatformPlugin).unwrap();
        let mut app = app.build().unwrap();
        {
            let virtual_input = app.world.resource_mut::<VirtualInput>();
            virtual_input.push(PlatformEvent::Keyboard(KeyboardEvent {
                key: Key::W,
                pressed: true,
                repeat: false,
            }));
            virtual_input.push(PlatformEvent::MouseButton(MouseButtonEvent {
                button: MouseButton::Left,
                pressed: true,
            }));
            virtual_input.push(PlatformEvent::MouseMotion(MouseMotion {
                delta: (5.0, 2.0),
                position: Some((30.0, 40.0)),
                scroll: 1.0,
            }));
        }
        app.update();
        assert!(app.world.resource::<Input>().is_down(Key::W));
        assert!(app
            .world
            .resource::<Input>()
            .mouse_button_down(MouseButton::Left));

        {
            let virtual_input = app.world.resource_mut::<VirtualInput>();
            // Focus loss may be queued after motion in the same frame; do not leak this delta
            // into the next camera update when the window is no longer active.
            virtual_input.push(PlatformEvent::MouseMotion(MouseMotion {
                delta: (9.0, -3.0),
                position: Some((40.0, 50.0)),
                scroll: 2.0,
            }));
            virtual_input.push(PlatformEvent::Focused(false));
        }
        app.update();
        let input = app.world.resource::<Input>();
        assert!(!input.is_down(Key::W));
        assert!(input.just_released(Key::W));
        assert!(!input.mouse_button_down(MouseButton::Left));
        assert!(input.mouse_button_just_released(MouseButton::Left));
        assert_eq!(input.mouse_delta, (0.0, 0.0));
        assert_eq!(input.mouse_scroll, 0.0);
        assert_eq!(input.mouse_position, None);
    }

    #[test]
    fn text_input_preserves_unicode_commits_and_ime_composition_lifecycle() {
        use super::{TextInput, TextInputEvent};

        let mut text = TextInput::default();
        text.begin_frame();
        text.apply(&TextInputEvent::Enabled);
        text.apply(&TextInputEvent::Preedit {
            text: "にほん".to_owned(),
            cursor: Some((3, 3)),
        });
        assert!(text.ime_enabled());
        assert_eq!(text.preedit(), Some("にほん"));
        assert_eq!(text.preedit_cursor(), Some((3, 3)));
        // A malformed native selection may point into a UTF-8 continuation byte. Keep
        // consumers safe by snapping each byte offset backward to a character boundary.
        text.apply(&TextInputEvent::Preedit {
            text: "é🙂".to_owned(),
            cursor: Some((1, usize::MAX)),
        });
        assert_eq!(text.preedit_cursor(), Some((0, 6)));

        text.apply(&TextInputEvent::Commit("日本語🙂".to_owned()));
        assert_eq!(text.committed(), &["日本語🙂".to_owned()]);
        assert_eq!(text.preedit(), None);

        text.begin_frame();
        assert!(text.committed().is_empty());
        text.apply(&TextInputEvent::Preedit {
            text: "compose".to_owned(),
            cursor: Some((7, 7)),
        });
        text.apply(&TextInputEvent::Disabled);
        assert!(!text.ime_enabled());
        assert_eq!(text.preedit(), None);
        assert_eq!(text.preedit_cursor(), None);
    }

    #[test]
    fn display_settings_have_safe_windowed_defaults_and_are_independent_of_window_config() {
        use super::{DisplaySettings, VSyncMode, WindowMode};

        let mut settings = DisplaySettings::default();
        assert_eq!(settings.window_mode, WindowMode::Windowed);
        assert_eq!(settings.vsync, None);
        settings.window_mode = WindowMode::BorderlessFullscreen;
        settings.vsync = Some(VSyncMode::On);
        settings.reset();
        assert_eq!(settings, DisplaySettings::default());
    }

    #[test]
    fn display_resolution_rejects_zero_and_supports_runtime_override_reset() {
        use super::{DisplayResolution, DisplaySettings};

        assert_eq!(
            DisplayResolution::new(1920, 1080),
            Some(DisplayResolution {
                width: 1920,
                height: 1080
            })
        );
        assert_eq!(DisplayResolution::new(0, 1080), None);
        assert_eq!(DisplayResolution::new(1920, 0), None);

        let mut settings = DisplaySettings::default();
        assert!(!settings.set_resolution(0, 720));
        assert_eq!(settings.resolution, None);
        assert!(settings.set_resolution(1920, 1080));
        assert_eq!(settings.resolution, DisplayResolution::new(1920, 1080));
        settings.clear_resolution_override();
        assert_eq!(settings.resolution, None);
    }

    #[test]
    fn virtual_input_press_edge_lasts_exactly_one_frame() {
        let mut app = App::new();
        app.add_plugin(PlatformPlugin).unwrap();
        let mut app = app.build().unwrap();
        app.world
            .resource_mut::<VirtualInput>()
            .push(PlatformEvent::Keyboard(KeyboardEvent {
                key: Key::W,
                pressed: true,
                repeat: false,
            }));
        app.update();
        assert!(app.world.resource::<Input>().just_pressed(Key::W));
        assert!(app.world.resource::<Input>().is_down(Key::W));
        app.update();
        assert!(!app.world.resource::<Input>().just_pressed(Key::W));
        assert!(app.world.resource::<Input>().is_down(Key::W));
    }

    #[test]
    fn injected_gamepad_axis_drives_bindings_and_survives_focus_loss() {
        use crate::gamepad::inject_raw_gamepad_event;
        use crate::input::{BindingSource, Bindings, GamepadAxis, RawGamepadEvent};
        let mut app = App::new();
        app.add_plugin(PlatformPlugin).unwrap();
        let mut app = app.build().unwrap();
        inject_raw_gamepad_event(&mut app.world, RawGamepadEvent::Connected { slot: 0 });
        inject_raw_gamepad_event(
            &mut app.world,
            RawGamepadEvent::Axis {
                slot: 0,
                axis: GamepadAxis::LeftStickX,
                value: 0.8,
            },
        );
        let mut bindings = Bindings::new();
        bindings.set_sources(
            "move",
            vec![BindingSource::gamepad_axis(
                0,
                GamepadAxis::LeftStickX,
                true,
                0.5,
            )],
        );
        assert!(bindings.is_active(app.world.resource::<Input>(), "move"));
        app.world
            .resource_mut::<Input>()
            .apply(super::PlatformEvent::Focused(false));
        assert!(
            app.world
                .resource::<Input>()
                .gamepads()
                .axis(0, GamepadAxis::LeftStickX)
                >= 0.5
        );
    }
}
