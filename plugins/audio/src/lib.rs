//! Sound-bank data and event-driven audio playback integration.
//!
//! Device output is supplied through `AudioSink`; games own IDs and embedded sound assets.

use ribbon_app::{App, LogBuffer, LogLevel, Stage};
use ribbon_ecs::World;
use ribbon_events::{EventCursor, WorldEventsExt};
use ribbon_math::{forward_vec, Vect};
use ribbon_plugins::Plugin;
use ribbon_scene::GlobalTransform;

pub mod assets;
pub mod bank;
pub mod debug;
pub mod ecs;
#[cfg(feature = "audio")]
pub mod kira_backend;
pub mod mixer;
pub mod registry;
#[cfg(feature = "rodio-compat")]
pub mod rodio_backend;
pub mod spatial;
#[cfg(feature = "audio")]
pub mod threaded;
pub mod voices;
pub use assets::{AudioAssetLoader, EncodedAudioAsset};
pub use bank::{AmbienceAsset, AmbienceId, SoundAsset, SoundBank, SoundId};
pub use debug::AudioDiagnostics;
pub use ecs::{
    AudioEmitter, AudioOcclusionProvider, AudioOcclusionQuery, AudioOcclusionResult,
    AudioZoneVolume,
};
#[cfg(feature = "audio")]
pub use kira_backend::PlaybackOptions;
pub use mixer::{AudioEffect, MixerSnapshot};
pub use registry::{
    AudioBus, AudioEventDef, AudioEventDescriptor, AudioEventKind, AudioEventRegistry,
    AudioEventRuntime, AudioParameter, AudioParameters, ResolvedAudioEvent,
};
pub use spatial::{AcousticZone, Emitter3d, Listener3d, SpatialResult, Vec3 as AudioVec3};
#[cfg(feature = "audio")]
pub use threaded::ThreadedAudioSink;
pub use voices::{PlaybackStatus, VoiceAdmission, VoiceId, VoiceManager, VoiceSpec, VoiceState};

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
/// Full 3D one-shot request. `gain` is an event-level multiplier on the emitter's own gain.
#[derive(Clone, Copy, Debug)]
pub struct PlaySpatialSound {
    pub id: SoundId,
    pub emitter: Emitter3d,
    pub gain: f32,
}
/// Optional full 3D listener pose. If absent, the legacy `AudioListener` is adapted to 3D.
#[derive(Clone, Copy, Debug, Default)]
pub struct SpatialAudioListener(pub Listener3d);
/// Listener pose in world space.
#[derive(Clone, Copy, Debug, Default)]
pub struct AudioListener {
    pub position: Vect,
    pub yaw: f32,
}

/// Parameters used to start a controllable spatial emitter voice.
#[derive(Clone, Copy, Debug)]
pub struct ManagedEmitterPlayback {
    pub sound: SoundId,
    pub gain: f32,
    pub pan: f32,
    pub playback_rate: f32,
    pub looping: bool,
    pub priority: u8,
    pub bus: AudioBus,
}

/// Platform-facing output. A missing output keeps the engine in silent mode.
pub trait AudioSink: Send + Sync + 'static {
    fn play(&mut self, id: SoundId, gain: f32, pan: f32);
    fn set_ambience(&mut self, id: Option<AmbienceId>, gain: f32);
    fn play_spatial_advanced(&mut self, id: SoundId, gain: f32, pan: f32, _doppler_ratio: f32) {
        self.play(id, gain, pan);
    }

    /// Starts a controllable emitter voice when the backend supports opaque voice handles.
    /// Legacy sinks get one best-effort one-shot and return `None`.
    fn play_managed_emitter(&mut self, playback: ManagedEmitterPlayback) -> Option<VoiceId> {
        let _ = (
            playback.playback_rate,
            playback.looping,
            playback.priority,
            playback.bus,
        );
        self.play_spatial_advanced(playback.sound, playback.gain, playback.pan, 1.0);
        None
    }

    /// Refresh spatial controls on a live emitter. Unsupported by legacy sinks.
    fn update_managed_emitter(
        &mut self,
        _voice: VoiceId,
        _gain: f32,
        _pan: f32,
        _playback_rate: f32,
        _occlusion: f32,
    ) {
    }

    /// Stop a voice controlled by `play_managed_emitter`.
    fn stop_managed_emitter(&mut self, _voice: VoiceId, _fade_seconds: f32) {}

    /// Apply environmental effects on a bus. Backends should smooth parameter changes.
    fn set_bus_effects(&mut self, _bus: AudioBus, _effects: &[AudioEffect]) {}
    /// Apply a zone-specific gain multiplier without overwriting the game's configured bus gain.
    fn set_bus_zone_gain(&mut self, _bus: AudioBus, _gain: f32, _fade_seconds: f32) {}
}
#[derive(Default)]
pub struct AudioOutput(pub Option<Box<dyn AudioSink>>);
#[derive(Default)]
struct AudioCursors {
    sounds: EventCursor<PlaySound>,
    ambience: EventCursor<SetAmbience>,
    spatial_sounds: EventCursor<PlaySpatialSound>,
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
    let spatial_sounds = world.resource_scope::<AudioCursors, _>(|world, cursor| {
        cursor
            .spatial_sounds
            .read(world.events::<PlaySpatialSound>())
            .copied()
            .collect::<Vec<_>>()
    });
    if sounds.is_empty() && ambiences.is_empty() && spatial_sounds.is_empty() {
        return;
    }
    let listener = world
        .query::<&AudioListener>()
        .iter()
        .min_by_key(|(entity, _)| entity.to_bits())
        .map(|(_, listener)| *listener);
    let spatial_listener = world
        .query::<(&SpatialAudioListener, &GlobalTransform)>()
        .iter()
        .min_by_key(|(entity, _)| entity.to_bits())
        .map(|(_, (listener, transform))| Listener3d {
            position: AudioVec3::new(
                transform.translation.x,
                transform.translation.y,
                transform.translation.z,
            ),
            forward: AudioVec3::new(
                forward_vec(transform.rotation).x,
                forward_vec(transform.rotation).y,
                forward_vec(transform.rotation).z,
            ),
            up: AudioVec3::new(0.0, 0.0, 1.0),
            velocity: listener.0.velocity,
        })
        .or_else(|| {
            world
                .query::<&SpatialAudioListener>()
                .iter()
                .min_by_key(|(entity, _)| entity.to_bits())
                .map(|(_, listener)| listener.0)
        })
        .or_else(|| {
            listener.map(|listener| Listener3d {
                position: AudioVec3::new(
                    listener.position.x,
                    listener.position.y,
                    listener.position.z,
                ),
                forward: AudioVec3::new(listener.yaw.sin(), listener.yaw.cos(), 0.0),
                up: AudioVec3::new(0.0, 0.0, 1.0),
                velocity: AudioVec3::ZERO,
            })
        });
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
            let base_gain = if event.gain.is_finite() {
                event.gain.clamp(0.0, 1.0)
            } else {
                0.0
            };
            let (gain, pan) =
                event
                    .position
                    .zip(listener)
                    .map_or((base_gain, 0.0), |(source, l)| {
                        let dx = source.x - l.position.x;
                        let dy = source.y - l.position.y;
                        let distance =
                            (dx * dx + dy * dy + (source.z - l.position.z).powi(2)).sqrt();
                        let right = Vect::new(l.yaw.cos(), -l.yaw.sin(), 0.0);
                        let pan =
                            ((dx * right.x + dy * right.y) / distance.max(0.001)).clamp(-1.0, 1.0);
                        let gain = base_gain / (1.0 + 0.15 * distance.max(0.0));
                        (gain, if pan.is_finite() { pan } else { 0.0 })
                    });
            sink.play(event.id, gain, pan);
        }
        for event in spatial_sounds {
            let Some(listener) = spatial_listener else {
                continue;
            };
            let result = spatial::calculate(listener, event.emitter, 343.0);
            let event_gain = if event.gain.is_finite() {
                event.gain.clamp(0.0, 1.0)
            } else {
                0.0
            };
            sink.play_spatial_advanced(
                event.id,
                (result.gain * event_gain).clamp(0.0, 1.0),
                result.pan,
                result.doppler_ratio,
            );
        }
        for event in ambiences {
            let gain = if event.gain.is_finite() {
                event.gain.clamp(0.0, 1.0)
            } else {
                0.0
            };
            sink.set_ambience(if gain > 0.0 { event.id } else { None }, gain);
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
        app.add_event::<PlaySound>()
            .add_event::<SetAmbience>()
            .add_event::<PlaySpatialSound>();
        app.insert_resource(AudioOutput::default())
            .insert_resource(AudioCursors::default())
            .insert_resource(ecs::AudioEmitterRuntime::default());
        app.add_system(Stage::Last, audio_event_system);
        app.add_system(Stage::Last, ecs::audio_emitter_system);
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
        world.add_event::<PlaySpatialSound>();
        world.insert_resource(AudioCursors::default());
        world.insert_resource(AudioOutput(Some(Box::new(MockSink(Arc::clone(&calls))))));
        world.spawn((AudioListener {
            position: Vect::default(),
            yaw: 0.0,
        },));
        world.send_event(PlaySound {
            id: SoundId(4),
            position: Some(Vect::new(2.0, 0.0, 0.0)),
            gain: 1.0,
        });
        audio_event_system(&mut world);
        {
            let output = calls.lock().unwrap();
            assert_eq!(output.0.len(), 1);
            assert_eq!(output.0[0].0, SoundId(4));
            assert!((output.0[0].1 - 1.0 / 1.3).abs() < 1e-6);
            assert!((output.0[0].2 - 1.0).abs() < 1e-6);
        }
        // Calling the consumer again in the same frame must not replay an already consumed event.
        audio_event_system(&mut world);
        assert_eq!(calls.lock().unwrap().0.len(), 1);
    }
}
