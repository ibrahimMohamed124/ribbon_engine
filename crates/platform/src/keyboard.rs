//! Physical keyboard and mouse event state for the desktop application.

use crate::config::{MouseButtons, MouseInput};
use crate::input::{
    InputState, Key, KeyState, MouseButton as EngineMouseButton, MouseInputState, NativeKeyCode,
};
use winit::event::{ElementState, MouseButton, MouseScrollDelta};
use winit::keyboard::{KeyCode, NativeKeyCode as WinitNativeKeyCode, PhysicalKey};

#[derive(Default)]
pub struct KeyboardState {
    state: InputState,
}

impl KeyboardState {
    pub fn process_key(&mut self, key: Key, state: ElementState, repeat: bool) {
        match state {
            ElementState::Pressed => self.state.press_key(key, repeat),
            ElementState::Released => self.state.release_key(key),
        }
    }

    pub fn process_winit_key(&mut self, key: KeyCode, state: ElementState, repeat: bool) {
        if let Some(key) = from_winit(key) {
            self.process_key(key, state, repeat);
        }
    }

    pub fn pressed(&self) -> &[Key] {
        self.state.pressed()
    }

    pub fn take_pressed(&mut self) -> Vec<Key> {
        self.state.take_pressed()
    }

    pub fn clear(&mut self) {
        self.state.focus_lost();
    }
}

impl KeyState for KeyboardState {
    fn is_down(&self, key: Key) -> bool {
        self.state.is_down(key)
    }
}

/// Map physical keys, preserving physical-position controls across keyboard layouts.
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
        KeyCode::Digit0 => Key::Digit0,
        KeyCode::Digit1 => Key::Digit1,
        KeyCode::Digit2 => Key::Digit2,
        KeyCode::Digit3 => Key::Digit3,
        KeyCode::Digit4 => Key::Digit4,
        KeyCode::Digit5 => Key::Digit5,
        KeyCode::Digit6 => Key::Digit6,
        KeyCode::Digit7 => Key::Digit7,
        KeyCode::Digit8 => Key::Digit8,
        KeyCode::Digit9 => Key::Digit9,
        KeyCode::Numpad0 => Key::Numpad0,
        KeyCode::Numpad1 => Key::Numpad1,
        KeyCode::Numpad2 => Key::Numpad2,
        KeyCode::Numpad3 => Key::Numpad3,
        KeyCode::Numpad4 => Key::Numpad4,
        KeyCode::Numpad5 => Key::Numpad5,
        KeyCode::Numpad6 => Key::Numpad6,
        KeyCode::Numpad7 => Key::Numpad7,
        KeyCode::Numpad8 => Key::Numpad8,
        KeyCode::Numpad9 => Key::Numpad9,
        KeyCode::F1 => Key::F1,
        KeyCode::F2 => Key::F2,
        KeyCode::F3 => Key::F3,
        KeyCode::F4 => Key::F4,
        KeyCode::F5 => Key::F5,
        KeyCode::F6 => Key::F6,
        KeyCode::F7 => Key::F7,
        KeyCode::F8 => Key::F8,
        KeyCode::F9 => Key::F9,
        KeyCode::F10 => Key::F10,
        KeyCode::F11 => Key::F11,
        KeyCode::F12 => Key::F12,
        KeyCode::F13 => Key::F13,
        KeyCode::F14 => Key::F14,
        KeyCode::F15 => Key::F15,
        KeyCode::F16 => Key::F16,
        KeyCode::F17 => Key::F17,
        KeyCode::F18 => Key::F18,
        KeyCode::F19 => Key::F19,
        KeyCode::F20 => Key::F20,
        KeyCode::F21 => Key::F21,
        KeyCode::F22 => Key::F22,
        KeyCode::F23 => Key::F23,
        KeyCode::F24 => Key::F24,
        KeyCode::ArrowUp => Key::Up,
        KeyCode::ArrowDown => Key::Down,
        KeyCode::ArrowLeft => Key::Left,
        KeyCode::ArrowRight => Key::Right,
        KeyCode::ControlLeft => Key::LeftControl,
        KeyCode::ControlRight => Key::RightControl,
        KeyCode::AltLeft => Key::LeftAlt,
        KeyCode::AltRight => Key::RightAlt,
        KeyCode::ShiftLeft => Key::LeftShift,
        KeyCode::ShiftRight => Key::RightShift,
        KeyCode::SuperLeft => Key::LeftSuper,
        KeyCode::SuperRight => Key::RightSuper,
        KeyCode::Space => Key::Space,
        KeyCode::Escape => Key::Escape,
        KeyCode::Enter => Key::Enter,
        KeyCode::Tab => Key::Tab,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Delete => Key::Delete,
        KeyCode::Insert => Key::Insert,
        KeyCode::Home => Key::Home,
        KeyCode::End => Key::End,
        KeyCode::PageUp => Key::PageUp,
        KeyCode::PageDown => Key::PageDown,
        KeyCode::Minus => Key::Minus,
        KeyCode::Equal => Key::Equal,
        KeyCode::BracketLeft => Key::BracketLeft,
        KeyCode::BracketRight => Key::BracketRight,
        KeyCode::Backslash => Key::Backslash,
        KeyCode::Semicolon => Key::Semicolon,
        KeyCode::Quote => Key::Quote,
        KeyCode::Backquote => Key::Backquote,
        KeyCode::Comma => Key::Comma,
        KeyCode::Period => Key::Period,
        KeyCode::Slash => Key::Slash,
        KeyCode::CapsLock => Key::CapsLock,
        KeyCode::NumLock => Key::NumLock,
        KeyCode::ScrollLock => Key::ScrollLock,
        KeyCode::PrintScreen => Key::PrintScreen,
        KeyCode::Pause => Key::Pause,
        KeyCode::NumpadAdd => Key::NumpadAdd,
        KeyCode::NumpadSubtract => Key::NumpadSubtract,
        KeyCode::NumpadMultiply => Key::NumpadMultiply,
        KeyCode::NumpadDivide => Key::NumpadDivide,
        KeyCode::NumpadDecimal => Key::NumpadDecimal,
        KeyCode::NumpadEnter => Key::NumpadEnter,
        KeyCode::NumpadEqual => Key::NumpadEqual,
        KeyCode::NumpadComma => Key::NumpadComma,
        KeyCode::Abort => Key::Abort,
        KeyCode::Again => Key::Again,
        KeyCode::AudioVolumeDown => Key::AudioVolumeDown,
        KeyCode::AudioVolumeMute => Key::AudioVolumeMute,
        KeyCode::AudioVolumeUp => Key::AudioVolumeUp,
        KeyCode::BrowserBack => Key::BrowserBack,
        KeyCode::BrowserFavorites => Key::BrowserFavorites,
        KeyCode::BrowserForward => Key::BrowserForward,
        KeyCode::BrowserHome => Key::BrowserHome,
        KeyCode::BrowserRefresh => Key::BrowserRefresh,
        KeyCode::BrowserSearch => Key::BrowserSearch,
        KeyCode::BrowserStop => Key::BrowserStop,
        KeyCode::ContextMenu => Key::ContextMenu,
        KeyCode::Convert => Key::Convert,
        KeyCode::Copy => Key::Copy,
        KeyCode::Cut => Key::Cut,
        KeyCode::Eject => Key::Eject,
        KeyCode::F25 => Key::F25,
        KeyCode::F26 => Key::F26,
        KeyCode::F27 => Key::F27,
        KeyCode::F28 => Key::F28,
        KeyCode::F29 => Key::F29,
        KeyCode::F30 => Key::F30,
        KeyCode::F31 => Key::F31,
        KeyCode::F32 => Key::F32,
        KeyCode::F33 => Key::F33,
        KeyCode::F34 => Key::F34,
        KeyCode::F35 => Key::F35,
        KeyCode::Find => Key::Find,
        KeyCode::Fn => Key::Fn,
        KeyCode::FnLock => Key::FnLock,
        KeyCode::Help => Key::Help,
        KeyCode::Hiragana => Key::Hiragana,
        KeyCode::Hyper => Key::Hyper,
        KeyCode::IntlBackslash => Key::IntlBackslash,
        KeyCode::IntlRo => Key::IntlRo,
        KeyCode::IntlYen => Key::IntlYen,
        KeyCode::KanaMode => Key::KanaMode,
        KeyCode::Katakana => Key::Katakana,
        KeyCode::Lang1 => Key::Lang1,
        KeyCode::Lang2 => Key::Lang2,
        KeyCode::Lang3 => Key::Lang3,
        KeyCode::Lang4 => Key::Lang4,
        KeyCode::Lang5 => Key::Lang5,
        KeyCode::LaunchApp1 => Key::LaunchApp1,
        KeyCode::LaunchApp2 => Key::LaunchApp2,
        KeyCode::LaunchMail => Key::LaunchMail,
        KeyCode::MediaPlayPause => Key::MediaPlayPause,
        KeyCode::MediaSelect => Key::MediaSelect,
        KeyCode::MediaStop => Key::MediaStop,
        KeyCode::MediaTrackNext => Key::MediaTrackNext,
        KeyCode::MediaTrackPrevious => Key::MediaTrackPrevious,
        KeyCode::Meta => Key::Meta,
        KeyCode::NonConvert => Key::NonConvert,
        KeyCode::NumpadBackspace => Key::NumpadBackspace,
        KeyCode::NumpadClear => Key::NumpadClear,
        KeyCode::NumpadClearEntry => Key::NumpadClearEntry,
        KeyCode::NumpadHash => Key::NumpadHash,
        KeyCode::NumpadMemoryAdd => Key::NumpadMemoryAdd,
        KeyCode::NumpadMemoryClear => Key::NumpadMemoryClear,
        KeyCode::NumpadMemoryRecall => Key::NumpadMemoryRecall,
        KeyCode::NumpadMemoryStore => Key::NumpadMemoryStore,
        KeyCode::NumpadMemorySubtract => Key::NumpadMemorySubtract,
        KeyCode::NumpadParenLeft => Key::NumpadParenLeft,
        KeyCode::NumpadParenRight => Key::NumpadParenRight,
        KeyCode::NumpadStar => Key::NumpadStar,
        KeyCode::Open => Key::Open,
        KeyCode::Paste => Key::Paste,
        KeyCode::Power => Key::Power,
        KeyCode::Props => Key::Props,
        KeyCode::Resume => Key::Resume,
        KeyCode::Select => Key::Select,
        KeyCode::Sleep => Key::Sleep,
        KeyCode::Suspend => Key::Suspend,
        KeyCode::Turbo => Key::Turbo,
        KeyCode::Undo => Key::Undo,
        KeyCode::WakeUp => Key::WakeUp,
        _ => return None,
    })
}

/// Preserve a physical key even when winit cannot map it to one of the standard 194 codes.
/// Native scan-code identities are intentionally platform-specific and should be stored as local
/// user bindings rather than included in portable default control maps.
pub fn from_winit_physical(key: PhysicalKey) -> Option<Key> {
    match key {
        PhysicalKey::Code(code) => from_winit(code),
        PhysicalKey::Unidentified(code) => Some(Key::Native(match code {
            WinitNativeKeyCode::Unidentified => NativeKeyCode::Unidentified,
            WinitNativeKeyCode::Android(code) => NativeKeyCode::Android(code),
            WinitNativeKeyCode::MacOS(code) => NativeKeyCode::MacOS(code),
            WinitNativeKeyCode::Windows(code) => NativeKeyCode::Windows(code),
            WinitNativeKeyCode::Xkb(code) => NativeKeyCode::Xkb(code),
        })),
    }
}

/// Complete portable physical-key mapping exposed for regression checks and tools.
const KEY_MAPPING: &[(KeyCode, Key)] = &[
    (KeyCode::KeyA, Key::A),
    (KeyCode::KeyB, Key::B),
    (KeyCode::KeyC, Key::C),
    (KeyCode::KeyD, Key::D),
    (KeyCode::KeyE, Key::E),
    (KeyCode::KeyF, Key::F),
    (KeyCode::KeyG, Key::G),
    (KeyCode::KeyH, Key::H),
    (KeyCode::KeyI, Key::I),
    (KeyCode::KeyJ, Key::J),
    (KeyCode::KeyK, Key::K),
    (KeyCode::KeyL, Key::L),
    (KeyCode::KeyM, Key::M),
    (KeyCode::KeyN, Key::N),
    (KeyCode::KeyO, Key::O),
    (KeyCode::KeyP, Key::P),
    (KeyCode::KeyQ, Key::Q),
    (KeyCode::KeyR, Key::R),
    (KeyCode::KeyS, Key::S),
    (KeyCode::KeyT, Key::T),
    (KeyCode::KeyU, Key::U),
    (KeyCode::KeyV, Key::V),
    (KeyCode::KeyW, Key::W),
    (KeyCode::KeyX, Key::X),
    (KeyCode::KeyY, Key::Y),
    (KeyCode::KeyZ, Key::Z),
    (KeyCode::Digit0, Key::Digit0),
    (KeyCode::Digit1, Key::Digit1),
    (KeyCode::Digit2, Key::Digit2),
    (KeyCode::Digit3, Key::Digit3),
    (KeyCode::Digit4, Key::Digit4),
    (KeyCode::Digit5, Key::Digit5),
    (KeyCode::Digit6, Key::Digit6),
    (KeyCode::Digit7, Key::Digit7),
    (KeyCode::Digit8, Key::Digit8),
    (KeyCode::Digit9, Key::Digit9),
    (KeyCode::Numpad0, Key::Numpad0),
    (KeyCode::Numpad1, Key::Numpad1),
    (KeyCode::Numpad2, Key::Numpad2),
    (KeyCode::Numpad3, Key::Numpad3),
    (KeyCode::Numpad4, Key::Numpad4),
    (KeyCode::Numpad5, Key::Numpad5),
    (KeyCode::Numpad6, Key::Numpad6),
    (KeyCode::Numpad7, Key::Numpad7),
    (KeyCode::Numpad8, Key::Numpad8),
    (KeyCode::Numpad9, Key::Numpad9),
    (KeyCode::F1, Key::F1),
    (KeyCode::F2, Key::F2),
    (KeyCode::F3, Key::F3),
    (KeyCode::F4, Key::F4),
    (KeyCode::F5, Key::F5),
    (KeyCode::F6, Key::F6),
    (KeyCode::F7, Key::F7),
    (KeyCode::F8, Key::F8),
    (KeyCode::F9, Key::F9),
    (KeyCode::F10, Key::F10),
    (KeyCode::F11, Key::F11),
    (KeyCode::F12, Key::F12),
    (KeyCode::F13, Key::F13),
    (KeyCode::F14, Key::F14),
    (KeyCode::F15, Key::F15),
    (KeyCode::F16, Key::F16),
    (KeyCode::F17, Key::F17),
    (KeyCode::F18, Key::F18),
    (KeyCode::F19, Key::F19),
    (KeyCode::F20, Key::F20),
    (KeyCode::F21, Key::F21),
    (KeyCode::F22, Key::F22),
    (KeyCode::F23, Key::F23),
    (KeyCode::F24, Key::F24),
    (KeyCode::ArrowUp, Key::Up),
    (KeyCode::ArrowDown, Key::Down),
    (KeyCode::ArrowLeft, Key::Left),
    (KeyCode::ArrowRight, Key::Right),
    (KeyCode::ControlLeft, Key::LeftControl),
    (KeyCode::ControlRight, Key::RightControl),
    (KeyCode::AltLeft, Key::LeftAlt),
    (KeyCode::AltRight, Key::RightAlt),
    (KeyCode::ShiftLeft, Key::LeftShift),
    (KeyCode::ShiftRight, Key::RightShift),
    (KeyCode::SuperLeft, Key::LeftSuper),
    (KeyCode::SuperRight, Key::RightSuper),
    (KeyCode::Space, Key::Space),
    (KeyCode::Escape, Key::Escape),
    (KeyCode::Enter, Key::Enter),
    (KeyCode::Tab, Key::Tab),
    (KeyCode::Backspace, Key::Backspace),
    (KeyCode::Delete, Key::Delete),
    (KeyCode::Insert, Key::Insert),
    (KeyCode::Home, Key::Home),
    (KeyCode::End, Key::End),
    (KeyCode::PageUp, Key::PageUp),
    (KeyCode::PageDown, Key::PageDown),
    (KeyCode::Minus, Key::Minus),
    (KeyCode::Equal, Key::Equal),
    (KeyCode::BracketLeft, Key::BracketLeft),
    (KeyCode::BracketRight, Key::BracketRight),
    (KeyCode::Backslash, Key::Backslash),
    (KeyCode::Semicolon, Key::Semicolon),
    (KeyCode::Quote, Key::Quote),
    (KeyCode::Backquote, Key::Backquote),
    (KeyCode::Comma, Key::Comma),
    (KeyCode::Period, Key::Period),
    (KeyCode::Slash, Key::Slash),
    (KeyCode::CapsLock, Key::CapsLock),
    (KeyCode::NumLock, Key::NumLock),
    (KeyCode::ScrollLock, Key::ScrollLock),
    (KeyCode::PrintScreen, Key::PrintScreen),
    (KeyCode::Pause, Key::Pause),
    (KeyCode::NumpadAdd, Key::NumpadAdd),
    (KeyCode::NumpadSubtract, Key::NumpadSubtract),
    (KeyCode::NumpadMultiply, Key::NumpadMultiply),
    (KeyCode::NumpadDivide, Key::NumpadDivide),
    (KeyCode::NumpadDecimal, Key::NumpadDecimal),
    (KeyCode::NumpadEnter, Key::NumpadEnter),
    (KeyCode::NumpadEqual, Key::NumpadEqual),
    (KeyCode::NumpadComma, Key::NumpadComma),
    (KeyCode::Abort, Key::Abort),
    (KeyCode::Again, Key::Again),
    (KeyCode::AudioVolumeDown, Key::AudioVolumeDown),
    (KeyCode::AudioVolumeMute, Key::AudioVolumeMute),
    (KeyCode::AudioVolumeUp, Key::AudioVolumeUp),
    (KeyCode::BrowserBack, Key::BrowserBack),
    (KeyCode::BrowserFavorites, Key::BrowserFavorites),
    (KeyCode::BrowserForward, Key::BrowserForward),
    (KeyCode::BrowserHome, Key::BrowserHome),
    (KeyCode::BrowserRefresh, Key::BrowserRefresh),
    (KeyCode::BrowserSearch, Key::BrowserSearch),
    (KeyCode::BrowserStop, Key::BrowserStop),
    (KeyCode::ContextMenu, Key::ContextMenu),
    (KeyCode::Convert, Key::Convert),
    (KeyCode::Copy, Key::Copy),
    (KeyCode::Cut, Key::Cut),
    (KeyCode::Eject, Key::Eject),
    (KeyCode::F25, Key::F25),
    (KeyCode::F26, Key::F26),
    (KeyCode::F27, Key::F27),
    (KeyCode::F28, Key::F28),
    (KeyCode::F29, Key::F29),
    (KeyCode::F30, Key::F30),
    (KeyCode::F31, Key::F31),
    (KeyCode::F32, Key::F32),
    (KeyCode::F33, Key::F33),
    (KeyCode::F34, Key::F34),
    (KeyCode::F35, Key::F35),
    (KeyCode::Find, Key::Find),
    (KeyCode::Fn, Key::Fn),
    (KeyCode::FnLock, Key::FnLock),
    (KeyCode::Help, Key::Help),
    (KeyCode::Hiragana, Key::Hiragana),
    (KeyCode::Hyper, Key::Hyper),
    (KeyCode::IntlBackslash, Key::IntlBackslash),
    (KeyCode::IntlRo, Key::IntlRo),
    (KeyCode::IntlYen, Key::IntlYen),
    (KeyCode::KanaMode, Key::KanaMode),
    (KeyCode::Katakana, Key::Katakana),
    (KeyCode::Lang1, Key::Lang1),
    (KeyCode::Lang2, Key::Lang2),
    (KeyCode::Lang3, Key::Lang3),
    (KeyCode::Lang4, Key::Lang4),
    (KeyCode::Lang5, Key::Lang5),
    (KeyCode::LaunchApp1, Key::LaunchApp1),
    (KeyCode::LaunchApp2, Key::LaunchApp2),
    (KeyCode::LaunchMail, Key::LaunchMail),
    (KeyCode::MediaPlayPause, Key::MediaPlayPause),
    (KeyCode::MediaSelect, Key::MediaSelect),
    (KeyCode::MediaStop, Key::MediaStop),
    (KeyCode::MediaTrackNext, Key::MediaTrackNext),
    (KeyCode::MediaTrackPrevious, Key::MediaTrackPrevious),
    (KeyCode::Meta, Key::Meta),
    (KeyCode::NonConvert, Key::NonConvert),
    (KeyCode::NumpadBackspace, Key::NumpadBackspace),
    (KeyCode::NumpadClear, Key::NumpadClear),
    (KeyCode::NumpadClearEntry, Key::NumpadClearEntry),
    (KeyCode::NumpadHash, Key::NumpadHash),
    (KeyCode::NumpadMemoryAdd, Key::NumpadMemoryAdd),
    (KeyCode::NumpadMemoryClear, Key::NumpadMemoryClear),
    (KeyCode::NumpadMemoryRecall, Key::NumpadMemoryRecall),
    (KeyCode::NumpadMemoryStore, Key::NumpadMemoryStore),
    (KeyCode::NumpadMemorySubtract, Key::NumpadMemorySubtract),
    (KeyCode::NumpadParenLeft, Key::NumpadParenLeft),
    (KeyCode::NumpadParenRight, Key::NumpadParenRight),
    (KeyCode::NumpadStar, Key::NumpadStar),
    (KeyCode::Open, Key::Open),
    (KeyCode::Paste, Key::Paste),
    (KeyCode::Power, Key::Power),
    (KeyCode::Props, Key::Props),
    (KeyCode::Resume, Key::Resume),
    (KeyCode::Select, Key::Select),
    (KeyCode::Sleep, Key::Sleep),
    (KeyCode::Suspend, Key::Suspend),
    (KeyCode::Turbo, Key::Turbo),
    (KeyCode::Undo, Key::Undo),
    (KeyCode::WakeUp, Key::WakeUp),
];

pub const fn key_mapping() -> &'static [(KeyCode, Key)] {
    KEY_MAPPING
}

#[derive(Default)]
pub struct MouseState {
    state: MouseInputState,
}

impl MouseState {
    pub fn process_button(&mut self, button: MouseButton, state: ElementState) {
        let engine_button = match button {
            MouseButton::Left => EngineMouseButton::Left,
            MouseButton::Right => EngineMouseButton::Right,
            MouseButton::Middle => EngineMouseButton::Middle,
            MouseButton::Back | MouseButton::Forward => return,
            MouseButton::Other(index) => EngineMouseButton::Other(index),
        };
        if state == ElementState::Pressed {
            self.state.press_button(engine_button);
        } else {
            self.state.release_button(engine_button);
        }
    }

    pub fn process_scroll(&mut self, delta: MouseScrollDelta) {
        let scroll = match delta {
            MouseScrollDelta::LineDelta(_, y) => y,
            MouseScrollDelta::PixelDelta(position) => position.y as f32 / 40.0,
        };
        self.state.apply_motion((0.0, 0.0), None, scroll);
    }

    pub fn snapshot(&mut self, delta: (f32, f32), position: Option<(f32, f32)>) -> MouseInput {
        let snapshot = MouseInput {
            delta,
            position,
            scroll: self.state.scroll(),
            buttons: MouseButtons {
                left: self.state.is_down(EngineMouseButton::Left),
                right: self.state.is_down(EngineMouseButton::Right),
                middle: self.state.is_down(EngineMouseButton::Middle),
            },
        };
        self.state.begin_frame();
        snapshot
    }

    pub fn clear(&mut self) {
        self.state.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::{from_winit, from_winit_physical, key_mapping, KeyboardState, MouseState};
    use crate::input::{Key, KeyState, NativeKeyCode, ALL_KEYS};
    use winit::event::{ElementState, MouseButton, MouseScrollDelta};
    use winit::keyboard::{NativeKeyCode as WinitNativeKeyCode, PhysicalKey};

    #[test]
    fn every_engine_key_has_a_physical_winit_mapping() {
        let mapping = key_mapping();
        assert_eq!(mapping.len(), ALL_KEYS.len());
        assert_eq!(mapping.len(), 194);
        for &(code, expected) in mapping {
            assert_eq!(from_winit(code), Some(expected));
        }
        assert!(mapping.iter().all(|(_, key)| ALL_KEYS.contains(key)));
        assert_eq!(
            mapping
                .iter()
                .map(|(_, key)| *key)
                .collect::<std::collections::HashSet<_>>()
                .len(),
            ALL_KEYS.len()
        );
        assert_eq!(from_winit(winit::keyboard::KeyCode::F24), Some(Key::F24));
        assert_eq!(
            from_winit(winit::keyboard::KeyCode::Digit9),
            Some(Key::Digit9)
        );
        assert_eq!(
            from_winit(winit::keyboard::KeyCode::ControlRight),
            Some(Key::RightControl)
        );
        assert_eq!(
            from_winit(winit::keyboard::KeyCode::Numpad7),
            Some(Key::Numpad7)
        );
        assert_eq!(
            from_winit(winit::keyboard::KeyCode::IntlYen),
            Some(Key::IntlYen)
        );
        assert_eq!(
            from_winit(winit::keyboard::KeyCode::MediaPlayPause),
            Some(Key::MediaPlayPause)
        );
        assert_eq!(from_winit(winit::keyboard::KeyCode::F35), Some(Key::F35));
    }

    #[test]
    fn unidentified_physical_keys_keep_the_native_scan_code_for_local_rebinding() {
        assert_eq!(
            from_winit_physical(PhysicalKey::Unidentified(WinitNativeKeyCode::Windows(0x2a))),
            Some(Key::Native(NativeKeyCode::Windows(0x2a)))
        );
        assert_eq!(
            from_winit_physical(PhysicalKey::Unidentified(WinitNativeKeyCode::Xkb(0x1234))),
            Some(Key::Native(NativeKeyCode::Xkb(0x1234)))
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
