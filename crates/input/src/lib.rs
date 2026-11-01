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
    F1,
}

pub const ALL_KEYS: [Key; 37] = [
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
    Key::F1,
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
                BindingSource::MouseButton(_) | BindingSource::GamepadButton { .. } => None,
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
        })
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
    ("F1", Key::F1),
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

/// Binding source is extensible without committing the engine to gamepad support today.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BindingSource {
    Key(Key),
    MouseButton(MouseButton),
    /// Reserved extension point for a future gamepad integration.
    GamepadButton {
        pad: u8,
        button: u16,
    },
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
