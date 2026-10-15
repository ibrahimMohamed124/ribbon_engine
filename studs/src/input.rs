//! Keyboard / mouse input, turned into game-level intent.
//!
//! The rest of the game never sees a windowing-library key: it only sees [`Controls`]
//! (continuous intent) and [`InputFrame`] (intent + one-shot events).
//! Physical keys live in [`KeyBindings`], which are user-configurable.

use engine_core::input::Bindings;
pub use engine_core::input::{key_name, parse_key, Key, KeyState, MouseSource};

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
    ToggleMouseLook,
}

impl Action {
    pub const ALL: [Action; 14] = [
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
        Action::ToggleMouseLook,
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
            Action::ToggleMouseLook => "toggle_mouse_look",
        }
    }

    pub fn from_name(name: &str) -> Option<Action> {
        Action::ALL.iter().copied().find(|a| a.name() == name)
    }
}

// ------------------------------------------------------------------ bindings

#[derive(Clone)]
pub struct KeyBindings(Bindings<Action>);

impl Default for KeyBindings {
    fn default() -> Self {
        let mut bindings = Bindings::new();
        bindings.set(Action::LookUp, vec![Key::Up]);
        bindings.set(Action::LookDown, vec![Key::Down]);
        bindings.set(Action::LookLeft, vec![Key::Left]);
        bindings.set(Action::LookRight, vec![Key::Right]);
        bindings.set(Action::Forward, vec![Key::W]);
        bindings.set(Action::Back, vec![Key::S]);
        bindings.set(Action::Left, vec![Key::A]);
        bindings.set(Action::Right, vec![Key::D]);
        bindings.set(Action::Jump, vec![Key::E]);
        bindings.set(Action::Sprint, vec![Key::LeftShift, Key::RightShift]);
        bindings.set(Action::Flashlight, vec![Key::F]);
        bindings.set(Action::Restart, vec![Key::R]);
        bindings.set(Action::Quit, vec![Key::Q]);
        bindings.set(Action::ToggleMouseLook, vec![Key::Tab, Key::M]);
        KeyBindings(bindings)
    }
}

impl KeyBindings {
    pub fn keys(&self, action: Action) -> &[Key] {
        self.0.keys(action)
    }

    pub fn set(&mut self, action: Action, keys: Vec<Key>) {
        self.0.set(action, keys);
    }

    pub fn is_down<K: KeyState + ?Sized>(&self, state: &K, action: Action) -> bool {
        self.0.is_down(state, action)
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
    pub toggle_mouse_look: bool,
}

impl InputFrame {
    /// Combine frames while simulation is waiting for its next fixed step.
    /// Held controls use the latest frame; mouse deltas and one-shot actions accumulate.
    pub fn merge(&mut self, latest: Self) {
        let mouse_yaw = self.controls.mouse_yaw + latest.controls.mouse_yaw;
        let mouse_pitch = self.controls.mouse_pitch + latest.controls.mouse_pitch;
        self.controls = latest.controls;
        self.controls.mouse_yaw = mouse_yaw;
        self.controls.mouse_pitch = mouse_pitch;
        self.toggle_flashlight |= latest.toggle_flashlight;
        self.restart |= latest.restart;
        self.quit |= latest.quit;
        self.toggle_mouse_look |= latest.toggle_mouse_look;
    }

    /// Keep held controls for later fixed substeps, consuming all one-shots and deltas.
    pub fn level_only(self) -> Self {
        Self {
            controls: Controls {
                look_up: self.controls.look_up,
                look_down: self.controls.look_down,
                look_left: self.controls.look_left,
                look_right: self.controls.look_right,
                forward: self.controls.forward,
                back: self.controls.back,
                left: self.controls.left,
                right: self.controls.right,
                jump: self.controls.jump,
                sprint: self.controls.sprint,
                ..Controls::default()
            },
            ..Self::default()
        }
    }
}

/// Polls devices once per frame and keeps the little bit of state needed for
/// edge detection (flashlight toggle) and mouse deltas.
pub struct Input {
    bindings: KeyBindings,
    mouse_sensitivity: f32,
    prev_flashlight: bool,
    last_mouse: Option<(f32, f32)>,
    mouse_look_enabled: bool,
    invert_y: bool,
    prev_mouse_look: bool,
    ignore_mouse_delta: bool,
}

impl Input {
    /// `mouse_sensitivity` is radians per pixel; 0 disables mouse look.
    pub fn new(bindings: KeyBindings, mouse_sensitivity: f32) -> Self {
        Input {
            bindings,
            mouse_sensitivity,
            prev_flashlight: false,
            last_mouse: None,
            mouse_look_enabled: false,
            invert_y: false,
            prev_mouse_look: false,
            ignore_mouse_delta: false,
        }
    }

    pub fn with_mouse_look(
        bindings: KeyBindings,
        mouse_sensitivity: f32,
        enabled: bool,
        invert_y: bool,
    ) -> Self {
        let mut input = Self::new(bindings, mouse_sensitivity);
        input.mouse_look_enabled = enabled;
        input.ignore_mouse_delta = enabled;
        input.invert_y = invert_y;
        input
    }

    pub fn mouse_look_enabled(&self) -> bool {
        self.mouse_look_enabled
    }

    /// Whether mouse look is enabled and has a usable sensitivity.
    pub fn mouse_look_active(&self) -> bool {
        self.mouse_look_enabled && self.mouse_sensitivity > 0.0
    }

    pub fn set_bindings(&mut self, bindings: KeyBindings) {
        self.bindings = bindings;
    }

    pub fn set_mouse_sensitivity(&mut self, sensitivity: f32) {
        self.mouse_sensitivity = sensitivity.clamp(0.0, 0.05);
    }

    pub fn set_invert_y(&mut self, invert_y: bool) {
        self.invert_y = invert_y;
    }

    pub fn synchronize_keys<K: KeyState + ?Sized>(&mut self, keys: &K) {
        self.prev_flashlight = self.bindings.is_down(keys, Action::Flashlight);
        self.prev_mouse_look = self.bindings.is_down(keys, Action::ToggleMouseLook);
    }

    /// Poll keyboard controls and a cursor source that reports relative deltas.
    pub fn poll_with_mouse_source<K: KeyState + ?Sized, M: MouseSource + ?Sized>(
        &mut self,
        keys: &K,
        mouse: &mut M,
        focused_and_playing: bool,
    ) -> InputFrame {
        let mut frame = self.poll_keyboard(keys);
        let toggle_now = self.bindings.is_down(keys, Action::ToggleMouseLook);
        frame.toggle_mouse_look = toggle_now && !self.prev_mouse_look;
        self.prev_mouse_look = toggle_now;
        if frame.toggle_mouse_look {
            self.mouse_look_enabled = !self.mouse_look_enabled;
            self.ignore_mouse_delta = self.mouse_look_enabled;
        }

        if !focused_and_playing || !self.mouse_look_enabled {
            self.ignore_mouse_delta = true;
            return frame;
        }

        let (dx, dy) = mouse.delta();
        if self.ignore_mouse_delta {
            self.ignore_mouse_delta = false;
            return frame;
        }
        const MAX_DELTA_PIXELS: f32 = 100.0;
        let dx = dx.clamp(-MAX_DELTA_PIXELS, MAX_DELTA_PIXELS);
        let dy = dy.clamp(-MAX_DELTA_PIXELS, MAX_DELTA_PIXELS);
        frame.controls.mouse_yaw = dx * self.mouse_sensitivity;
        let pitch_sign = if self.invert_y { 1.0 } else { -1.0 };
        frame.controls.mouse_pitch = dy * self.mouse_sensitivity * pitch_sign;
        frame
    }

    fn poll_keyboard<K: KeyState + ?Sized>(&mut self, keys: &K) -> InputFrame {
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
            ..Controls::default()
        };
        let flashlight_now = b.is_down(keys, Action::Flashlight);
        let restart = b.is_down(keys, Action::Restart);
        let quit = b.is_down(keys, Action::Quit);
        let toggle_flashlight = flashlight_now && !self.prev_flashlight;
        self.prev_flashlight = flashlight_now;
        InputFrame {
            controls,
            toggle_flashlight,
            restart,
            quit,
            toggle_mouse_look: false,
        }
    }

    pub fn bindings(&self) -> &KeyBindings {
        &self.bindings
    }

    pub fn poll<K: KeyState + ?Sized>(
        &mut self,
        keys: &K,
        mouse: Option<(f32, f32)>,
    ) -> InputFrame {
        let mut frame = self.poll_keyboard(keys);
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

        frame.controls.mouse_yaw = mouse_yaw;
        frame.controls.mouse_pitch = mouse_pitch;
        frame
    }
}
