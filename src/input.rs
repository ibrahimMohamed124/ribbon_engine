//! Keyboard / mouse input, turned into game-level intent.
//!
//! The rest of the game never sees a `minifb::Key`: it only sees [`Controls`]
//! (continuous intent) and [`InputFrame`] (intent + one-shot events).
//! Physical keys live in [`KeyBindings`], which are user-configurable.

use std::collections::HashMap;

use minifb::{Key, Window};

/// Everything the player can ask the game to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    LookUp,
    LookDown,
    LookLeft,
    LookRight,
    Forward,
    Back,
    Left,
    Right,
    Jump,
    Sprint,
    Flashlight,
    Restart,
    Quit,
}

impl Action {
    pub const ALL: [Action; 13] = [
        Action::LookUp,
        Action::LookDown,
        Action::LookLeft,
        Action::LookRight,
        Action::Forward,
        Action::Back,
        Action::Left,
        Action::Right,
        Action::Jump,
        Action::Sprint,
        Action::Flashlight,
        Action::Restart,
        Action::Quit,
    ];

    /// Stable name used in the settings file.
    pub fn name(self) -> &'static str {
        match self {
            Action::LookUp => "look_up",
            Action::LookDown => "look_down",
            Action::LookLeft => "look_left",
            Action::LookRight => "look_right",
            Action::Forward => "forward",
            Action::Back => "back",
            Action::Left => "left",
            Action::Right => "right",
            Action::Jump => "jump",
            Action::Sprint => "sprint",
            Action::Flashlight => "flashlight",
            Action::Restart => "restart",
            Action::Quit => "quit",
        }
    }

    pub fn from_name(name: &str) -> Option<Action> {
        Action::ALL.iter().copied().find(|a| a.name() == name)
    }
}

// ------------------------------------------------------------------ key names

const KEY_NAMES: &[(&str, Key)] = &[
    ("A", Key::A), ("B", Key::B), ("C", Key::C), ("D", Key::D), ("E", Key::E),
    ("F", Key::F), ("G", Key::G), ("H", Key::H), ("I", Key::I), ("J", Key::J),
    ("K", Key::K), ("L", Key::L), ("M", Key::M), ("N", Key::N), ("O", Key::O),
    ("P", Key::P), ("Q", Key::Q), ("R", Key::R), ("S", Key::S), ("T", Key::T),
    ("U", Key::U), ("V", Key::V), ("W", Key::W), ("X", Key::X), ("Y", Key::Y),
    ("Z", Key::Z),
    ("Up", Key::Up), ("Down", Key::Down), ("Left", Key::Left), ("Right", Key::Right),
    ("LeftShift", Key::LeftShift), ("RightShift", Key::RightShift),
    ("Space", Key::Space), ("Escape", Key::Escape), ("Enter", Key::Enter), ("Tab", Key::Tab),
];

fn same_key(a: Key, b: Key) -> bool {
    std::mem::discriminant(&a) == std::mem::discriminant(&b)
}

/// Settings-file name of a key (`None` for keys we don't expose).
pub fn key_name(key: Key) -> Option<&'static str> {
    KEY_NAMES.iter().find(|(_, k)| same_key(*k, key)).map(|(n, _)| *n)
}

/// Parse a key name from the settings file (case-insensitive).
pub fn parse_key(name: &str) -> Option<Key> {
    let name = name.trim();
    KEY_NAMES
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, k)| *k)
}

// ------------------------------------------------------------------ key state

/// Anything that can tell whether a key is currently held. `minifb::Window`
/// implements it; tests use a fake.
pub trait KeyState {
    fn is_down(&self, key: Key) -> bool;
}

impl KeyState for Window {
    fn is_down(&self, key: Key) -> bool {
        self.is_key_down(key)
    }
}

// ------------------------------------------------------------------ bindings

#[derive(Clone)]
pub struct KeyBindings {
    map: HashMap<Action, Vec<Key>>,
}

impl Default for KeyBindings {
    fn default() -> Self {
        let mut map = HashMap::new();
        map.insert(Action::LookUp, vec![Key::W, Key::Up]);
        map.insert(Action::LookDown, vec![Key::S, Key::Down]);
        map.insert(Action::LookLeft, vec![Key::A, Key::Left]);
        map.insert(Action::LookRight, vec![Key::D, Key::Right]);
        map.insert(Action::Forward, vec![Key::I]);
        map.insert(Action::Back, vec![Key::K]);
        map.insert(Action::Left, vec![Key::J]);
        map.insert(Action::Right, vec![Key::L]);
        map.insert(Action::Jump, vec![Key::E]);
        map.insert(Action::Sprint, vec![Key::LeftShift, Key::RightShift]);
        map.insert(Action::Flashlight, vec![Key::F]);
        map.insert(Action::Restart, vec![Key::R]);
        map.insert(Action::Quit, vec![Key::Escape, Key::Q]);
        KeyBindings { map }
    }
}

impl KeyBindings {
    pub fn keys(&self, action: Action) -> &[Key] {
        self.map.get(&action).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub fn set(&mut self, action: Action, keys: Vec<Key>) {
        self.map.insert(action, keys);
    }

    pub fn is_down<K: KeyState + ?Sized>(&self, state: &K, action: Action) -> bool {
        self.keys(action).iter().any(|k| state.is_down(*k))
    }
}

// ------------------------------------------------------------------ intent

/// Continuous intent, consumed by `Player::update`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Controls {
    pub look_up: bool,
    pub look_down: bool,
    pub look_left: bool,
    pub look_right: bool,
    pub forward: bool,
    pub back: bool,
    pub left: bool,
    pub right: bool,
    pub jump: bool,
    pub sprint: bool,
    /// Yaw change from the mouse this frame, in radians (positive = turn right).
    pub mouse_yaw: f32,
    /// Pitch change from the mouse this frame, in radians (positive = look up).
    pub mouse_pitch: f32,
}

/// One frame of input: continuous controls plus one-shot events.
#[derive(Clone, Copy, Debug, Default)]
pub struct InputFrame {
    pub controls: Controls,
    pub toggle_flashlight: bool,
    pub restart: bool,
    pub quit: bool,
}

/// Polls devices once per frame and keeps the little bit of state needed for
/// edge detection (flashlight toggle) and mouse deltas.
pub struct Input {
    bindings: KeyBindings,
    mouse_sensitivity: f32,
    prev_flashlight: bool,
    last_mouse: Option<(f32, f32)>,
}

impl Input {
    /// `mouse_sensitivity` is radians per pixel; 0 disables mouse look.
    pub fn new(bindings: KeyBindings, mouse_sensitivity: f32) -> Self {
        Input { bindings, mouse_sensitivity, prev_flashlight: false, last_mouse: None }
    }

    pub fn bindings(&self) -> &KeyBindings {
        &self.bindings
    }

    pub fn poll<K: KeyState + ?Sized>(&mut self, keys: &K, mouse: Option<(f32, f32)>) -> InputFrame {
        let mut mouse_yaw = 0.0f32;
        let mut mouse_pitch = 0.0f32;
        if self.mouse_sensitivity > 0.0 {
            if let (Some((x, y)), Some((lx, ly))) = (mouse, self.last_mouse) {
                mouse_yaw = (x - lx) * self.mouse_sensitivity;
                mouse_pitch = -(y - ly) * self.mouse_sensitivity;
            }
            self.last_mouse = mouse;
        } else {
            self.last_mouse = None;
        }

        let b = &self.bindings;
        let controls = Controls {
            look_up: b.is_down(keys, Action::LookUp),
            look_down: b.is_down(keys, Action::LookDown),
            look_left: b.is_down(keys, Action::LookLeft),
            look_right: b.is_down(keys, Action::LookRight),
            forward: b.is_down(keys, Action::Forward),
            back: b.is_down(keys, Action::Back),
            left: b.is_down(keys, Action::Left),
            right: b.is_down(keys, Action::Right),
            jump: b.is_down(keys, Action::Jump),
            sprint: b.is_down(keys, Action::Sprint),
            mouse_yaw,
            mouse_pitch,
        };
        let flashlight_now = b.is_down(keys, Action::Flashlight);
        let restart = b.is_down(keys, Action::Restart);
        let quit = b.is_down(keys, Action::Quit);

        let toggle_flashlight = flashlight_now && !self.prev_flashlight;
        self.prev_flashlight = flashlight_now;

        InputFrame { controls, toggle_flashlight, restart, quit }
    }
}
