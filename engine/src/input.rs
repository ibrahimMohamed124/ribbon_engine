//! Platform-independent physical keys and reusable key bindings.

use std::collections::HashMap;
use std::hash::Hash;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,
    Up,
    Down,
    Left,
    Right,
    LeftShift,
    RightShift,
    Space,
    Escape,
    Enter,
    Tab,
}

pub const ALL_KEYS: [Key; 36] = [
    Key::A,
    Key::B,
    Key::C,
    Key::D,
    Key::E,
    Key::F,
    Key::G,
    Key::H,
    Key::I,
    Key::J,
    Key::K,
    Key::L,
    Key::M,
    Key::N,
    Key::O,
    Key::P,
    Key::Q,
    Key::R,
    Key::S,
    Key::T,
    Key::U,
    Key::V,
    Key::W,
    Key::X,
    Key::Y,
    Key::Z,
    Key::Up,
    Key::Down,
    Key::Left,
    Key::Right,
    Key::LeftShift,
    Key::RightShift,
    Key::Space,
    Key::Escape,
    Key::Enter,
    Key::Tab,
];

pub trait KeyState {
    fn is_down(&self, key: Key) -> bool;
}

pub trait MouseSource {
    fn delta(&mut self) -> (f32, f32);
}

pub struct Bindings<A> {
    map: HashMap<A, Vec<Key>>,
}

impl<A: Eq + Hash> Default for Bindings<A> {
    fn default() -> Self {
        Self::new()
    }
}

impl<A: Clone> Clone for Bindings<A> {
    fn clone(&self) -> Self {
        Self {
            map: self.map.clone(),
        }
    }
}

impl<A: Eq + Hash> Bindings<A> {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    pub fn keys(&self, action: A) -> &[Key] {
        self.map.get(&action).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn set(&mut self, action: A, keys: Vec<Key>) {
        self.map.insert(action, keys);
    }

    pub fn is_down<K: KeyState + ?Sized>(&self, state: &K, action: A) -> bool {
        self.keys(action).iter().any(|key| state.is_down(*key))
    }
}

const KEY_NAMES: &[(&str, Key)] = &[
    ("A", Key::A),
    ("B", Key::B),
    ("C", Key::C),
    ("D", Key::D),
    ("E", Key::E),
    ("F", Key::F),
    ("G", Key::G),
    ("H", Key::H),
    ("I", Key::I),
    ("J", Key::J),
    ("K", Key::K),
    ("L", Key::L),
    ("M", Key::M),
    ("N", Key::N),
    ("O", Key::O),
    ("P", Key::P),
    ("Q", Key::Q),
    ("R", Key::R),
    ("S", Key::S),
    ("T", Key::T),
    ("U", Key::U),
    ("V", Key::V),
    ("W", Key::W),
    ("X", Key::X),
    ("Y", Key::Y),
    ("Z", Key::Z),
    ("Up", Key::Up),
    ("Down", Key::Down),
    ("Left", Key::Left),
    ("Right", Key::Right),
    ("LeftShift", Key::LeftShift),
    ("RightShift", Key::RightShift),
    ("Space", Key::Space),
    ("Escape", Key::Escape),
    ("Enter", Key::Enter),
    ("Tab", Key::Tab),
];

pub fn key_name(key: Key) -> Option<&'static str> {
    KEY_NAMES
        .iter()
        .find(|(_, candidate)| *candidate == key)
        .map(|(name, _)| *name)
}

pub fn parse_key(name: &str) -> Option<Key> {
    let name = name.trim();
    KEY_NAMES
        .iter()
        .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
        .map(|(_, key)| *key)
}
