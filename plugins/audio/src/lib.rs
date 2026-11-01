//! Sound-bank data and event-driven audio playback integration.
//!
//! Device output is supplied through `AudioSink`; games own IDs and embedded sound assets.

use ribbon_app::{App, LogBuffer, LogLevel, Stage};
use ribbon_assets::AssetServer;
use ribbon_ecs::World;
use ribbon_events::{EventCursor, WorldEventsExt};
use ribbon_math::{forward_vec, Vect};
use ribbon_plugins::Plugin;
use ribbon_scene::GlobalTransform;

pub mod assets;
pub mod automation;
pub mod bank;
pub mod debug;
pub mod decode_budget;
pub mod ecs;
#[cfg(feature = "audio")]
pub mod kira_backend;
pub mod mixer;
pub mod mixer_graph;
pub mod propagation;
pub mod registry;
#[cfg(feature = "rodio-compat")]
pub mod rodio_backend;
pub mod spatial;
#[cfg(feature = "steam-audio")]
pub mod steam_audio;
#[cfg(feature = "audio")]
pub mod threaded;
pub mod voices;
pub use assets::{
    register_audio_assets, AudioAssetHandle, AudioAssetLoader, AudioAssetPoll, AudioAssetSnapshot,
    EncodedAudioAsset,
};
pub use automation::{AudioAutomation, AutomationBinding, AutomationCommand, AutomationTarget};
pub use bank::{AmbienceAsset, AmbienceId, SoundAsset, SoundBank, SoundId};
pub use debug::AudioDiagnostics;
pub use ecs::{
    AudioAcousticResponse, AudioEmitter, AudioOcclusionProvider, AudioOcclusionQuery,
    AudioOcclusionResult, AudioZoneVolume,
};
#[cfg(feature = "audio")]
pub use kira_backend::PlaybackOptions;
pub use mixer::{AudioDistortionKind, AudioEffect, AudioSnapshotRegistry, MixerSnapshot};
pub use mixer_graph::{
    AudioSendConfig, AudioSendEffect, AudioSendId, CustomBusConfig, CustomBusId, MixerBusNode,
    MixerBusParent, MixerBusParent as AudioMixerBusParent, MixerGraph, MixerSendRoute,
};
pub use propagation::{
    AcousticMaterial, AcousticPortalId, AcousticPropagationGraph, AcousticPropagationResult,
    AcousticZoneId, DoorPortalState, PortalAcousticEdge,
};
pub use registry::{
    AudioBus, AudioEventDef, AudioEventDescriptor, AudioEventKind, AudioEventRegistry,
    AudioEventRuntime, AudioParameter, AudioParameters, ResolvedAudioEvent,
};
pub use spatial::{
    calculate_with_model, AcousticZone, DistanceModel, Emitter3d, Listener3d, SpatialResult,
    Vec3 as AudioVec3,
};
#[cfg(feature = "steam-audio")]
pub use steam_audio::{
    SteamAudioBinauralEffectBuilder, SteamAudioBinauralHandle, SteamAudioBinauralRenderer,
    SteamAudioDirectEffectBuilder, SteamAudioDirectEffectHandle,
};
#[cfg(feature = "audio")]
pub use threaded::ThreadedAudioSink;
pub use voices::{
    PlaybackStatus, VoiceAdmission, VoiceCompletions, VoiceId, VoiceManager, VoiceSpec, VoiceState,
};

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
    /// Steam Audio listener-local direction (right, up, behind) for automatic ECS binaural playback.
    #[cfg(feature = "steam-audio")]
    pub binaural_direction: Option<[f32; 3]>,
}

/// Listener-relative controls and material response for an already-playing spatial voice.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ManagedEmitterAcoustics {
    pub gain: f32,
    pub pan: f32,
    pub playback_rate: f32,
    pub occlusion: f32,
    pub transmission_gain: f32,
    pub low_pass_cutoff_hz: f32,
    /// Updated source direction for an already-playing Steam Audio binaural voice.
    #[cfg(feature = "steam-audio")]
    pub binaural_direction: Option<[f32; 3]>,
}

/// Platform-facing output. A missing output keeps the engine in silent mode.
pub trait AudioSink: Send + Sync + 'static {
    fn play(&mut self, id: SoundId, gain: f32, pan: f32);
    fn set_ambience(&mut self, id: Option<AmbienceId>, gain: f32);

    /// Drain playback handles which completed since the previous poll. Legacy sinks may return
    /// an empty batch; threaded backends should implement this without blocking the game thread.
    fn take_completed_voices(&mut self) -> VoiceCompletions {
        VoiceCompletions::default()
    }
    fn play_spatial_advanced(&mut self, id: SoundId, gain: f32, pan: f32, _doppler_ratio: f32) {
        self.play(id, gain, pan);
    }

    /// Starts a controllable emitter voice when the backend supports opaque voice handles.
    /// Legacy sinks get one best-effort one-shot and return `None`.
    /// Whether this sink supports stable, controllable emitter voices. Legacy sinks return false
    /// because their default adapter emits a one-shot and intentionally has no persistent handle.
    fn supports_managed_voices(&self) -> bool {
        false
    }

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

    /// Apply a material-aware acoustic response to a moving emitter. Legacy sinks retain their
    /// previous interface and receive the transmission factor as an additional gain multiplier.
    fn update_managed_emitter_acoustics(
        &mut self,
        voice: VoiceId,
        acoustics: ManagedEmitterAcoustics,
    ) {
        self.update_managed_emitter(
            voice,
            acoustics.gain * acoustics.transmission_gain.clamp(0.0, 1.0),
            acoustics.pan,
            acoustics.playback_rate,
            acoustics.occlusion,
        );
    }

    /// Stop a voice controlled by `play_managed_emitter`.
    fn stop_managed_emitter(&mut self, _voice: VoiceId, _fade_seconds: f32) {}

    /// Apply environmental effects on a bus. Backends should smooth parameter changes.
    fn set_bus_effects(&mut self, _bus: AudioBus, _effects: &[AudioEffect]) {}
    /// Checked variant used by ECS synchronization so a full command queue does not permanently
    /// suppress a zone update. Legacy sinks that cannot fail keep their previous behavior.
    fn try_set_bus_effects(
        &mut self,
        bus: AudioBus,
        effects: &[AudioEffect],
    ) -> Result<(), String> {
        self.set_bus_effects(bus, effects);
        Ok(())
    }
    /// Apply a zone-specific gain multiplier without overwriting the game's configured bus gain.
    fn set_bus_zone_gain(&mut self, _bus: AudioBus, _gain: f32, _fade_seconds: f32) {}
    /// Checked variant for reliable retry after bounded queue saturation.
    fn try_set_bus_zone_gain(
        &mut self,
        bus: AudioBus,
        gain: f32,
        fade_seconds: f32,
    ) -> Result<(), String> {
        self.set_bus_zone_gain(bus, gain, fade_seconds);
        Ok(())
    }
}
#[derive(Default)]
pub struct AudioOutput(pub Option<Box<dyn AudioSink>>);

/// ECS event emitted once when a backend reports a playback handle as completed or retired.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioVoiceCompleted {
    pub voice: VoiceId,
}

/// Diagnostic event indicating that a backend's bounded completion queue overflowed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioCompletionOverflow {
    pub dropped_total: u64,
}

#[derive(Default)]
struct AudioAssetLoaderRegistered;

fn register_audio_asset_loader_system(world: &mut World) {
    if world.try_resource::<AudioAssetLoaderRegistered>().is_some()
        || world.try_resource::<AssetServer>().is_none()
    {
        return;
    }
    world.resource_scope::<AssetServer, _>(|_, server| assets::register_audio_assets(server));
    world.insert_resource(AudioAssetLoaderRegistered);
}
#[derive(Default)]
struct AudioCursors {
    sounds: EventCursor<PlaySound>,
    ambience: EventCursor<SetAmbience>,
    spatial_sounds: EventCursor<PlaySpatialSound>,
    /// Ambience is persistent state, so retain only its latest value while output is unavailable.
    pending_ambience: Option<SetAmbience>,
    warned_silent: bool,
    completion_dropped_total: u64,
}

/// Poll backend completions, optionally release a world-resident event runtime's reservations,
/// and publish completion events for gameplay systems. The backend poll is non-blocking.
pub fn audio_completion_system(world: &mut World) {
    let completions = world.resource_scope::<AudioOutput, _>(|_, output| {
        output
            .0
            .as_deref_mut()
            .map(AudioSink::take_completed_voices)
            .unwrap_or_default()
    });

    if world.try_resource::<AudioEventRuntime>().is_some() {
        world
            .resource_mut::<AudioEventRuntime>()
            .finish_completions(&completions);
    }
    ecs::reconcile_emitter_completions(world, &completions);

    for voice in completions.voices {
        world.send_event(AudioVoiceCompleted { voice });
    }

    let dropped_delta = world.resource_scope::<AudioCursors, _>(|_, cursors| {
        let delta = completions
            .dropped_notifications_total
            .saturating_sub(cursors.completion_dropped_total);
        cursors.completion_dropped_total = cursors
            .completion_dropped_total
            .max(completions.dropped_notifications_total);
        delta
    });
    if dropped_delta > 0 {
        world.send_event(AudioCompletionOverflow {
            dropped_total: completions.dropped_notifications_total,
        });
    }
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
    if let Some(latest) = ambiences.last().copied() {
        world.resource_mut::<AudioCursors>().pending_ambience = Some(latest);
    }
    let has_pending_ambience = world.resource::<AudioCursors>().pending_ambience.is_some();
    if sounds.is_empty() && spatial_sounds.is_empty() && !has_pending_ambience {
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
    let pending_ambience = world.resource_mut::<AudioCursors>().pending_ambience.take();
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
        if let Some(event) = pending_ambience {
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
            .add_event::<PlaySpatialSound>()
            .add_event::<AudioVoiceCompleted>()
            .add_event::<AudioCompletionOverflow>();
        app.insert_resource(AudioOutput::default())
            .insert_resource(AudioCursors::default())
            .insert_resource(ecs::AudioEmitterRuntime::default());
        // The AssetPlugin may be registered before or after AudioPlugin. Register the loader on
        // the first frame where AssetServer exists, exactly once, without creating a second cache.
        app.add_system(Stage::First, register_audio_asset_loader_system);
        app.add_system(Stage::Last, audio_event_system);
        app.add_system(Stage::Last, audio_completion_system);
        app.add_system(Stage::Last, ecs::audio_emitter_system);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    #[derive(Default)]
    struct Calls {
        sounds: Vec<(SoundId, f32, f32)>,
        ambiences: Vec<(Option<AmbienceId>, f32)>,
    }
    struct MockSink(Arc<Mutex<Calls>>);
    impl AudioSink for MockSink {
        fn play(&mut self, id: SoundId, gain: f32, pan: f32) {
            self.0.lock().unwrap().sounds.push((id, gain, pan));
        }
        fn set_ambience(&mut self, id: Option<AmbienceId>, gain: f32) {
            self.0.lock().unwrap().ambiences.push((id, gain));
        }
    }

    struct CompletionSink(Option<VoiceCompletions>);
    impl AudioSink for CompletionSink {
        fn play(&mut self, _id: SoundId, _gain: f32, _pan: f32) {}
        fn set_ambience(&mut self, _id: Option<AmbienceId>, _gain: f32) {}
        fn take_completed_voices(&mut self) -> VoiceCompletions {
            self.0.take().unwrap_or_default()
        }
    }

    #[test]
    fn completion_system_publishes_voices_and_releases_bound_event_slots() {
        let mut registry = AudioEventRegistry::default();
        registry
            .register_variants(
                "impact",
                AudioEventDescriptor {
                    variants: vec![SoundId(4)],
                    gain_min: 1.0,
                    gain_max: 1.0,
                    playback_rate_min: 1.0,
                    playback_rate_max: 1.0,
                    priority: 10,
                    max_instances: 1,
                    cooldown_seconds: 0.0,
                    bus: AudioBus::Sfx,
                    looping: false,
                },
                &[SoundId(4)],
            )
            .unwrap();
        let mut runtime = AudioEventRuntime::new(7);
        runtime.trigger(&registry, "impact", 0.0).unwrap();
        runtime.bind_voice("impact", VoiceId(77)).unwrap();

        let mut world = World::new();
        world.add_event::<AudioVoiceCompleted>();
        world.add_event::<AudioCompletionOverflow>();
        world.insert_resource(AudioCursors::default());
        world.insert_resource(runtime);
        world.insert_resource(AudioOutput(Some(Box::new(CompletionSink(Some(
            VoiceCompletions {
                voices: vec![VoiceId(77)],
                dropped_notifications_total: 0,
                active_voices: Some(Vec::new()),
            },
        ))))));

        audio_completion_system(&mut world);

        assert_eq!(world.events::<AudioVoiceCompleted>().len(), 1);
        assert_eq!(world.events::<AudioCompletionOverflow>().len(), 0);
        assert_eq!(
            world
                .resource::<AudioEventRuntime>()
                .active_instances("impact"),
            0
        );
    }

    #[test]
    fn latest_ambience_is_retained_until_an_audio_sink_becomes_available() {
        let calls = Arc::new(Mutex::new(Calls::default()));
        let mut world = World::new();
        world.add_event::<PlaySound>();
        world.add_event::<SetAmbience>();
        world.add_event::<PlaySpatialSound>();
        world.insert_resource(AudioCursors::default());
        world.insert_resource(AudioOutput::default());
        world.insert_resource(LogBuffer::default());
        world.send_event(SetAmbience {
            id: Some(AmbienceId(9)),
            gain: 0.6,
        });

        audio_event_system(&mut world);
        assert_eq!(
            world
                .resource::<AudioCursors>()
                .pending_ambience
                .map(|event| event.id),
            Some(Some(AmbienceId(9)))
        );

        world.resource_mut::<AudioOutput>().0 = Some(Box::new(MockSink(Arc::clone(&calls))));
        audio_event_system(&mut world);
        assert_eq!(calls.lock().unwrap().sounds.len(), 0);
        assert_eq!(
            calls.lock().unwrap().ambiences.last(),
            Some(&(Some(AmbienceId(9)), 0.6))
        );
        assert!(world.resource::<AudioCursors>().pending_ambience.is_none());
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
            assert_eq!(output.sounds.len(), 1);
            assert_eq!(output.sounds[0].0, SoundId(4));
            assert!((output.sounds[0].1 - 1.0 / 1.3).abs() < 1e-6);
            assert!((output.sounds[0].2 - 1.0).abs() < 1e-6);
        }
        // Calling the consumer again in the same frame must not replay an already consumed event.
        audio_event_system(&mut world);
        assert_eq!(calls.lock().unwrap().sounds.len(), 1);
    }
}
