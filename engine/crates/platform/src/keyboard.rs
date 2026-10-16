//! Physical keyboard and mouse event state for the desktop application.

use std::collections::HashSet;

use voxelworks_api::{MouseButtons, MouseInput};
use voxelworks_core::input::{Key, KeyState};
use winit::event::{ElementState, MouseButton, MouseScrollDelta};
use winit::keyboard::KeyCode;

#[derive(Default)]
pub struct KeyboardState {
    down: HashSet<Key>,
    pressed: Vec<Key>,
}

impl KeyboardState {
    pub fn process_key(&mut self, key: Key, state: ElementState, repeat: bool) {
        match state {
            ElementState::Pressed => {
                if self.down.insert(key) && !repeat {
                    self.pressed.push(key);
                }
            }
            ElementState::Released => {
                self.down.remove(&key);
            }
        }
    }

    pub fn process_winit_key(&mut self, key: KeyCode, state: ElementState, repeat: bool) {
        if let Some(key) = from_winit(key) {
            self.process_key(key, state, repeat);
        }
    }

    pub fn pressed(&self) -> &[Key] {
        &self.pressed
    }

    pub fn take_pressed(&mut self) -> Vec<Key> {
        std::mem::take(&mut self.pressed)
    }

    pub fn clear(&mut self) {
        self.down.clear();
        self.pressed.clear();
    }
}

impl KeyState for KeyboardState {
    fn is_down(&self, key: Key) -> bool {
        self.down.contains(&key)
    }
}

/// Map physical keys, preserving QWERTY-position controls across keyboard layouts.
pub fn from_winit(code: KeyCode) -> Option<Key> {
    Some(match code {
        KeyCode::KeyA => Key::A,
        KeyCode::KeyB => Key::B,
        KeyCode::KeyC => Key::C,
        KeyCode::KeyD => Key::D,
        KeyCode::KeyE => Key::E,
        KeyCode::KeyF => Key::F,
        KeyCode::KeyG => Key::G,
        KeyCode::KeyH => Key::H,
        KeyCode::KeyI => Key::I,
        KeyCode::KeyJ => Key::J,
        KeyCode::KeyK => Key::K,
        KeyCode::KeyL => Key::L,
        KeyCode::KeyM => Key::M,
        KeyCode::KeyN => Key::N,
        KeyCode::KeyO => Key::O,
        KeyCode::KeyP => Key::P,
        KeyCode::KeyQ => Key::Q,
        KeyCode::KeyR => Key::R,
        KeyCode::KeyS => Key::S,
        KeyCode::KeyT => Key::T,
        KeyCode::KeyU => Key::U,
        KeyCode::KeyV => Key::V,
        KeyCode::KeyW => Key::W,
        KeyCode::KeyX => Key::X,
        KeyCode::KeyY => Key::Y,
        KeyCode::KeyZ => Key::Z,
        KeyCode::ArrowUp => Key::Up,
        KeyCode::ArrowDown => Key::Down,
        KeyCode::ArrowLeft => Key::Left,
        KeyCode::ArrowRight => Key::Right,
        KeyCode::ShiftLeft => Key::LeftShift,
        KeyCode::ShiftRight => Key::RightShift,
        KeyCode::Space => Key::Space,
        KeyCode::Escape => Key::Escape,
        KeyCode::Enter => Key::Enter,
        KeyCode::Tab => Key::Tab,
        _ => return None,
    })
}

#[derive(Default)]
pub struct MouseState {
    buttons: MouseButtons,
    scroll: f32,
}

impl MouseState {
    pub fn process_button(&mut self, button: MouseButton, state: ElementState) {
        let down = state == ElementState::Pressed;
        match button {
            MouseButton::Left => self.buttons.left = down,
            MouseButton::Right => self.buttons.right = down,
            MouseButton::Middle => self.buttons.middle = down,
            MouseButton::Back | MouseButton::Forward | MouseButton::Other(_) => {}
        }
    }

    pub fn process_scroll(&mut self, delta: MouseScrollDelta) {
        self.scroll += match delta {
            MouseScrollDelta::LineDelta(_, y) => y,
            MouseScrollDelta::PixelDelta(position) => position.y as f32 / 40.0,
        };
    }

    pub fn snapshot(&mut self, delta: (f32, f32), position: Option<(f32, f32)>) -> MouseInput {
        MouseInput {
            delta,
            position,
            scroll: std::mem::take(&mut self.scroll),
            buttons: self.buttons,
        }
    }

    pub fn clear(&mut self) {
        self.buttons = MouseButtons::default();
        self.scroll = 0.0;
    }
}

pub const fn key_mapping() -> [(KeyCode, Key); 36] {
    use voxelworks_core::input::Key as K;
    use KeyCode as C;
    [
        (C::KeyA, K::A),
        (C::KeyB, K::B),
        (C::KeyC, K::C),
        (C::KeyD, K::D),
        (C::KeyE, K::E),
        (C::KeyF, K::F),
        (C::KeyG, K::G),
        (C::KeyH, K::H),
        (C::KeyI, K::I),
        (C::KeyJ, K::J),
        (C::KeyK, K::K),
        (C::KeyL, K::L),
        (C::KeyM, K::M),
        (C::KeyN, K::N),
        (C::KeyO, K::O),
        (C::KeyP, K::P),
        (C::KeyQ, K::Q),
        (C::KeyR, K::R),
        (C::KeyS, K::S),
        (C::KeyT, K::T),
        (C::KeyU, K::U),
        (C::KeyV, K::V),
        (C::KeyW, K::W),
        (C::KeyX, K::X),
        (C::KeyY, K::Y),
        (C::KeyZ, K::Z),
        (C::ArrowUp, K::Up),
        (C::ArrowDown, K::Down),
        (C::ArrowLeft, K::Left),
        (C::ArrowRight, K::Right),
        (C::ShiftLeft, K::LeftShift),
        (C::ShiftRight, K::RightShift),
        (C::Space, K::Space),
        (C::Escape, K::Escape),
        (C::Enter, K::Enter),
        (C::Tab, K::Tab),
    ]
}

#[cfg(test)]
mod tests {
    use super::{from_winit, key_mapping, KeyboardState, MouseState};
    use voxelworks_core::input::{Key, KeyState, ALL_KEYS};
    use winit::event::{ElementState, MouseButton, MouseScrollDelta};

    #[test]
    fn every_engine_key_has_a_physical_winit_mapping() {
        let mapping = key_mapping();
        assert_eq!(mapping.len(), ALL_KEYS.len());
        for (code, expected) in mapping {
            assert_eq!(from_winit(code), Some(expected));
        }
        assert!(mapping.iter().all(|(_, key)| ALL_KEYS.contains(key)));
        assert_eq!(
            mapping
                .iter()
                .map(|(_, key)| *key)
                .collect::<std::collections::HashSet<_>>()
                .len(),
            36
        );
    }

    #[test]
    fn key_events_track_edges_ignore_repeat_and_clear_on_focus_loss() {
        let mut keys = KeyboardState::default();
        keys.process_key(Key::W, ElementState::Pressed, false);
        keys.process_key(Key::W, ElementState::Pressed, true);
        assert!(keys.is_down(Key::W));
        assert_eq!(keys.pressed(), &[Key::W]);
        assert_eq!(keys.take_pressed(), vec![Key::W]);
        keys.process_key(Key::W, ElementState::Released, false);
        assert!(!keys.is_down(Key::W));
        keys.process_key(Key::A, ElementState::Pressed, false);
        keys.clear();
        assert!(!keys.is_down(Key::A));
        assert!(keys.pressed().is_empty());
    }

    #[test]
    fn mouse_buttons_and_scroll_are_captured_in_one_snapshot() {
        let mut mouse = MouseState::default();
        mouse.process_button(MouseButton::Left, ElementState::Pressed);
        mouse.process_button(MouseButton::Right, ElementState::Pressed);
        mouse.process_scroll(MouseScrollDelta::LineDelta(0.0, 1.5));
        let snapshot = mouse.snapshot((2.0, -3.0), Some((4.0, 5.0)));
        assert!(snapshot.buttons.left && snapshot.buttons.right);
        assert_eq!(snapshot.scroll, 1.5);
        assert_eq!(snapshot.delta, (2.0, -3.0));
        assert_eq!(snapshot.position, Some((4.0, 5.0)));
        assert_eq!(mouse.snapshot((0.0, 0.0), None).scroll, 0.0);
        mouse.clear();
    }
}
