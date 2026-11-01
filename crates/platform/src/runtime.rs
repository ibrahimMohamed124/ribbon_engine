//! Platform input state, scripted input, and typed events shared by desktop runners.

use crate::config::MouseButtons;
pub use crate::input::MouseButton as InputMouseButton;
use crate::input::{ActionState, InputState, Key, KeyState, MouseInputState};
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
    /// Change the window icon and, on Windows, its taskbar icon.
    pub fn set_icon(&self, icon: winit::window::Icon) {
        crate::window::set_window_icon(&self.0, Some(icon));
    }

    /// Remove a custom icon from the window and its Windows taskbar button.
    pub fn clear_icon(&self) {
        crate::window::set_window_icon(&self.0, None);
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
}

/// Scripted events consumed at the start of the next frame. The queue implementation lives in
/// the platform-independent `ribbon_input` crate, while the event enum remains platform-owned.
pub type VirtualInput = crate::input::VirtualInput<PlatformEvent>;

#[derive(Default)]
pub(crate) struct PendingInput {
    pub events: Vec<PlatformEvent>,
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
            .insert_resource(WindowFrame::default());
        app.add_event::<KeyboardEvent>()
            .add_event::<MouseMotion>()
            .add_event::<MouseButtonEvent>()
            .add_event::<WindowResized>()
            .add_event::<CloseRequested>()
            .add_event::<Focused>();
        app.add_system(ribbon_app::Stage::First, update_input);
        app.set_runner(crate::WinitRunner);
    }
}

fn update_input(world: &mut ribbon_app::World) {
    let mut pending = std::mem::take(&mut world.resource_mut::<PendingInput>().events);
    pending.extend(world.resource_mut::<VirtualInput>().drain());
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
}
