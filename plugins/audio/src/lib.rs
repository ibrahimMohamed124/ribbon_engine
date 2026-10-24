//! Sound-bank data and event-driven audio playback integration.
//!
//! Device output is supplied through `AudioSink`; games own IDs and embedded sound assets.

use ribbon_app::{App, LogBuffer, LogLevel, Stage};
use ribbon_ecs::World;
use ribbon_events::{EventCursor, WorldEventsExt};
use ribbon_math::Vect;
use ribbon_plugins::Plugin;

pub mod bank;
pub mod mixer;
#[cfg(feature = "audio")]
pub mod rodio_backend;
pub use bank::{AmbienceAsset, AmbienceId, SoundAsset, SoundBank, SoundId};

/// Request a one-shot sound, optionally at a world position.
#[derive(Clone, Copy, Debug)]
pub struct PlaySound {
    pub id: SoundId,
    pub position: Option<Vect>,
    pub gain: f32,
}
/// Change or stop the current looping ambience.
#[derive(Clone, Copy, Debug)]
pub struct SetAmbience {
    pub id: Option<AmbienceId>,
    pub gain: f32,
}
/// Listener pose in world space.
#[derive(Clone, Copy, Debug, Default)]
pub struct AudioListener {
    pub position: Vect,
    pub yaw: f32,
}

/// Platform-facing output. A missing output keeps the engine in silent mode.
pub trait AudioSink: Send + Sync + 'static {
    fn play(&mut self, id: SoundId, gain: f32, pan: f32);
    fn set_ambience(&mut self, id: Option<AmbienceId>, gain: f32);
}
#[derive(Default)]
pub struct AudioOutput(pub Option<Box<dyn AudioSink>>);
#[derive(Default)]
struct AudioCursors {
    sounds: EventCursor<PlaySound>,
    ambience: EventCursor<SetAmbience>,
    warned_silent: bool,
}

/// Consume playback events and apply shared listener-relative pan and falloff.
pub fn audio_event_system(world: &mut World) {
    let sounds = world.resource_scope::<AudioCursors, _>(|world, cursor| {
        cursor
            .sounds
            .read(world.events::<PlaySound>())
            .copied()
            .collect::<Vec<_>>()
    });
    let ambiences = world.resource_scope::<AudioCursors, _>(|world, cursor| {
        cursor
            .ambience
            .read(world.events::<SetAmbience>())
            .copied()
            .collect::<Vec<_>>()
    });
    if sounds.is_empty() && ambiences.is_empty() {
        return;
    }
    let listener = world
        .query::<&AudioListener>()
        .iter()
        .next()
        .map(|(_, l)| *l);
    let has_sink = world.resource::<AudioOutput>().0.is_some();
    if !has_sink {
        let should_warn = world.resource_scope::<AudioCursors, _>(|_, cursor| {
            let first = !cursor.warned_silent;
            cursor.warned_silent = true;
            first
        });
        if should_warn {
            world
                .resource_mut::<LogBuffer>()
                .push(LogLevel::Warn, "audio output unavailable; running silently");
        }
        return;
    }
    world.resource_scope::<AudioOutput, _>(|_, output| {
        let Some(sink) = output.0.as_mut() else {
            return;
        };
        for event in sounds {
            let (gain, pan) =
                event
                    .position
                    .zip(listener)
                    .map_or((event.gain, 0.0), |(source, l)| {
                        let dx = source.x - l.position.x;
                        let dy = source.y - l.position.y;
                        let distance =
                            (dx * dx + dy * dy + (source.z - l.position.z).powi(2)).sqrt();
                        let right = Vect::new(-l.yaw.sin(), l.yaw.cos(), 0.0);
                        let pan =
                            ((dx * right.x + dy * right.y) / distance.max(0.001)).clamp(-1.0, 1.0);
                        (event.gain / (1.0 + 0.15 * distance), pan)
                    });
            sink.play(event.id, gain, pan);
        }
        for event in ambiences {
            sink.set_ambience(event.id, event.gain);
        }
    });
}

/// Registers audio event storage and the end-of-frame playback consumer.
pub struct AudioPlugin;
impl Plugin for AudioPlugin {
    fn name(&self) -> &'static str {
        "AudioPlugin"
    }
    fn build(&self, app: &mut App) {
        app.add_event::<PlaySound>().add_event::<SetAmbience>();
        app.insert_resource(AudioOutput::default())
            .insert_resource(AudioCursors::default());
        app.add_system(Stage::Last, audio_event_system);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    #[derive(Default)]
    struct Calls(Vec<(SoundId, f32, f32)>);
    struct MockSink(Arc<Mutex<Calls>>);
    impl AudioSink for MockSink {
        fn play(&mut self, id: SoundId, gain: f32, pan: f32) {
            self.0.lock().unwrap().0.push((id, gain, pan));
        }
        fn set_ambience(&mut self, _id: Option<AmbienceId>, _gain: f32) {}
    }
    #[test]
    fn event_consumer_applies_listener_relative_spatial_pan_and_falloff() {
        let calls = Arc::new(Mutex::new(Calls::default()));
        let mut world = World::new();
        world.add_event::<PlaySound>();
        world.add_event::<SetAmbience>();
        world.insert_resource(AudioCursors::default());
        world.insert_resource(AudioOutput(Some(Box::new(MockSink(Arc::clone(&calls))))));
        world.spawn((AudioListener {
            position: Vect::default(),
            yaw: 0.0,
        },));
        world.send_event(PlaySound {
            id: SoundId(4),
            position: Some(Vect::new(0.0, 2.0, 0.0)),
            gain: 1.0,
        });
        audio_event_system(&mut world);
        let output = calls.lock().unwrap();
        assert_eq!(output.0[0].0, SoundId(4));
        assert!((output.0[0].1 - 1.0 / 1.3).abs() < 1e-6);
        assert!((output.0[0].2 - 1.0).abs() < 1e-6);
    }
}
