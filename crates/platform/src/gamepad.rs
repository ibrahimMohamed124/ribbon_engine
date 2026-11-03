//! Native gamepad backend and deterministic injection hook.

use crate::Input;
use ribbon_app::{App, LogLevel, Stage, World};
use ribbon_events::{EventCursor, WorldEventsExt};
#[cfg(feature = "gamepad")]
use ribbon_input::{GamepadAxis, GamepadButton, MAX_GAMEPADS};
use ribbon_input::{GamepadEvent, RawGamepadEvent};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;

/// Request rumble on one gamepad.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GamepadRumbleRequest {
    pub pad: u8,
    pub strong: f32,
    pub weak: f32,
    pub duration_ms: u32,
}

#[derive(Clone, Copy)]
#[cfg_attr(not(feature = "gamepad"), allow(dead_code))]
struct RumbleCommand(GamepadRumbleRequest);

/// Owns the native polling thread and its channels.
pub struct GamepadBackend {
    receiver: Mutex<mpsc::Receiver<RawGamepadEvent>>,
    rumble: mpsc::Sender<RumbleCommand>,
    stop: Arc<AtomicBool>,
    join: Mutex<Option<JoinHandle<()>>>,
    errors: Mutex<mpsc::Receiver<String>>,
}

impl GamepadBackend {
    #[cfg(feature = "gamepad")]
    pub(crate) fn start() -> Self {
        let (tx, receiver) = mpsc::channel();
        let (rumble, rumble_rx) = mpsc::channel();
        let (error_tx, errors) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let join = std::thread::Builder::new()
            .name("ribbon-gamepad".into())
            .spawn(move || {
                if let Err(error) = gamepad_thread(tx, rumble_rx, thread_stop) {
                    let _ = error_tx.send(error);
                }
            })
            .ok();
        Self {
            receiver: Mutex::new(receiver),
            rumble,
            stop,
            join: Mutex::new(join),
            errors: Mutex::new(errors),
        }
    }

    #[cfg(not(feature = "gamepad"))]
    pub(crate) fn start() -> Self {
        let (_tx, receiver) = mpsc::channel();
        let (rumble, _rx) = mpsc::channel();
        let (_error_tx, errors) = mpsc::channel();
        Self {
            receiver: Mutex::new(receiver),
            rumble,
            stop: Arc::new(AtomicBool::new(true)),
            join: Mutex::new(None),
            errors: Mutex::new(errors),
        }
    }

    fn shutdown(&self) {
        self.stop.store(true, Ordering::Release);
        if let Ok(mut join) = self.join.lock() {
            if let Some(join) = join.take() {
                let _ = join.join();
            }
        }
    }
}
impl Drop for GamepadBackend {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Apply one deterministic backend event and publish any processed events.
pub fn inject_raw_gamepad_event(world: &mut World, event: RawGamepadEvent) {
    world.resource_scope::<Input, _>(|world, input| {
        let mut events = Vec::new();
        input.gamepads_mut().apply(event, &mut events);
        for event in events {
            world.send_event(event);
        }
    });
}

pub(crate) fn install(app: &mut App) {
    app.add_event::<GamepadEvent>()
        .add_event::<GamepadRumbleRequest>();
    app.world.insert_resource(RumbleCursor::default());
    if app.world.try_resource::<GamepadBackend>().is_none() {
        app.world.insert_resource(GamepadBackend::start());
    }
    app.add_system(Stage::First, poll_gamepads);
    app.add_system(Stage::Last, forward_rumble);
    app.add_system(Stage::Shutdown, shutdown_gamepad);
}

fn poll_gamepads(world: &mut World) {
    let (events, errors) = {
        let backend = world.resource::<GamepadBackend>();
        let mut raw = Vec::new();
        if let Ok(receiver) = backend.receiver.lock() {
            while let Ok(event) = receiver.try_recv() {
                raw.push(event);
            }
        }
        let mut failures = Vec::new();
        if let Ok(receiver) = backend.errors.lock() {
            while let Ok(error) = receiver.try_recv() {
                failures.push(error);
            }
        }
        (raw, failures)
    };
    for error in errors {
        world.resource_mut::<ribbon_app::LogBuffer>().push(
            LogLevel::Warn,
            format!("gamepad backend unavailable: {error}"),
        );
    }
    world.resource_scope::<Input, _>(|world, input| {
        input.gamepads_mut().begin_frame();
        let mut out = Vec::new();
        for event in events {
            input.gamepads_mut().apply(event, &mut out);
        }
        for event in out {
            world.send_event(event);
        }
    });
}
fn forward_rumble(world: &mut World) {
    let requests = world.resource_scope::<RumbleCursor, _>(|world, cursor| {
        cursor
            .0
            .read(world.events::<GamepadRumbleRequest>())
            .copied()
            .collect::<Vec<_>>()
    });
    if let Some(backend) = world.try_resource::<GamepadBackend>() {
        for request in requests {
            let _ = backend.rumble.send(RumbleCommand(GamepadRumbleRequest {
                strong: finite(request.strong).clamp(0.0, 1.0),
                weak: finite(request.weak).clamp(0.0, 1.0),
                duration_ms: request.duration_ms.clamp(1, 5000),
                ..request
            }));
        }
    }
}
#[derive(Default)]
struct RumbleCursor(EventCursor<GamepadRumbleRequest>);
fn shutdown_gamepad(world: &mut World) {
    if let Some(backend) = world.try_resource::<GamepadBackend>() {
        backend.shutdown();
    }
}
fn finite(v: f32) -> f32 {
    if v.is_finite() {
        v
    } else {
        0.0
    }
}

#[cfg(feature = "gamepad")]
fn gamepad_thread(
    tx: mpsc::Sender<RawGamepadEvent>,
    rumble_rx: mpsc::Receiver<RumbleCommand>,
    stop: Arc<AtomicBool>,
) -> Result<(), String> {
    use gilrs::{Button, EventType, Gilrs};
    use std::collections::HashMap;
    use std::time::Duration;
    let mut gilrs = Gilrs::new().map_err(|e| e.to_string())?;
    let mut slots: HashMap<gilrs::GamepadId, u8> = HashMap::new();
    let mut used = [false; MAX_GAMEPADS];
    for (id, _) in gilrs.gamepads() {
        if let Some(slot) = alloc_slot(id, &mut slots, &mut used) {
            let _ = tx.send(RawGamepadEvent::Connected { slot });
        }
    }
    while !stop.load(Ordering::Acquire) {
        if let Some(event) = gilrs.next_event_blocking(Some(Duration::from_millis(4))) {
            let id = event.id;
            match event.event {
                EventType::Connected => {
                    if let Some(slot) = alloc_slot(id, &mut slots, &mut used) {
                        let _ = tx.send(RawGamepadEvent::Connected { slot });
                    }
                }
                EventType::Disconnected => {
                    if let Some(slot) = slots.get(&id).copied() {
                        used[slot as usize] = false;
                        let _ = tx.send(RawGamepadEvent::Disconnected { slot });
                    }
                }
                EventType::ButtonPressed(b, _) | EventType::ButtonRepeated(b, _) => {
                    if let (Some(slot), Some(button)) = (slots.get(&id).copied(), map_button(b)) {
                        let _ = tx.send(RawGamepadEvent::Button {
                            slot,
                            button,
                            pressed: true,
                        });
                    }
                }
                EventType::ButtonReleased(b, _) => {
                    if let (Some(slot), Some(button)) = (slots.get(&id).copied(), map_button(b)) {
                        let _ = tx.send(RawGamepadEvent::Button {
                            slot,
                            button,
                            pressed: false,
                        });
                    }
                }
                EventType::ButtonChanged(Button::LeftTrigger2, v, _) => {
                    if let Some(slot) = slots.get(&id).copied() {
                        let _ = tx.send(RawGamepadEvent::Axis {
                            slot,
                            axis: GamepadAxis::LeftTrigger,
                            value: v,
                        });
                    }
                }
                EventType::ButtonChanged(Button::RightTrigger2, v, _) => {
                    if let Some(slot) = slots.get(&id).copied() {
                        let _ = tx.send(RawGamepadEvent::Axis {
                            slot,
                            axis: GamepadAxis::RightTrigger,
                            value: v,
                        });
                    }
                }
                EventType::ButtonChanged(_, _, _) => {}
                EventType::AxisChanged(axis, v, _) => {
                    if let (Some(slot), Some(axis)) = (slots.get(&id).copied(), map_axis(axis)) {
                        let _ = tx.send(RawGamepadEvent::Axis {
                            slot,
                            axis,
                            value: v,
                        });
                    }
                }
                _ => {}
            }
        }
        while let Ok(command) = rumble_rx.try_recv() {
            apply_rumble(&mut gilrs, &slots, command.0);
        }
    }
    Ok(())
}
#[cfg(feature = "gamepad")]
fn alloc_slot(
    id: gilrs::GamepadId,
    slots: &mut std::collections::HashMap<gilrs::GamepadId, u8>,
    used: &mut [bool; MAX_GAMEPADS],
) -> Option<u8> {
    if let Some(slot) = slots.get(&id).copied() {
        if !used[slot as usize] {
            used[slot as usize] = true;
            return Some(slot);
        }
    }
    let slot = used.iter().position(|x| !*x)? as u8;
    used[slot as usize] = true;
    slots.insert(id, slot);
    Some(slot)
}
#[cfg(feature = "gamepad")]
fn map_button(b: gilrs::Button) -> Option<GamepadButton> {
    use gilrs::Button as B;
    Some(match b {
        B::South => GamepadButton::South,
        B::East => GamepadButton::East,
        B::West => GamepadButton::West,
        B::North => GamepadButton::North,
        B::LeftTrigger => GamepadButton::LeftBumper,
        B::RightTrigger => GamepadButton::RightBumper,
        B::LeftTrigger2 | B::RightTrigger2 => return None,
        B::Select => GamepadButton::Select,
        B::Start => GamepadButton::Start,
        B::Mode => GamepadButton::Mode,
        B::LeftThumb => GamepadButton::LeftStick,
        B::RightThumb => GamepadButton::RightStick,
        B::DPadUp => GamepadButton::DPadUp,
        B::DPadDown => GamepadButton::DPadDown,
        B::DPadLeft => GamepadButton::DPadLeft,
        B::DPadRight => GamepadButton::DPadRight,
        _ => return None,
    })
}
#[cfg(feature = "gamepad")]
fn map_axis(a: gilrs::Axis) -> Option<GamepadAxis> {
    use gilrs::Axis as A;
    Some(match a {
        A::LeftStickX => GamepadAxis::LeftStickX,
        A::LeftStickY => GamepadAxis::LeftStickY,
        A::RightStickX => GamepadAxis::RightStickX,
        A::RightStickY => GamepadAxis::RightStickY,
        A::LeftZ => GamepadAxis::LeftTrigger,
        A::RightZ => GamepadAxis::RightTrigger,
        _ => return None,
    })
}
#[cfg(feature = "gamepad")]
fn apply_rumble(
    gilrs: &mut gilrs::Gilrs,
    slots: &std::collections::HashMap<gilrs::GamepadId, u8>,
    request: GamepadRumbleRequest,
) {
    use gilrs::ff::{BaseEffect, BaseEffectType, EffectBuilder, Replay, Ticks};
    let Some((id, _)) = slots.iter().find(|(_, slot)| **slot == request.pad) else {
        return;
    };
    let play_for = Ticks::from_ms(request.duration_ms);
    let mut builder = EffectBuilder::new();
    if request.strong > 0.0 {
        builder.add_effect(BaseEffect {
            kind: BaseEffectType::Strong {
                magnitude: (request.strong * 65535.0) as u16,
            },
            scheduling: Replay {
                play_for,
                ..Default::default()
            },
            ..Default::default()
        });
    }
    if request.weak > 0.0 {
        builder.add_effect(BaseEffect {
            kind: BaseEffectType::Weak {
                magnitude: (request.weak * 65535.0) as u16,
            },
            scheduling: Replay {
                play_for,
                ..Default::default()
            },
            ..Default::default()
        });
    }
    if let Ok(effect) = builder.gamepads(&[*id]).finish(gilrs) {
        let _ = effect.play();
    }
}
