//! Mapping between the engine's physical keys and minifb.

use std::collections::HashSet;

use engine_core::input::{Key, KeyState, ALL_KEYS};
use minifb::{Key as DesktopKey, Window};

pub fn from_minifb(key: DesktopKey) -> Option<Key> {
    Some(match key {
        DesktopKey::A => Key::A,
        DesktopKey::B => Key::B,
        DesktopKey::C => Key::C,
        DesktopKey::D => Key::D,
        DesktopKey::E => Key::E,
        DesktopKey::F => Key::F,
        DesktopKey::G => Key::G,
        DesktopKey::H => Key::H,
        DesktopKey::I => Key::I,
        DesktopKey::J => Key::J,
        DesktopKey::K => Key::K,
        DesktopKey::L => Key::L,
        DesktopKey::M => Key::M,
        DesktopKey::N => Key::N,
        DesktopKey::O => Key::O,
        DesktopKey::P => Key::P,
        DesktopKey::Q => Key::Q,
        DesktopKey::R => Key::R,
        DesktopKey::S => Key::S,
        DesktopKey::T => Key::T,
        DesktopKey::U => Key::U,
        DesktopKey::V => Key::V,
        DesktopKey::W => Key::W,
        DesktopKey::X => Key::X,
        DesktopKey::Y => Key::Y,
        DesktopKey::Z => Key::Z,
        DesktopKey::Up => Key::Up,
        DesktopKey::Down => Key::Down,
        DesktopKey::Left => Key::Left,
        DesktopKey::Right => Key::Right,
        DesktopKey::LeftShift => Key::LeftShift,
        DesktopKey::RightShift => Key::RightShift,
        DesktopKey::Space => Key::Space,
        DesktopKey::Escape => Key::Escape,
        DesktopKey::Enter => Key::Enter,
        DesktopKey::Tab => Key::Tab,
        _ => return None,
    })
}

pub fn to_minifb(key: Key) -> DesktopKey {
    match key {
        Key::A => DesktopKey::A,
        Key::B => DesktopKey::B,
        Key::C => DesktopKey::C,
        Key::D => DesktopKey::D,
        Key::E => DesktopKey::E,
        Key::F => DesktopKey::F,
        Key::G => DesktopKey::G,
        Key::H => DesktopKey::H,
        Key::I => DesktopKey::I,
        Key::J => DesktopKey::J,
        Key::K => DesktopKey::K,
        Key::L => DesktopKey::L,
        Key::M => DesktopKey::M,
        Key::N => DesktopKey::N,
        Key::O => DesktopKey::O,
        Key::P => DesktopKey::P,
        Key::Q => DesktopKey::Q,
        Key::R => DesktopKey::R,
        Key::S => DesktopKey::S,
        Key::T => DesktopKey::T,
        Key::U => DesktopKey::U,
        Key::V => DesktopKey::V,
        Key::W => DesktopKey::W,
        Key::X => DesktopKey::X,
        Key::Y => DesktopKey::Y,
        Key::Z => DesktopKey::Z,
        Key::Up => DesktopKey::Up,
        Key::Down => DesktopKey::Down,
        Key::Left => DesktopKey::Left,
        Key::Right => DesktopKey::Right,
        Key::LeftShift => DesktopKey::LeftShift,
        Key::RightShift => DesktopKey::RightShift,
        Key::Space => DesktopKey::Space,
        Key::Escape => DesktopKey::Escape,
        Key::Enter => DesktopKey::Enter,
        Key::Tab => DesktopKey::Tab,
    }
}

pub struct WindowKeys<'a>(pub &'a Window);

impl KeyState for WindowKeys<'_> {
    fn is_down(&self, key: Key) -> bool {
        self.0.is_key_down(to_minifb(key))
    }
}

pub struct KeySnapshot(HashSet<Key>);

pub fn snapshot(window: &Window) -> KeySnapshot {
    KeySnapshot(
        ALL_KEYS
            .into_iter()
            .filter(|&key| window.is_key_down(to_minifb(key)))
            .collect(),
    )
}

impl KeyState for KeySnapshot {
    fn is_down(&self, key: Key) -> bool {
        self.0.contains(&key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposed_keys_round_trip() {
        for name in [
            "A",
            "B",
            "C",
            "D",
            "E",
            "F",
            "G",
            "H",
            "I",
            "J",
            "K",
            "L",
            "M",
            "N",
            "O",
            "P",
            "Q",
            "R",
            "S",
            "T",
            "U",
            "V",
            "W",
            "X",
            "Y",
            "Z",
            "Up",
            "Down",
            "Left",
            "Right",
            "LeftShift",
            "RightShift",
            "Space",
            "Escape",
            "Enter",
            "Tab",
        ] {
            let key = engine_core::input::parse_key(name).unwrap();
            assert_eq!(from_minifb(to_minifb(key)), Some(key));
        }
    }
}
