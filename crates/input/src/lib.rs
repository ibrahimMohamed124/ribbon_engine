//! Platform-independent physical keys and reusable key bindings.

pub mod gamepad;
pub use gamepad::{
    GamepadAxis, GamepadButton, GamepadEvent, GamepadSet, GamepadSettings, RawGamepadEvent,
    MAX_GAMEPADS,
};

use std::collections::HashMap;
use std::hash::Hash;

/// Platform-specific physical scan-code fallback for keys that a windowing backend does not
/// recognize as one of its standardized [`KeyCode`](https://docs.rs/winit/0.30.13/winit/keyboard/enum.KeyCode.html) values.
/// These values are suitable for local rebinding but are not portable between operating systems.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NativeKeyCode {
    Unidentified,
    Android(u32),
    MacOS(u16),
    Windows(u16),
    Xkb(u32),
}

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
    Digit0,
    Digit1,
    Digit2,
    Digit3,
    Digit4,
    Digit5,
    Digit6,
    Digit7,
    Digit8,
    Digit9,
    Numpad0,
    Numpad1,
    Numpad2,
    Numpad3,
    Numpad4,
    Numpad5,
    Numpad6,
    Numpad7,
    Numpad8,
    Numpad9,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    F13,
    F14,
    F15,
    F16,
    F17,
    F18,
    F19,
    F20,
    F21,
    F22,
    F23,
    F24,
    Up,
    Down,
    Left,
    Right,
    LeftControl,
    RightControl,
    LeftAlt,
    RightAlt,
    LeftShift,
    RightShift,
    LeftSuper,
    RightSuper,
    Space,
    Escape,
    Enter,
    Tab,
    Backspace,
    Delete,
    Insert,
    Home,
    End,
    PageUp,
    PageDown,
    Minus,
    Equal,
    BracketLeft,
    BracketRight,
    Backslash,
    Semicolon,
    Quote,
    Backquote,
    Comma,
    Period,
    Slash,
    CapsLock,
    NumLock,
    ScrollLock,
    PrintScreen,
    Pause,
    NumpadAdd,
    NumpadSubtract,
    NumpadMultiply,
    NumpadDivide,
    NumpadDecimal,
    NumpadEnter,
    NumpadEqual,
    NumpadComma,
    Abort,
    Again,
    AudioVolumeDown,
    AudioVolumeMute,
    AudioVolumeUp,
    BrowserBack,
    BrowserFavorites,
    BrowserForward,
    BrowserHome,
    BrowserRefresh,
    BrowserSearch,
    BrowserStop,
    ContextMenu,
    Convert,
    Copy,
    Cut,
    Eject,
    F25,
    F26,
    F27,
    F28,
    F29,
    F30,
    F31,
    F32,
    F33,
    F34,
    F35,
    Find,
    Fn,
    FnLock,
    Help,
    Hiragana,
    Hyper,
    IntlBackslash,
    IntlRo,
    IntlYen,
    KanaMode,
    Katakana,
    Lang1,
    Lang2,
    Lang3,
    Lang4,
    Lang5,
    LaunchApp1,
    LaunchApp2,
    LaunchMail,
    MediaPlayPause,
    MediaSelect,
    MediaStop,
    MediaTrackNext,
    MediaTrackPrevious,
    Meta,
    NonConvert,
    NumpadBackspace,
    NumpadClear,
    NumpadClearEntry,
    NumpadHash,
    NumpadMemoryAdd,
    NumpadMemoryClear,
    NumpadMemoryRecall,
    NumpadMemoryStore,
    NumpadMemorySubtract,
    NumpadParenLeft,
    NumpadParenRight,
    NumpadStar,
    Open,
    Paste,
    Power,
    Props,
    Resume,
    Select,
    Sleep,
    Suspend,
    Turbo,
    Undo,
    WakeUp,
    /// Raw platform-specific key identity for physical keys not represented by a standard code.
    Native(NativeKeyCode),
}

pub const ALL_KEYS: [Key; 194] = [
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
    Key::Digit0,
    Key::Digit1,
    Key::Digit2,
    Key::Digit3,
    Key::Digit4,
    Key::Digit5,
    Key::Digit6,
    Key::Digit7,
    Key::Digit8,
    Key::Digit9,
    Key::Numpad0,
    Key::Numpad1,
    Key::Numpad2,
    Key::Numpad3,
    Key::Numpad4,
    Key::Numpad5,
    Key::Numpad6,
    Key::Numpad7,
    Key::Numpad8,
    Key::Numpad9,
    Key::F1,
    Key::F2,
    Key::F3,
    Key::F4,
    Key::F5,
    Key::F6,
    Key::F7,
    Key::F8,
    Key::F9,
    Key::F10,
    Key::F11,
    Key::F12,
    Key::F13,
    Key::F14,
    Key::F15,
    Key::F16,
    Key::F17,
    Key::F18,
    Key::F19,
    Key::F20,
    Key::F21,
    Key::F22,
    Key::F23,
    Key::F24,
    Key::Up,
    Key::Down,
    Key::Left,
    Key::Right,
    Key::LeftControl,
    Key::RightControl,
    Key::LeftAlt,
    Key::RightAlt,
    Key::LeftShift,
    Key::RightShift,
    Key::LeftSuper,
    Key::RightSuper,
    Key::Space,
    Key::Escape,
    Key::Enter,
    Key::Tab,
    Key::Backspace,
    Key::Delete,
    Key::Insert,
    Key::Home,
    Key::End,
    Key::PageUp,
    Key::PageDown,
    Key::Minus,
    Key::Equal,
    Key::BracketLeft,
    Key::BracketRight,
    Key::Backslash,
    Key::Semicolon,
    Key::Quote,
    Key::Backquote,
    Key::Comma,
    Key::Period,
    Key::Slash,
    Key::CapsLock,
    Key::NumLock,
    Key::ScrollLock,
    Key::PrintScreen,
    Key::Pause,
    Key::NumpadAdd,
    Key::NumpadSubtract,
    Key::NumpadMultiply,
    Key::NumpadDivide,
    Key::NumpadDecimal,
    Key::NumpadEnter,
    Key::NumpadEqual,
    Key::NumpadComma,
    Key::Abort,
    Key::Again,
    Key::AudioVolumeDown,
    Key::AudioVolumeMute,
    Key::AudioVolumeUp,
    Key::BrowserBack,
    Key::BrowserFavorites,
    Key::BrowserForward,
    Key::BrowserHome,
    Key::BrowserRefresh,
    Key::BrowserSearch,
    Key::BrowserStop,
    Key::ContextMenu,
    Key::Convert,
    Key::Copy,
    Key::Cut,
    Key::Eject,
    Key::F25,
    Key::F26,
    Key::F27,
    Key::F28,
    Key::F29,
    Key::F30,
    Key::F31,
    Key::F32,
    Key::F33,
    Key::F34,
    Key::F35,
    Key::Find,
    Key::Fn,
    Key::FnLock,
    Key::Help,
    Key::Hiragana,
    Key::Hyper,
    Key::IntlBackslash,
    Key::IntlRo,
    Key::IntlYen,
    Key::KanaMode,
    Key::Katakana,
    Key::Lang1,
    Key::Lang2,
    Key::Lang3,
    Key::Lang4,
    Key::Lang5,
    Key::LaunchApp1,
    Key::LaunchApp2,
    Key::LaunchMail,
    Key::MediaPlayPause,
    Key::MediaSelect,
    Key::MediaStop,
    Key::MediaTrackNext,
    Key::MediaTrackPrevious,
    Key::Meta,
    Key::NonConvert,
    Key::NumpadBackspace,
    Key::NumpadClear,
    Key::NumpadClearEntry,
    Key::NumpadHash,
    Key::NumpadMemoryAdd,
    Key::NumpadMemoryClear,
    Key::NumpadMemoryRecall,
    Key::NumpadMemoryStore,
    Key::NumpadMemorySubtract,
    Key::NumpadParenLeft,
    Key::NumpadParenRight,
    Key::NumpadStar,
    Key::Open,
    Key::Paste,
    Key::Power,
    Key::Props,
    Key::Resume,
    Key::Select,
    Key::Sleep,
    Key::Suspend,
    Key::Turbo,
    Key::Undo,
    Key::WakeUp,
];

pub trait KeyState {
    fn is_down(&self, key: Key) -> bool;
}

/// Queries a runtime input snapshot for any source supported by an action binding.
pub trait ActionState: KeyState {
    fn is_mouse_button_down(&self, _button: MouseButton) -> bool {
        false
    }
    fn is_gamepad_button_down(&self, _pad: u8, _button: u16) -> bool {
        false
    }
    fn gamepad_axis_value(&self, _pad: u8, _axis: u8) -> f32 {
        0.0
    }
}

/// Platform-neutral physical key state with frame edges and focus-loss cleanup.
#[derive(Default, Debug)]
pub struct InputState {
    down: std::collections::HashSet<Key>,
    pressed: Vec<Key>,
    released: Vec<Key>,
}

impl InputState {
    pub fn begin_frame(&mut self) {
        self.pressed.clear();
        self.released.clear();
    }
    pub fn press_key(&mut self, key: Key, repeat: bool) {
        if self.down.insert(key) && !repeat {
            self.pressed.push(key);
        }
    }
    pub fn release_key(&mut self, key: Key) {
        if self.down.remove(&key) {
            self.released.push(key);
        }
    }
    pub fn pressed(&self) -> &[Key] {
        &self.pressed
    }
    pub fn released(&self) -> &[Key] {
        &self.released
    }
    pub fn is_just_pressed(&self, key: Key) -> bool {
        self.pressed.contains(&key)
    }
    pub fn is_just_released(&self, key: Key) -> bool {
        self.released.contains(&key)
    }
    pub fn take_pressed(&mut self) -> Vec<Key> {
        std::mem::take(&mut self.pressed)
    }
    /// Release held keys without discarding other edges already recorded this frame.
    pub fn release_all(&mut self) {
        self.released.extend(self.down.drain());
    }
    /// Clear all held keys when the application loses focus so keys cannot remain stuck.
    pub fn focus_lost(&mut self) {
        self.released.extend(self.down.drain());
        self.pressed.clear();
    }
    pub fn clear(&mut self) {
        self.down.clear();
        self.pressed.clear();
        self.released.clear();
    }
}

impl KeyState for InputState {
    fn is_down(&self, key: Key) -> bool {
        self.down.contains(&key)
    }
}

pub trait MouseSource {
    fn delta(&mut self) -> (f32, f32);
}

/// Platform-independent mouse button identity. OS-specific button values are mapped by the platform crate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Other(u16),
}

/// Accumulated mouse state with per-frame button edges, pointer deltas, position, and scroll.
#[derive(Default, Debug)]
pub struct MouseInputState {
    down: std::collections::HashSet<MouseButton>,
    pressed: std::collections::HashSet<MouseButton>,
    released: std::collections::HashSet<MouseButton>,
    delta: (f32, f32),
    scroll: f32,
    position: Option<(f32, f32)>,
}

impl MouseInputState {
    pub fn begin_frame(&mut self) {
        self.pressed.clear();
        self.released.clear();
        self.delta = (0.0, 0.0);
        self.scroll = 0.0;
    }
    pub fn press_button(&mut self, button: MouseButton) {
        if self.down.insert(button) {
            self.pressed.insert(button);
        }
    }
    pub fn release_button(&mut self, button: MouseButton) {
        if self.down.remove(&button) {
            self.released.insert(button);
        }
    }
    pub fn is_down(&self, button: MouseButton) -> bool {
        self.down.contains(&button)
    }
    pub fn just_pressed(&self, button: MouseButton) -> bool {
        self.pressed.contains(&button)
    }
    pub fn just_released(&self, button: MouseButton) -> bool {
        self.released.contains(&button)
    }
    pub fn delta(&self) -> (f32, f32) {
        self.delta
    }
    pub fn scroll(&self) -> f32 {
        self.scroll
    }
    pub fn position(&self) -> Option<(f32, f32)> {
        self.position
    }
    pub fn apply_motion(&mut self, delta: (f32, f32), position: Option<(f32, f32)>, scroll: f32) {
        self.delta.0 += delta.0;
        self.delta.1 += delta.1;
        self.scroll += scroll;
        if position.is_some() {
            self.position = position;
        }
    }
    /// Release held buttons without discarding edge events already recorded this frame.
    pub fn release_all(&mut self) {
        self.released.extend(self.down.drain());
    }

    /// Clear held state on focus loss so a press that began before deactivation cannot stick.
    pub fn focus_lost(&mut self) {
        self.released.extend(self.down.drain());
        self.pressed.clear();
        self.delta = (0.0, 0.0);
        self.scroll = 0.0;
        self.position = None;
    }

    pub fn clear(&mut self) {
        self.down.clear();
        self.pressed.clear();
        self.released.clear();
        self.delta = (0.0, 0.0);
        self.scroll = 0.0;
        self.position = None;
    }
}

pub struct Bindings<A> {
    // Keep the original key view for compatibility with settings files and game code while
    // storing full binding sources for mouse/gamepad-capable callers.
    map: HashMap<A, Vec<Key>>,
    sources: HashMap<A, Vec<BindingSource>>,
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
            sources: self.sources.clone(),
        }
    }
}

impl<A: Eq + Hash> Bindings<A> {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
            sources: HashMap::new(),
        }
    }

    pub fn keys(&self, action: A) -> &[Key] {
        self.map.get(&action).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn set(&mut self, action: A, keys: Vec<Key>)
    where
        A: Clone,
    {
        self.sources.insert(
            action.clone(),
            keys.iter().copied().map(BindingSource::Key).collect(),
        );
        self.map.insert(action, keys);
    }

    /// Return all physical sources bound to an action. Key-only callers can keep using `keys`.
    pub fn sources(&self, action: A) -> &[BindingSource] {
        self.sources.get(&action).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Replace an action's physical sources. This is the extensible path for mouse/gamepad inputs.
    pub fn set_sources(&mut self, action: A, sources: Vec<BindingSource>)
    where
        A: Clone,
    {
        let keys = sources
            .iter()
            .filter_map(|source| match source {
                BindingSource::Key(key) => Some(*key),
                BindingSource::MouseButton(_)
                | BindingSource::GamepadButton { .. }
                | BindingSource::GamepadAxis { .. } => None,
            })
            .collect();
        self.map.insert(action.clone(), keys);
        self.sources.insert(action, sources);
    }

    /// Apply a rebinding captured by a settings UI or other input configuration tool.
    pub fn apply_rebinding(&mut self, rebinding: Rebinding<A>)
    where
        A: Clone,
    {
        self.set_sources(rebinding.action, rebinding.sources);
    }

    pub fn is_down<K: KeyState + ?Sized>(&self, state: &K, action: A) -> bool {
        self.keys(action).iter().any(|key| state.is_down(*key))
    }

    /// Test every source bound to an action. Unsupported device sources stay inactive by default.
    pub fn is_active<S: ActionState + ?Sized>(&self, state: &S, action: A) -> bool {
        self.sources(action).iter().any(|source| match source {
            BindingSource::Key(key) => state.is_down(*key),
            BindingSource::MouseButton(button) => state.is_mouse_button_down(*button),
            BindingSource::GamepadButton { pad, button } => {
                state.is_gamepad_button_down(*pad, *button)
            }
            BindingSource::GamepadAxis {
                pad,
                axis,
                positive,
                threshold_milli,
            } => {
                let v = state.gamepad_axis_value(*pad, *axis);
                let t = *threshold_milli as f32 / 1000.0;
                if *positive {
                    v >= t
                } else {
                    v <= -t
                }
            }
        })
    }

    /// Return the strongest signed value among this action's gamepad-axis sources.
    pub fn axis_value<S: ActionState + ?Sized>(&self, state: &S, action: A) -> f32 {
        self.sources(action)
            .iter()
            .filter_map(|source| match source {
                BindingSource::GamepadAxis {
                    pad,
                    axis,
                    positive,
                    ..
                } => {
                    let v = state.gamepad_axis_value(*pad, *axis);
                    Some(if *positive { v.max(0.0) } else { v.min(0.0) })
                }
                _ => None,
            })
            .max_by(|a, b| a.abs().total_cmp(&b.abs()))
            .unwrap_or(0.0)
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
    ("0", Key::Digit0),
    ("1", Key::Digit1),
    ("2", Key::Digit2),
    ("3", Key::Digit3),
    ("4", Key::Digit4),
    ("5", Key::Digit5),
    ("6", Key::Digit6),
    ("7", Key::Digit7),
    ("8", Key::Digit8),
    ("9", Key::Digit9),
    ("Num0", Key::Numpad0),
    ("Num1", Key::Numpad1),
    ("Num2", Key::Numpad2),
    ("Num3", Key::Numpad3),
    ("Num4", Key::Numpad4),
    ("Num5", Key::Numpad5),
    ("Num6", Key::Numpad6),
    ("Num7", Key::Numpad7),
    ("Num8", Key::Numpad8),
    ("Num9", Key::Numpad9),
    ("F1", Key::F1),
    ("F2", Key::F2),
    ("F3", Key::F3),
    ("F4", Key::F4),
    ("F5", Key::F5),
    ("F6", Key::F6),
    ("F7", Key::F7),
    ("F8", Key::F8),
    ("F9", Key::F9),
    ("F10", Key::F10),
    ("F11", Key::F11),
    ("F12", Key::F12),
    ("F13", Key::F13),
    ("F14", Key::F14),
    ("F15", Key::F15),
    ("F16", Key::F16),
    ("F17", Key::F17),
    ("F18", Key::F18),
    ("F19", Key::F19),
    ("F20", Key::F20),
    ("F21", Key::F21),
    ("F22", Key::F22),
    ("F23", Key::F23),
    ("F24", Key::F24),
    ("Up", Key::Up),
    ("Down", Key::Down),
    ("Left", Key::Left),
    ("Right", Key::Right),
    ("LeftControl", Key::LeftControl),
    ("RightControl", Key::RightControl),
    ("LeftAlt", Key::LeftAlt),
    ("RightAlt", Key::RightAlt),
    ("LeftShift", Key::LeftShift),
    ("RightShift", Key::RightShift),
    ("LeftSuper", Key::LeftSuper),
    ("RightSuper", Key::RightSuper),
    ("Space", Key::Space),
    ("Escape", Key::Escape),
    ("Enter", Key::Enter),
    ("Tab", Key::Tab),
    ("Backspace", Key::Backspace),
    ("Delete", Key::Delete),
    ("Insert", Key::Insert),
    ("Home", Key::Home),
    ("End", Key::End),
    ("PageUp", Key::PageUp),
    ("PageDown", Key::PageDown),
    ("Minus", Key::Minus),
    ("Equal", Key::Equal),
    ("BracketLeft", Key::BracketLeft),
    ("BracketRight", Key::BracketRight),
    ("Backslash", Key::Backslash),
    ("Semicolon", Key::Semicolon),
    ("Quote", Key::Quote),
    ("Backquote", Key::Backquote),
    ("Comma", Key::Comma),
    ("Period", Key::Period),
    ("Slash", Key::Slash),
    ("CapsLock", Key::CapsLock),
    ("NumLock", Key::NumLock),
    ("ScrollLock", Key::ScrollLock),
    ("PrintScreen", Key::PrintScreen),
    ("Pause", Key::Pause),
    ("NumAdd", Key::NumpadAdd),
    ("NumSubtract", Key::NumpadSubtract),
    ("NumMultiply", Key::NumpadMultiply),
    ("NumDivide", Key::NumpadDivide),
    ("NumDecimal", Key::NumpadDecimal),
    ("NumEnter", Key::NumpadEnter),
    ("NumEqual", Key::NumpadEqual),
    ("NumComma", Key::NumpadComma),
    ("Abort", Key::Abort),
    ("Again", Key::Again),
    ("AudioVolumeDown", Key::AudioVolumeDown),
    ("AudioVolumeMute", Key::AudioVolumeMute),
    ("AudioVolumeUp", Key::AudioVolumeUp),
    ("BrowserBack", Key::BrowserBack),
    ("BrowserFavorites", Key::BrowserFavorites),
    ("BrowserForward", Key::BrowserForward),
    ("BrowserHome", Key::BrowserHome),
    ("BrowserRefresh", Key::BrowserRefresh),
    ("BrowserSearch", Key::BrowserSearch),
    ("BrowserStop", Key::BrowserStop),
    ("ContextMenu", Key::ContextMenu),
    ("Convert", Key::Convert),
    ("Copy", Key::Copy),
    ("Cut", Key::Cut),
    ("Eject", Key::Eject),
    ("F25", Key::F25),
    ("F26", Key::F26),
    ("F27", Key::F27),
    ("F28", Key::F28),
    ("F29", Key::F29),
    ("F30", Key::F30),
    ("F31", Key::F31),
    ("F32", Key::F32),
    ("F33", Key::F33),
    ("F34", Key::F34),
    ("F35", Key::F35),
    ("Find", Key::Find),
    ("Fn", Key::Fn),
    ("FnLock", Key::FnLock),
    ("Help", Key::Help),
    ("Hiragana", Key::Hiragana),
    ("Hyper", Key::Hyper),
    ("IntlBackslash", Key::IntlBackslash),
    ("IntlRo", Key::IntlRo),
    ("IntlYen", Key::IntlYen),
    ("KanaMode", Key::KanaMode),
    ("Katakana", Key::Katakana),
    ("Lang1", Key::Lang1),
    ("Lang2", Key::Lang2),
    ("Lang3", Key::Lang3),
    ("Lang4", Key::Lang4),
    ("Lang5", Key::Lang5),
    ("LaunchApp1", Key::LaunchApp1),
    ("LaunchApp2", Key::LaunchApp2),
    ("LaunchMail", Key::LaunchMail),
    ("MediaPlayPause", Key::MediaPlayPause),
    ("MediaSelect", Key::MediaSelect),
    ("MediaStop", Key::MediaStop),
    ("MediaTrackNext", Key::MediaTrackNext),
    ("MediaTrackPrevious", Key::MediaTrackPrevious),
    ("Meta", Key::Meta),
    ("NonConvert", Key::NonConvert),
    ("NumpadBackspace", Key::NumpadBackspace),
    ("NumpadClear", Key::NumpadClear),
    ("NumpadClearEntry", Key::NumpadClearEntry),
    ("NumpadHash", Key::NumpadHash),
    ("NumpadMemoryAdd", Key::NumpadMemoryAdd),
    ("NumpadMemoryClear", Key::NumpadMemoryClear),
    ("NumpadMemoryRecall", Key::NumpadMemoryRecall),
    ("NumpadMemoryStore", Key::NumpadMemoryStore),
    ("NumpadMemorySubtract", Key::NumpadMemorySubtract),
    ("NumpadParenLeft", Key::NumpadParenLeft),
    ("NumpadParenRight", Key::NumpadParenRight),
    ("NumpadStar", Key::NumpadStar),
    ("Open", Key::Open),
    ("Paste", Key::Paste),
    ("Power", Key::Power),
    ("Props", Key::Props),
    ("Resume", Key::Resume),
    ("Select", Key::Select),
    ("Sleep", Key::Sleep),
    ("Suspend", Key::Suspend),
    ("Turbo", Key::Turbo),
    ("Undo", Key::Undo),
    ("WakeUp", Key::WakeUp),
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

/// Generic action-to-binding map; kept as an alias for compatibility with the existing API.
pub type ActionMap<A> = Bindings<A>;

/// A user-requested replacement for an action's physical bindings. The source enum is extensible
/// so future devices can add variants without redesigning the rebinding data model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rebinding<A> {
    pub action: A,
    pub sources: Vec<BindingSource>,
}

/// Binding source for physical keyboard, mouse, and gamepad controls.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BindingSource {
    Key(Key),
    MouseButton(MouseButton),
    GamepadButton {
        pad: u8,
        button: u16,
    },
    GamepadAxis {
        pad: u8,
        axis: u8,
        positive: bool,
        threshold_milli: u16,
    },
}
impl BindingSource {
    pub fn gamepad_button(pad: u8, button: GamepadButton) -> Self {
        Self::GamepadButton {
            pad,
            button: button.id(),
        }
    }
    pub fn gamepad_axis(pad: u8, axis: GamepadAxis, positive: bool, threshold: f32) -> Self {
        Self::GamepadAxis {
            pad,
            axis: axis.id(),
            positive,
            threshold_milli: (threshold.clamp(0.001, 1.0) * 1000.0).round() as u16,
        }
    }
}

/// Small FIFO for input actions that must survive until the next simulation step.
#[derive(Clone, Debug)]
pub struct BufferedInput<T> {
    events: std::collections::VecDeque<T>,
}

impl<T> Default for BufferedInput<T> {
    fn default() -> Self {
        Self {
            events: std::collections::VecDeque::new(),
        }
    }
}

impl<T> BufferedInput<T> {
    pub fn push(&mut self, event: T) {
        self.events.push_back(event);
    }
    pub fn pop(&mut self) -> Option<T> {
        self.events.pop_front()
    }
    pub fn drain(&mut self) -> impl Iterator<Item = T> + '_ {
        self.events.drain(..)
    }
    pub fn clear(&mut self) {
        self.events.clear();
    }
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Merge a new frame into the newest buffered item, inserting it when the buffer is empty.
    /// This is useful for fixed-step applications that coalesce render-frame input while retaining
    /// their own domain-specific merge policy.
    pub fn accumulate_with(&mut self, value: T, merge: impl FnOnce(&mut T, T)) {
        if let Some(current) = self.events.back_mut() {
            merge(current, value);
        } else {
            self.events.push_back(value);
        }
    }
}

/// Scripted input queue for deterministic tools, replays, and tests.
#[derive(Clone, Debug)]
pub struct VirtualInput<T> {
    events: std::collections::VecDeque<T>,
}
impl<T> Default for VirtualInput<T> {
    fn default() -> Self {
        Self {
            events: std::collections::VecDeque::new(),
        }
    }
}
impl<T> VirtualInput<T> {
    pub fn push(&mut self, event: T) {
        self.events.push_back(event);
    }
    pub fn pop(&mut self) -> Option<T> {
        self.events.pop_front()
    }
    pub fn drain(&mut self) -> impl Iterator<Item = T> + '_ {
        self.events.drain(..)
    }
    pub fn clear(&mut self) {
        self.events.clear();
    }
}

#[cfg(test)]
mod physical_key_catalog_tests {
    use super::{key_name, parse_key, ALL_KEYS};
    use std::collections::HashSet;

    #[test]
    fn every_supported_physical_key_has_a_unique_round_trip_name() {
        assert_eq!(ALL_KEYS.len(), 194);
        let mut names = HashSet::with_capacity(ALL_KEYS.len());
        for key in ALL_KEYS {
            let name = key_name(key).expect("every physical key must have a stable name");
            assert!(names.insert(name), "duplicate physical key name: {name}");
            assert_eq!(parse_key(name), Some(key));
            assert_eq!(parse_key(&name.to_ascii_lowercase()), Some(key));
        }
        assert_eq!(names.len(), ALL_KEYS.len());
    }
}

#[cfg(test)]
mod state_tests {
    use super::{InputState, Key, KeyState};
    #[test]
    fn held_keys_and_edges_clear_on_focus_loss() {
        let mut input = InputState::default();
        input.press_key(Key::W, false);
        input.press_key(Key::W, true);
        assert!(input.is_down(Key::W));
        assert_eq!(input.pressed(), &[Key::W]);
        input.begin_frame();
        assert!(input.pressed().is_empty());
        assert!(input.is_down(Key::W));
        input.focus_lost();
        assert!(!input.is_down(Key::W));
        assert_eq!(input.released(), &[Key::W]);
    }

    #[test]
    fn unidentified_native_keys_participate_in_normal_input_state() {
        use super::NativeKeyCode;

        let key = Key::Native(NativeKeyCode::Windows(0x2a));
        let mut input = InputState::default();
        input.press_key(key, false);
        assert!(input.is_down(key));
        assert!(input.is_just_pressed(key));
        input.release_key(key);
        assert!(!input.is_down(key));
        assert!(input.is_just_released(key));
        assert_eq!(super::key_name(key), None);
    }
}

#[cfg(test)]
mod shared_input_tests {
    use super::{
        ActionMap, BindingSource, BufferedInput, InputState, Key, KeyState, MouseButton,
        MouseInputState, Rebinding,
    };

    struct CombinedInput {
        keys: InputState,
        mouse: MouseInputState,
    }
    impl KeyState for CombinedInput {
        fn is_down(&self, key: Key) -> bool {
            self.keys.is_down(key)
        }
    }
    impl super::ActionState for CombinedInput {
        fn is_mouse_button_down(&self, button: MouseButton) -> bool {
            self.mouse.is_down(button)
        }
    }

    #[test]
    fn mouse_state_tracks_edges_motion_scroll_and_focus_release() {
        let mut mouse = MouseInputState::default();
        mouse.begin_frame();
        mouse.press_button(MouseButton::Other(4));
        mouse.apply_motion((3.0, -2.0), Some((10.0, 20.0)), 1.5);
        assert!(mouse.is_down(MouseButton::Other(4)));
        assert!(mouse.just_pressed(MouseButton::Other(4)));
        assert_eq!(mouse.delta(), (3.0, -2.0));
        assert_eq!(mouse.scroll(), 1.5);
        assert_eq!(mouse.position(), Some((10.0, 20.0)));
        mouse.begin_frame();
        mouse.focus_lost();
        assert!(!mouse.is_down(MouseButton::Other(4)));
        assert!(mouse.just_released(MouseButton::Other(4)));
    }

    #[test]
    fn focus_loss_discards_press_edges_and_releases_held_mouse_buttons() {
        let mut mouse = MouseInputState::default();
        mouse.press_button(MouseButton::Left);
        mouse.apply_motion((4.0, 2.0), Some((20.0, 10.0)), 1.0);
        mouse.focus_lost();
        assert!(!mouse.is_down(MouseButton::Left));
        assert!(!mouse.just_pressed(MouseButton::Left));
        assert!(mouse.just_released(MouseButton::Left));
        assert_eq!(mouse.delta(), (0.0, 0.0));
        assert_eq!(mouse.scroll(), 0.0);
    }

    #[test]
    fn action_map_supports_key_and_mouse_binding_sources_without_gamepad_dependency() {
        let mut state = CombinedInput {
            keys: InputState::default(),
            mouse: MouseInputState::default(),
        };
        state.keys.press_key(Key::W, false);
        let mut map = ActionMap::new();
        map.apply_rebinding(Rebinding {
            action: "move",
            sources: vec![
                BindingSource::Key(Key::W),
                BindingSource::MouseButton(MouseButton::Left),
            ],
        });
        assert!(map.is_active(&state, "move"));
        state.keys.release_key(Key::W);
        state.mouse.press_button(MouseButton::Left);
        assert!(map.is_active(&state, "move"));
        assert_eq!(map.keys("move"), &[Key::W]);
    }

    #[test]
    fn buffered_input_can_coalesce_frames_without_losing_domain_merge_rules() {
        let mut buffer = BufferedInput::default();
        buffer.accumulate_with((1_u32, 2_u32), |current, next| {
            current.0 += next.0;
            current.1 = next.1;
        });
        buffer.accumulate_with((3, 4), |current, next| {
            current.0 += next.0;
            current.1 = next.1;
        });
        assert_eq!(buffer.len(), 1);
        assert_eq!(buffer.pop(), Some((4, 4)));
        assert!(buffer.is_empty());
    }
}
