//! Platform input state, scripted input, and typed events shared by desktop runners.

use crate::config::MouseButtons;
use crate::input::{Key, KeyState};
use engine_app::{App, AppExit};
use engine_events::WorldEventsExt;
use engine_plugins::Plugin;
use std::collections::HashSet;

/// A platform-neutral mouse button used by input events.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputMouseButton {
    /// Left button.
    Left,
    /// Right button.
    Right,
    /// Middle button.
    Middle,
    /// Additional button number reported by the operating system.
    Other(u16),
}

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
#[derive(Default)]
pub struct Input {
    down: HashSet<Key>,
    just_pressed: HashSet<Key>,
    just_released: HashSet<Key>,
    /// Current physical pointer position in window coordinates.
    pub mouse_position: Option<(f32, f32)>,
    /// Pointer delta accumulated this frame.
    pub mouse_delta: (f32, f32),
    /// Scroll amount accumulated this frame.
    pub mouse_scroll: f32,
    /// Mouse buttons held this frame.
    pub mouse_buttons: MouseButtons,
    /// Current focus state.
    pub focused: bool,
    /// Cursor capture state.
    pub cursor_grab: CursorGrabState,
}

impl Input {
    /// Check whether a physical key is held.
    pub fn is_down(&self, key: Key) -> bool {
        self.down.contains(&key)
    }

    /// Check whether a key became held during the current frame.
    pub fn just_pressed(&self, key: Key) -> bool {
        self.just_pressed.contains(&key)
    }

    /// Check whether a key became released during the current frame.
    pub fn just_released(&self, key: Key) -> bool {
        self.just_released.contains(&key)
    }

    fn begin_frame(&mut self) {
        self.just_pressed.clear();
        self.just_released.clear();
        self.mouse_delta = (0.0, 0.0);
        self.mouse_scroll = 0.0;
    }

    fn apply(&mut self, event: PlatformEvent) {
        match event {
            PlatformEvent::Keyboard(event) if event.pressed => {
                if self.down.insert(event.key) && !event.repeat {
                    self.just_pressed.insert(event.key);
                }
            }
            PlatformEvent::Keyboard(event) => {
                if self.down.remove(&event.key) {
                    self.just_released.insert(event.key);
                }
            }
            PlatformEvent::MouseMotion(event) => {
                self.mouse_delta.0 += event.delta.0;
                self.mouse_delta.1 += event.delta.1;
                self.mouse_scroll += event.scroll;
                if event.position.is_some() {
                    self.mouse_position = event.position;
                }
            }
            PlatformEvent::MouseButton(event) => {
                let state = match event.button {
                    InputMouseButton::Left => Some(&mut self.mouse_buttons.left),
                    InputMouseButton::Right => Some(&mut self.mouse_buttons.right),
                    InputMouseButton::Middle => Some(&mut self.mouse_buttons.middle),
                    InputMouseButton::Other(_) => None,
                };
                if let Some(state) = state {
                    *state = event.pressed;
                }
            }
            PlatformEvent::Focused(focused) => {
                self.focused = focused;
                if !focused {
                    self.just_released.extend(self.down.drain());
                    self.mouse_buttons = MouseButtons::default();
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

/// Scripted events consumed at the start of the next frame.
#[derive(Default)]
pub struct VirtualInput {
    events: Vec<PlatformEvent>,
}

impl VirtualInput {
    /// Queue an event for the next `First` stage.
    pub fn push(&mut self, event: PlatformEvent) {
        self.events.push(event);
    }
}

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
            title: "Voxelworks".to_owned(),
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
            .insert_resource(PendingInput::default());
        app.add_event::<KeyboardEvent>()
            .add_event::<MouseMotion>()
            .add_event::<MouseButtonEvent>()
            .add_event::<WindowResized>()
            .add_event::<CloseRequested>()
            .add_event::<Focused>();
        app.add_system(engine_app::Stage::First, update_input);
        app.set_runner(crate::WinitRunner);
    }
}

fn update_input(world: &mut engine_app::World) {
    let mut pending = std::mem::take(&mut world.resource_mut::<PendingInput>().events);
    pending.extend(std::mem::take(
        &mut world.resource_mut::<VirtualInput>().events,
    ));
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
    use super::{Input, KeyboardEvent, PlatformEvent, PlatformPlugin, VirtualInput};
    use crate::input::Key;
    use engine_app::App;
    use engine_plugins::AppPluginExt;

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
