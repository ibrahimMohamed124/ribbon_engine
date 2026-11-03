//! Pure gamepad state processing shared by native and virtual input sources.

/// Maximum number of simultaneously tracked pads.
pub const MAX_GAMEPADS: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum GamepadButton {
    South = 0,
    East = 1,
    West = 2,
    North = 3,
    LeftBumper = 4,
    RightBumper = 5,
    LeftTrigger = 6,
    RightTrigger = 7,
    Select = 8,
    Start = 9,
    Mode = 10,
    LeftStick = 11,
    RightStick = 12,
    DPadUp = 13,
    DPadDown = 14,
    DPadLeft = 15,
    DPadRight = 16,
}
impl GamepadButton {
    pub const fn id(self) -> u16 {
        self as u16
    }
    pub fn from_id(id: u16) -> Option<Self> {
        Some(match id {
            0 => Self::South,
            1 => Self::East,
            2 => Self::West,
            3 => Self::North,
            4 => Self::LeftBumper,
            5 => Self::RightBumper,
            6 => Self::LeftTrigger,
            7 => Self::RightTrigger,
            8 => Self::Select,
            9 => Self::Start,
            10 => Self::Mode,
            11 => Self::LeftStick,
            12 => Self::RightStick,
            13 => Self::DPadUp,
            14 => Self::DPadDown,
            15 => Self::DPadLeft,
            16 => Self::DPadRight,
            _ => return None,
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum GamepadAxis {
    LeftStickX = 0,
    LeftStickY = 1,
    RightStickX = 2,
    RightStickY = 3,
    LeftTrigger = 4,
    RightTrigger = 5,
}
impl GamepadAxis {
    pub const fn id(self) -> u8 {
        self as u8
    }
    pub fn from_id(id: u8) -> Option<Self> {
        Some(match id {
            0 => Self::LeftStickX,
            1 => Self::LeftStickY,
            2 => Self::RightStickX,
            3 => Self::RightStickY,
            4 => Self::LeftTrigger,
            5 => Self::RightTrigger,
            _ => return None,
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GamepadSettings {
    pub stick_deadzone: f32,
    pub trigger_deadzone: f32,
    pub trigger_press_threshold: f32,
    pub axis_binding_threshold: f32,
}
impl Default for GamepadSettings {
    fn default() -> Self {
        Self {
            stick_deadzone: 0.15,
            trigger_deadzone: 0.05,
            trigger_press_threshold: 0.5,
            axis_binding_threshold: 0.5,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RawGamepadEvent {
    Connected {
        slot: u8,
    },
    Disconnected {
        slot: u8,
    },
    Button {
        slot: u8,
        button: GamepadButton,
        pressed: bool,
    },
    Axis {
        slot: u8,
        axis: GamepadAxis,
        value: f32,
    },
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GamepadEvent {
    Connected {
        pad: u8,
    },
    Disconnected {
        pad: u8,
    },
    ButtonPressed {
        pad: u8,
        button: GamepadButton,
    },
    ButtonReleased {
        pad: u8,
        button: GamepadButton,
    },
    AxisChanged {
        pad: u8,
        axis: GamepadAxis,
        value: f32,
    },
}

#[derive(Clone)]
struct PadState {
    down: [bool; 17],
    pressed: [bool; 17],
    released: [bool; 17],
    raw: [f32; 6],
}
impl Default for PadState {
    fn default() -> Self {
        Self {
            down: [false; 17],
            pressed: [false; 17],
            released: [false; 17],
            raw: [0.0; 6],
        }
    }
}
/// Processed state for up to four gamepads.
pub struct GamepadSet {
    pads: [Option<PadState>; MAX_GAMEPADS],
    settings: GamepadSettings,
}
impl Default for GamepadSet {
    fn default() -> Self {
        Self::new(GamepadSettings::default())
    }
}
impl GamepadSet {
    pub fn new(settings: GamepadSettings) -> Self {
        Self {
            pads: std::array::from_fn(|_| None),
            settings,
        }
    }
    pub fn settings(&self) -> GamepadSettings {
        self.settings
    }
    pub fn set_settings(&mut self, s: GamepadSettings) {
        self.settings = s;
    }
    pub fn begin_frame(&mut self) {
        for p in self.pads.iter_mut().flatten() {
            p.pressed.fill(false);
            p.released.fill(false);
        }
    }
    pub fn apply(&mut self, e: RawGamepadEvent, out: &mut Vec<GamepadEvent>) {
        match e {
            RawGamepadEvent::Connected { slot } if (slot as usize) < MAX_GAMEPADS => {
                let i = slot as usize;
                if self.pads[i].is_none() {
                    self.pads[i] = Some(PadState::default());
                    out.push(GamepadEvent::Connected { pad: slot });
                }
            }
            RawGamepadEvent::Disconnected { slot } if (slot as usize) < MAX_GAMEPADS => {
                let i = slot as usize;
                if self.pads[i].take().is_some() {
                    out.push(GamepadEvent::Disconnected { pad: slot });
                }
            }
            RawGamepadEvent::Button {
                slot,
                button,
                pressed,
            } if (slot as usize) < MAX_GAMEPADS => {
                if let Some(p) = self.pads[slot as usize].as_mut() {
                    if matches!(
                        button,
                        GamepadButton::LeftTrigger | GamepadButton::RightTrigger
                    ) {
                        return;
                    }
                    set_button(p, slot, button, pressed, out);
                }
            }
            RawGamepadEvent::Axis { slot, axis, value } if (slot as usize) < MAX_GAMEPADS => {
                let settings = self.settings;
                if let Some(p) = self.pads[slot as usize].as_mut() {
                    let axes = [
                        GamepadAxis::LeftStickX,
                        GamepadAxis::LeftStickY,
                        GamepadAxis::RightStickX,
                        GamepadAxis::RightStickY,
                        GamepadAxis::LeftTrigger,
                        GamepadAxis::RightTrigger,
                    ];
                    let previous = axes.map(|candidate| process_axis(p, candidate, settings));
                    p.raw[axis.id() as usize] =
                        finite(value).clamp(if axis as u8 >= 4 { 0.0 } else { -1.0 }, 1.0);
                    let mut trigger_values = [0.0; 2];
                    for (index, candidate) in axes.into_iter().enumerate() {
                        let processed = process_axis(p, candidate, settings);
                        if (processed - previous[index]).abs() > 1.0e-6 {
                            out.push(GamepadEvent::AxisChanged {
                                pad: slot,
                                axis: candidate,
                                value: processed,
                            });
                        }
                        if candidate == GamepadAxis::LeftTrigger {
                            trigger_values[0] = processed;
                        } else if candidate == GamepadAxis::RightTrigger {
                            trigger_values[1] = processed;
                        }
                    }
                    for (button, processed) in [
                        (GamepadButton::LeftTrigger, trigger_values[0]),
                        (GamepadButton::RightTrigger, trigger_values[1]),
                    ] {
                        let was = p.down[button.id() as usize];
                        let base_threshold =
                            finite(settings.trigger_press_threshold).clamp(0.0, 1.0);
                        let threshold = (base_threshold - f32::from(was) * 0.05).max(0.0);
                        set_button(p, slot, button, processed >= threshold, out);
                    }
                }
            }
            _ => {}
        }
    }
    fn axis_from(&self, p: &PadState, a: GamepadAxis) -> f32 {
        process_axis(p, a, self.settings)
    }
    fn stick_from(&self, p: &PadState, left: bool) -> (f32, f32) {
        process_stick(p, left, self.settings)
    }
    pub fn is_connected(&self, pad: u8) -> bool {
        self.pads.get(pad as usize).is_some_and(Option::is_some)
    }
    pub fn connected_pads(&self) -> impl Iterator<Item = u8> + '_ {
        self.pads
            .iter()
            .enumerate()
            .filter_map(|(i, p)| p.as_ref().map(|_| i as u8))
    }
    pub fn button_down(&self, pad: u8, b: GamepadButton) -> bool {
        self.pads
            .get(pad as usize)
            .and_then(Option::as_ref)
            .is_some_and(|p| p.down[b.id() as usize])
    }
    pub fn just_pressed(&self, pad: u8, b: GamepadButton) -> bool {
        self.pads
            .get(pad as usize)
            .and_then(Option::as_ref)
            .is_some_and(|p| p.pressed[b.id() as usize])
    }
    pub fn just_released(&self, pad: u8, b: GamepadButton) -> bool {
        self.pads
            .get(pad as usize)
            .and_then(Option::as_ref)
            .is_some_and(|p| p.released[b.id() as usize])
    }
    pub fn axis(&self, pad: u8, a: GamepadAxis) -> f32 {
        self.pads
            .get(pad as usize)
            .and_then(Option::as_ref)
            .map_or(0.0, |p| self.axis_from(p, a))
    }
    pub fn stick(&self, pad: u8, left: bool) -> (f32, f32) {
        self.pads
            .get(pad as usize)
            .and_then(Option::as_ref)
            .map_or((0.0, 0.0), |p| self.stick_from(p, left))
    }
}
fn process_axis(p: &PadState, a: GamepadAxis, settings: GamepadSettings) -> f32 {
    match a {
        GamepadAxis::LeftStickX => process_stick(p, true, settings).0,
        GamepadAxis::LeftStickY => process_stick(p, true, settings).1,
        GamepadAxis::RightStickX => process_stick(p, false, settings).0,
        GamepadAxis::RightStickY => process_stick(p, false, settings).1,
        GamepadAxis::LeftTrigger | GamepadAxis::RightTrigger => {
            let v = p.raw[a.id() as usize];
            let d = finite(settings.trigger_deadzone).clamp(0.0, 0.999);
            if v < d {
                0.0
            } else {
                (v - d) / (1.0 - d)
            }
        }
    }
}
fn process_stick(p: &PadState, left: bool, settings: GamepadSettings) -> (f32, f32) {
    let k = if left { 0 } else { 2 };
    let x = p.raw[k];
    let y = p.raw[k + 1];
    let m = x.hypot(y);
    let dz = finite(settings.stick_deadzone).clamp(0.0, 0.999);
    if m == 0.0 || m < dz {
        return (0.0, 0.0);
    }
    let scale = ((m - dz) / (1.0 - dz)).min(1.0) / m;
    (x * scale, y * scale)
}
fn finite(v: f32) -> f32 {
    if v.is_finite() {
        v
    } else {
        0.0
    }
}
fn set_button(
    p: &mut PadState,
    pad: u8,
    b: GamepadButton,
    value: bool,
    out: &mut Vec<GamepadEvent>,
) {
    let i = b.id() as usize;
    if p.down[i] != value {
        p.down[i] = value;
        if value {
            p.pressed[i] = true;
            out.push(GamepadEvent::ButtonPressed { pad, button: b })
        } else {
            p.released[i] = true;
            out.push(GamepadEvent::ButtonReleased { pad, button: b })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ActionState, BindingSource, Bindings, Key, KeyState};
    #[test]
    fn radial_stick_deadzone_and_rescale() {
        let mut s = GamepadSet::new(GamepadSettings::default());
        let mut o = vec![];
        s.apply(RawGamepadEvent::Connected { slot: 0 }, &mut o);
        s.apply(
            RawGamepadEvent::Axis {
                slot: 0,
                axis: GamepadAxis::LeftStickX,
                value: 0.075,
            },
            &mut o,
        );
        assert_eq!(s.axis(0, GamepadAxis::LeftStickX), 0.0);
        s.apply(
            RawGamepadEvent::Axis {
                slot: 0,
                axis: GamepadAxis::LeftStickX,
                value: 0.575,
            },
            &mut o,
        );
        assert!((s.axis(0, GamepadAxis::LeftStickX) - 0.5).abs() < 0.0001);
    }
    #[test]
    fn trigger_rescale_hysteresis_and_edges() {
        let mut s = GamepadSet::new(GamepadSettings::default());
        let mut o = vec![];
        s.apply(RawGamepadEvent::Connected { slot: 0 }, &mut o);
        s.begin_frame();
        s.apply(
            RawGamepadEvent::Axis {
                slot: 0,
                axis: GamepadAxis::LeftTrigger,
                value: 0.55,
            },
            &mut o,
        );
        assert!(s.button_down(0, GamepadButton::LeftTrigger));
        assert!(s.just_pressed(0, GamepadButton::LeftTrigger));
        assert!((s.axis(0, GamepadAxis::LeftTrigger) - 0.5263).abs() < 0.01);
        s.begin_frame();
        s.apply(
            RawGamepadEvent::Axis {
                slot: 0,
                axis: GamepadAxis::LeftTrigger,
                value: 0.52,
            },
            &mut o,
        );
        assert!(s.button_down(0, GamepadButton::LeftTrigger));
        s.apply(
            RawGamepadEvent::Axis {
                slot: 0,
                axis: GamepadAxis::LeftTrigger,
                value: 0.47,
            },
            &mut o,
        );
        assert!(!s.button_down(0, GamepadButton::LeftTrigger));
        assert!(s.just_released(0, GamepadButton::LeftTrigger));
    }
    #[test]
    fn disconnect_clears_and_nonfinite_is_zero() {
        let mut s = GamepadSet::new(GamepadSettings::default());
        let mut o = vec![];
        s.apply(RawGamepadEvent::Connected { slot: 0 }, &mut o);
        s.apply(
            RawGamepadEvent::Button {
                slot: 0,
                button: GamepadButton::South,
                pressed: true,
            },
            &mut o,
        );
        s.apply(
            RawGamepadEvent::Axis {
                slot: 0,
                axis: GamepadAxis::LeftStickX,
                value: f32::NAN,
            },
            &mut o,
        );
        assert!(s.button_down(0, GamepadButton::South));
        assert_eq!(s.axis(0, GamepadAxis::LeftStickX), 0.0);
        s.apply(RawGamepadEvent::Disconnected { slot: 0 }, &mut o);
        assert!(!s.is_connected(0));
        assert!(!s.button_down(0, GamepadButton::South));
    }

    #[test]
    fn invalid_trigger_threshold_is_sanitized() {
        let settings = GamepadSettings {
            trigger_press_threshold: f32::NAN,
            ..GamepadSettings::default()
        };
        let mut set = GamepadSet::new(settings);
        let mut events = Vec::new();
        set.apply(RawGamepadEvent::Connected { slot: 0 }, &mut events);
        set.apply(
            RawGamepadEvent::Axis {
                slot: 0,
                axis: GamepadAxis::RightTrigger,
                value: 0.6,
            },
            &mut events,
        );
        assert!(set.button_down(0, GamepadButton::RightTrigger));
    }

    struct State(f32);
    impl KeyState for State {
        fn is_down(&self, _: Key) -> bool {
            false
        }
    }
    impl ActionState for State {
        fn gamepad_axis_value(&self, pad: u8, axis: u8) -> f32 {
            if pad == 0 && axis == GamepadAxis::LeftStickX.id() {
                self.0
            } else {
                0.0
            }
        }
    }

    #[test]
    fn unchanged_processed_axes_do_not_emit_duplicate_axis_events() {
        let mut pads = GamepadSet::default();
        let mut events = Vec::new();
        pads.apply(RawGamepadEvent::Connected { slot: 0 }, &mut events);
        events.clear();
        let axis_event = RawGamepadEvent::Axis {
            slot: 0,
            axis: GamepadAxis::LeftStickX,
            value: 0.8,
        };
        pads.apply(axis_event, &mut events);
        assert!(events.iter().any(|event| matches!(
            event,
            GamepadEvent::AxisChanged {
                axis: GamepadAxis::LeftStickX,
                ..
            }
        )));
        events.clear();
        pads.apply(axis_event, &mut events);
        assert!(events.is_empty());
    }

    #[test]
    fn axis_binding_resolves_threshold_and_signed_value() {
        let mut bindings = Bindings::new();
        bindings.set_sources(
            "move",
            vec![
                BindingSource::gamepad_axis(0, GamepadAxis::LeftStickX, true, 0.5),
                BindingSource::gamepad_axis(0, GamepadAxis::LeftStickX, false, 0.5),
            ],
        );
        assert!(bindings.is_active(&State(0.6), "move"));
        assert_eq!(bindings.axis_value(&State(-0.75), "move"), -0.75);
        assert!(!bindings.is_active(&State(0.49), "move"));
    }
}
