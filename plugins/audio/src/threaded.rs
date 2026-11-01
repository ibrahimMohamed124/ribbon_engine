//! Bounded, non-blocking command bridge to the Kira backend.
//!
//! All `AudioManager`, sound handles, decoder and mixer-track operations stay on the named
//! worker thread. Gameplay receives a stable `VoiceId` immediately after a command is accepted
//! into the bounded queue; backend rejection is exposed by `backend_failures()`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::assets::AudioAssetHandle;
use crate::automation::AutomationCommand;
use crate::bank::{AmbienceId, SoundBank, SoundId};
use crate::debug::AudioDiagnostics;
use crate::kira_backend::{KiraAudioBackend, PlaybackOptions};
use crate::mixer::{validate_effect_chain, AudioEffect, AudioSnapshotRegistry, MixerSnapshot};
use crate::mixer_graph::{AudioSendConfig, AudioSendId, CustomBusConfig, CustomBusId};
use crate::registry::{AudioBus, ResolvedAudioEvent};
use crate::voices::{PlaybackStatus, VoiceCompletions, VoiceId};
use crate::{AudioSink, ManagedEmitterAcoustics};

const COMMAND_CAPACITY: usize = 256;
const MAX_PENDING_STREAM_BYTES: u64 = 64 * 1024 * 1024;
// 256 parameter bindings may additionally collapse into one effect-chain command for each of
// the seven built-in buses.
const MAX_AUTOMATION_COMMANDS: usize = 263;

enum Command {
    Play {
        voice: VoiceId,
        sound: SoundId,
        options: PlaybackOptions,
        doppler: f32,
    },
    StreamFile {
        voice: VoiceId,
        path: PathBuf,
        options: PlaybackOptions,
    },
    StreamBytes {
        voice: VoiceId,
        bytes: Vec<u8>,
        options: PlaybackOptions,
    },
    Stop {
        voice: VoiceId,
        fade_seconds: f32,
    },
    Pause {
        voice: VoiceId,
    },
    Resume {
        voice: VoiceId,
    },
    Gain {
        voice: VoiceId,
        gain: f32,
        fade_seconds: f32,
    },
    Rate {
        voice: VoiceId,
        rate: f32,
        fade_seconds: f32,
    },
    Pan {
        voice: VoiceId,
        pan: f32,
        fade_seconds: f32,
    },
    Looping {
        voice: VoiceId,
        looping: bool,
    },
    StopBus {
        bus: AudioBus,
        fade_seconds: f32,
    },
    PauseBus {
        bus: AudioBus,
    },
    ResumeBus {
        bus: AudioBus,
    },
    StopAll {
        fade_seconds: f32,
    },
    Ambience {
        id: Option<AmbienceId>,
        gain: f32,
    },
    AmbienceLayer {
        id: AmbienceId,
        gain: f32,
        fade_seconds: f32,
    },
    StopAmbienceLayer {
        id: AmbienceId,
        fade_seconds: f32,
    },
    BusGain {
        bus: AudioBus,
        gain: f32,
        fade_seconds: f32,
    },
    BusMute {
        bus: AudioBus,
        muted: bool,
        fade_seconds: f32,
    },
    BusZoneGain {
        bus: AudioBus,
        gain: f32,
        fade_seconds: f32,
    },
    Effects {
        bus: AudioBus,
        effects: Vec<AudioEffect>,
        transition_seconds: f32,
    },
    Snapshot {
        gains: Vec<(AudioBus, f32)>,
        mutes: Vec<(AudioBus, bool)>,
        zone_gains: Vec<(AudioBus, f32)>,
        effects: Vec<(AudioBus, Vec<AudioEffect>)>,
        transition_seconds: f32,
    },
    AutomationFrame {
        commands: Vec<AutomationCommand>,
        transition_seconds: f32,
    },
    VoiceState {
        voice: VoiceId,
        reply: SyncSender<Option<PlaybackStatus>>,
    },
    CompletedVoices {
        reply: SyncSender<VoiceCompletions>,
    },
    Diagnostics {
        reply: SyncSender<AudioDiagnostics>,
    },
    SpatialUpdate {
        voice: VoiceId,
        acoustics: ManagedEmitterAcoustics,
    },
    #[cfg(feature = "steam-audio")]
    BinauralDirection {
        voice: VoiceId,
        direction: [f32; 3],
    },
    CreateSend(AudioSendConfig),
    RemoveSend(AudioSendId),
    SendGain {
        id: AudioSendId,
        gain: f32,
        fade_seconds: f32,
    },
    CreateCustomBus(CustomBusConfig),
    RemoveCustomBus(CustomBusId),
    CustomBusGain {
        id: CustomBusId,
        gain: f32,
        fade_seconds: f32,
    },
    CustomBusMute {
        id: CustomBusId,
        muted: bool,
        fade_seconds: f32,
    },
    CustomBusEffects {
        id: CustomBusId,
        effects: Vec<AudioEffect>,
        fade_seconds: f32,
    },
    CustomBusSendGain {
        bus: CustomBusId,
        send: AudioSendId,
        gain: f32,
        fade_seconds: f32,
    },
    StopCustomBus {
        id: CustomBusId,
        fade_seconds: f32,
    },
    PauseCustomBus(CustomBusId),
    ResumeCustomBus(CustomBusId),
}

/// Thread-safe façade over Kira with a strictly bounded command queue.
pub struct ThreadedAudioSink {
    sender: Option<SyncSender<Command>>,
    worker: Option<JoinHandle<()>>,
    next_voice: AtomicU64,
    dropped_commands: Arc<AtomicU64>,
    backend_failures: Arc<AtomicU64>,
    last_backend_error: Arc<Mutex<Option<String>>>,
    pending_stream_bytes: Arc<AtomicU64>,
    completion_reply: Mutex<Option<Receiver<VoiceCompletions>>>,
    known_sounds: HashSet<SoundId>,
}

impl ThreadedAudioSink {
    /// Initialize Kira on a dedicated thread. Fails cleanly if the audio device cannot start.
    pub fn new(bank: SoundBank) -> Result<Self, String> {
        bank.validate()
            .map_err(|error| format!("invalid audio sound bank: {error}"))?;
        let known_sounds = bank.sounds().map(|asset| asset.id).collect();
        let (sender, receiver) = mpsc::sync_channel(COMMAND_CAPACITY);
        let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<(), String>>(1);
        let dropped_commands = Arc::new(AtomicU64::new(0));
        let backend_failures = Arc::new(AtomicU64::new(0));
        let last_backend_error = Arc::new(Mutex::new(None));
        let pending_stream_bytes = Arc::new(AtomicU64::new(0));
        let failures_for_worker = Arc::clone(&backend_failures);
        let last_error_for_worker = Arc::clone(&last_backend_error);
        let pending_bytes_for_worker = Arc::clone(&pending_stream_bytes);
        let worker = thread::Builder::new()
            .name("ribbon-audio-kira".to_owned())
            .spawn(move || {
                worker_main(
                    bank,
                    receiver,
                    ready_tx,
                    failures_for_worker,
                    last_error_for_worker,
                    pending_bytes_for_worker,
                )
            })
            .map_err(|error| format!("could not start audio worker: {error}"))?;

        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                sender: Some(sender),
                worker: Some(worker),
                next_voice: AtomicU64::new(1),
                dropped_commands,
                backend_failures,
                last_backend_error,
                pending_stream_bytes,
                completion_reply: Mutex::new(None),
                known_sounds,
            }),
            Ok(Err(error)) => {
                let _ = worker.join();
                Err(error)
            }
            Err(error) => {
                let _ = worker.join();
                Err(format!("Kira worker exited during initialization: {error}"))
            }
        }
    }

    /// Commands rejected because the bounded queue was full or closed.
    pub fn dropped_commands(&self) -> u64 {
        self.dropped_commands.load(Ordering::Relaxed)
    }
    /// Backend-side failures (for example, a decode or resource admission failure).
    pub fn backend_failures(&self) -> u64 {
        self.backend_failures.load(Ordering::Relaxed)
    }
    /// Most recent backend-side error. Repeated identical errors replace one bounded diagnostic
    /// value rather than growing an unbounded log buffer.
    pub fn last_backend_error(&self) -> Option<String> {
        self.last_backend_error
            .lock()
            .ok()
            .and_then(|error| error.clone())
    }

    fn enqueue(&self, command: Command) -> Result<(), String> {
        let Some(sender) = self.sender.as_ref() else {
            self.dropped_commands.fetch_add(1, Ordering::Relaxed);
            return Err("audio command channel is closed".to_owned());
        };
        sender.try_send(command).map_err(|error| {
            self.dropped_commands.fetch_add(1, Ordering::Relaxed);
            match error {
                TrySendError::Full(_) => "audio command queue is full".to_owned(),
                TrySendError::Disconnected(_) => "audio worker is unavailable".to_owned(),
            }
        })
    }

    fn allocate_voice(&self) -> Result<VoiceId, String> {
        allocate_voice_id(&self.next_voice)
    }

    fn record_facade_failure(&self, error: &str) {
        self.backend_failures.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut stored) = self.last_backend_error.lock() {
            if stored.as_deref() != Some(error) {
                *stored = Some(error.to_owned());
            }
        }
    }

    /// Queue a static registered sound. The returned ID may be used to control this instance.
    /// An `Ok` result means the command entered the queue; device-side admission remains async.
    pub fn play_voice(&self, sound: SoundId, options: PlaybackOptions) -> Result<VoiceId, String> {
        if !self.known_sounds.contains(&sound) {
            return Err(format!("unknown sound id {:?}", sound));
        }
        let voice = self.allocate_voice()?;
        self.enqueue(Command::Play {
            voice,
            sound,
            options,
            doppler: 1.0,
        })?;
        Ok(voice)
    }

    /// Resolve and submit a variant-aware sound event. The caller's `AudioEventRuntime` owns
    /// cooldown/instance accounting; call `finish(event.name)` when that voice completes.
    pub fn play_resolved_event(&self, event: &ResolvedAudioEvent) -> Result<VoiceId, String> {
        self.play_voice(
            event.sound,
            PlaybackOptions {
                gain: event.gain,
                playback_rate: event.playback_rate,
                priority: event.priority,
                bus: event.bus,
                looping: event.looping,
                ..PlaybackOptions::default()
            },
        )
    }

    /// Play a file as a Kira streaming sound. This does not decode and retain the entire PCM clip.
    pub fn play_streaming_file(
        &self,
        path: impl AsRef<Path>,
        options: PlaybackOptions,
    ) -> Result<VoiceId, String> {
        let voice = self.allocate_voice()?;
        self.enqueue(Command::StreamFile {
            voice,
            path: path.as_ref().to_path_buf(),
            options,
        })?;
        Ok(voice)
    }

    /// Play an encoded byte source through Kira's streaming decoder (encoded bytes are retained;
    /// decoded PCM is produced in chunks). Useful for packaged long tracks.
    /// Stream a dynamic Ribbon audio asset through Kira's chunked decoder. The handle must have
    /// completed its initial load (`AudioAssetHandle::snapshot` is then available). The immutable
    /// revision snapshot makes a hot reload affect future playbacks without mutating bytes already
    /// submitted to a live voice. Use the embedded `SoundBank` path for frequently reused short
    /// clips that should be predecoded and cached as static sounds.
    pub fn play_audio_asset(
        &self,
        asset: &AudioAssetHandle,
        options: PlaybackOptions,
    ) -> Result<VoiceId, String> {
        let snapshot = asset
            .snapshot()
            .ok_or_else(|| format!("audio asset `{}` is not loaded yet", asset.path()))?;
        if snapshot.bytes.is_empty() {
            return Err(format!(
                "audio asset `{}` has no encoded data",
                snapshot.path
            ));
        }
        // Check before cloning: an oversized in-memory AssetServer source must not allocate a
        // second large Vec merely to be rejected by `play_streaming_bytes`.
        if snapshot.bytes.len() as u64 > MAX_PENDING_STREAM_BYTES {
            return Err(format!(
                "audio asset `{}` exceeds the 64 MiB in-memory stream limit; use play_streaming_file for long tracks",
                snapshot.path
            ));
        }
        self.play_streaming_bytes(snapshot.bytes.as_ref().to_vec(), options)
    }

    pub fn play_streaming_bytes(
        &self,
        bytes: Vec<u8>,
        options: PlaybackOptions,
    ) -> Result<VoiceId, String> {
        if bytes.is_empty() {
            return Err("streaming audio payload is empty".into());
        }
        let size =
            u64::try_from(bytes.len()).map_err(|_| "streaming source size overflow".to_owned())?;
        if size > MAX_PENDING_STREAM_BYTES {
            return Err("streaming byte source exceeds the 64 MiB command-budget limit; use play_streaming_file for long tracks".into());
        }
        self.reserve_stream_bytes(size)?;
        let voice = match self.allocate_voice() {
            Ok(voice) => voice,
            Err(error) => {
                self.pending_stream_bytes.fetch_sub(size, Ordering::AcqRel);
                return Err(error);
            }
        };
        match self.enqueue(Command::StreamBytes {
            voice,
            bytes,
            options,
        }) {
            Ok(()) => Ok(voice),
            Err(error) => {
                self.pending_stream_bytes.fetch_sub(size, Ordering::AcqRel);
                Err(error)
            }
        }
    }

    pub fn stop_voice(&self, voice: VoiceId, fade_seconds: f32) -> Result<(), String> {
        self.enqueue(Command::Stop {
            voice,
            fade_seconds,
        })
    }
    pub fn pause_voice(&self, voice: VoiceId) -> Result<(), String> {
        self.enqueue(Command::Pause { voice })
    }
    pub fn resume_voice(&self, voice: VoiceId) -> Result<(), String> {
        self.enqueue(Command::Resume { voice })
    }
    pub fn set_voice_gain(
        &self,
        voice: VoiceId,
        gain: f32,
        fade_seconds: f32,
    ) -> Result<(), String> {
        self.enqueue(Command::Gain {
            voice,
            gain,
            fade_seconds,
        })
    }
    pub fn set_voice_rate(
        &self,
        voice: VoiceId,
        rate: f32,
        fade_seconds: f32,
    ) -> Result<(), String> {
        self.enqueue(Command::Rate {
            voice,
            rate,
            fade_seconds,
        })
    }
    pub fn set_voice_pan(&self, voice: VoiceId, pan: f32, fade_seconds: f32) -> Result<(), String> {
        self.enqueue(Command::Pan {
            voice,
            pan,
            fade_seconds,
        })
    }
    pub fn set_voice_looping(&self, voice: VoiceId, looping: bool) -> Result<(), String> {
        self.enqueue(Command::Looping { voice, looping })
    }
    pub fn stop_bus(&self, bus: AudioBus, fade_seconds: f32) -> Result<(), String> {
        self.enqueue(Command::StopBus { bus, fade_seconds })
    }
    pub fn pause_bus(&self, bus: AudioBus) -> Result<(), String> {
        self.enqueue(Command::PauseBus { bus })
    }
    pub fn resume_bus(&self, bus: AudioBus) -> Result<(), String> {
        self.enqueue(Command::ResumeBus { bus })
    }
    pub fn stop_all_faded(&self, fade_seconds: f32) -> Result<(), String> {
        self.enqueue(Command::StopAll { fade_seconds })
    }
    /// Request playback state without blocking the caller. Poll the returned receiver with
    /// `try_recv`; a stale/completed ID is reported as `None`.
    pub fn request_voice_state(
        &self,
        voice: VoiceId,
    ) -> Result<Receiver<Option<PlaybackStatus>>, String> {
        let (reply, result) = mpsc::sync_channel(1);
        self.enqueue(Command::VoiceState { voice, reply })?;
        Ok(result)
    }
    /// Request completed playback handles without blocking the caller. Feed the returned voice
    /// IDs to `AudioEventRuntime::finish_voice` to release event concurrency slots automatically.
    /// The response reports cumulative notification overflow so consumers can detect missed IDs.
    pub fn request_completed_voices(&self) -> Result<Receiver<VoiceCompletions>, String> {
        let (reply, result) = mpsc::sync_channel(1);
        self.enqueue(Command::CompletedVoices { reply })?;
        Ok(result)
    }

    /// Poll completed playback handles without waiting for the Kira worker.
    ///
    /// A query is submitted on the first call. Its response is collected on the next call, which
    /// deliberately adds at most one update of observation latency while keeping the game thread
    /// non-blocking. Query submission uses the same bounded command queue as playback controls.
    pub fn poll_completed_voices(&self) -> VoiceCompletions {
        let Ok(mut pending) = self.completion_reply.lock() else {
            self.record_facade_failure("completion reply lock is poisoned");
            return VoiceCompletions::default();
        };

        let mut result = VoiceCompletions::default();
        if let Some(receiver) = pending.as_ref() {
            match receiver.try_recv() {
                Ok(completions) => {
                    result = completions;
                    *pending = None;
                }
                Err(mpsc::TryRecvError::Empty) => return result,
                Err(mpsc::TryRecvError::Disconnected) => {
                    *pending = None;
                    self.record_facade_failure("audio worker dropped completion response");
                }
            }
        }

        if pending.is_none() {
            let (reply, response) = mpsc::sync_channel(1);
            if self.enqueue(Command::CompletedVoices { reply }).is_ok() {
                *pending = Some(response);
            }
        }
        result
    }
    /// Request owner-thread metrics without forcing Kira handles to cross threads.
    pub fn request_diagnostics(&self) -> Result<Receiver<AudioDiagnostics>, String> {
        let (reply, result) = mpsc::sync_channel(1);
        self.enqueue(Command::Diagnostics { reply })?;
        Ok(result)
    }
    pub fn set_bus_gain(&self, bus: AudioBus, gain: f32, fade_seconds: f32) -> Result<(), String> {
        if !gain.is_finite() || !(0.0..=2.0).contains(&gain) {
            return Err("bus gain must be finite and within [0, 2]".into());
        }
        validate_transition_seconds(fade_seconds)?;
        self.enqueue(Command::BusGain {
            bus,
            gain,
            fade_seconds,
        })
    }
    pub fn set_bus_muted(
        &self,
        bus: AudioBus,
        muted: bool,
        fade_seconds: f32,
    ) -> Result<(), String> {
        validate_transition_seconds(fade_seconds)?;
        self.enqueue(Command::BusMute {
            bus,
            muted,
            fade_seconds,
        })
    }
    /// Apply a separate acoustic-zone multiplier without overwriting the user-configured bus gain.
    pub fn set_bus_zone_gain(
        &self,
        bus: AudioBus,
        gain: f32,
        fade_seconds: f32,
    ) -> Result<(), String> {
        if !gain.is_finite() || !(0.0..=2.0).contains(&gain) {
            return Err("bus zone gain must be finite and within [0, 2]".into());
        }
        validate_transition_seconds(fade_seconds)?;
        self.enqueue(Command::BusZoneGain {
            bus,
            gain,
            fade_seconds,
        })
    }
    pub fn set_bus_effects(&self, bus: AudioBus, effects: Vec<AudioEffect>) -> Result<(), String> {
        validate_effect_chain(&effects)?;
        self.enqueue(Command::Effects {
            bus,
            effects,
            transition_seconds: 0.08,
        })
    }

    /// Create a shared effect send. Custom bus routes to this send are declared when the bus
    /// is created; route topology is intentionally immutable while its Kira track is alive.
    pub fn create_audio_send(&self, config: AudioSendConfig) -> Result<(), String> {
        self.enqueue(Command::CreateSend(config))
    }
    pub fn remove_audio_send(&self, id: AudioSendId) -> Result<(), String> {
        self.enqueue(Command::RemoveSend(id))
    }
    pub fn set_audio_send_gain(
        &self,
        id: AudioSendId,
        gain: f32,
        fade_seconds: f32,
    ) -> Result<(), String> {
        if !gain.is_finite() || !(0.0..=2.0).contains(&gain) {
            return Err("send gain must be finite and within [0, 2]".into());
        }
        validate_transition_seconds(fade_seconds)?;
        self.enqueue(Command::SendGain {
            id,
            gain,
            fade_seconds,
        })
    }
    pub fn create_custom_bus(&self, config: CustomBusConfig) -> Result<(), String> {
        self.enqueue(Command::CreateCustomBus(config))
    }
    pub fn remove_custom_bus(&self, id: CustomBusId) -> Result<(), String> {
        self.enqueue(Command::RemoveCustomBus(id))
    }
    pub fn set_custom_bus_gain(
        &self,
        id: CustomBusId,
        gain: f32,
        fade_seconds: f32,
    ) -> Result<(), String> {
        if !gain.is_finite() || !(0.0..=2.0).contains(&gain) {
            return Err("custom bus gain must be finite and within [0, 2]".into());
        }
        validate_transition_seconds(fade_seconds)?;
        self.enqueue(Command::CustomBusGain {
            id,
            gain,
            fade_seconds,
        })
    }
    pub fn set_custom_bus_muted(
        &self,
        id: CustomBusId,
        muted: bool,
        fade_seconds: f32,
    ) -> Result<(), String> {
        validate_transition_seconds(fade_seconds)?;
        self.enqueue(Command::CustomBusMute {
            id,
            muted,
            fade_seconds,
        })
    }
    pub fn set_custom_bus_effects(
        &self,
        id: CustomBusId,
        effects: Vec<AudioEffect>,
        fade_seconds: f32,
    ) -> Result<(), String> {
        validate_effect_chain(&effects)?;
        validate_transition_seconds(fade_seconds)?;
        self.enqueue(Command::CustomBusEffects {
            id,
            effects,
            fade_seconds,
        })
    }
    pub fn set_custom_bus_send_gain(
        &self,
        bus: CustomBusId,
        send: AudioSendId,
        gain: f32,
        fade_seconds: f32,
    ) -> Result<(), String> {
        if !gain.is_finite() || !(0.0..=2.0).contains(&gain) {
            return Err("send route gain must be finite and within [0, 2]".into());
        }
        validate_transition_seconds(fade_seconds)?;
        self.enqueue(Command::CustomBusSendGain {
            bus,
            send,
            gain,
            fade_seconds,
        })
    }
    pub fn stop_custom_bus(&self, id: CustomBusId, fade_seconds: f32) -> Result<(), String> {
        validate_transition_seconds(fade_seconds)?;
        self.enqueue(Command::StopCustomBus { id, fade_seconds })
    }
    pub fn pause_custom_bus(&self, id: CustomBusId) -> Result<(), String> {
        self.enqueue(Command::PauseCustomBus(id))
    }
    pub fn resume_custom_bus(&self, id: CustomBusId) -> Result<(), String> {
        self.enqueue(Command::ResumeCustomBus(id))
    }

    /// Submit one bounded automation frame as a single queue message. This guarantees all
    /// commands in the frame are either accepted together or rejected together when the queue is
    /// saturated. The backend still reports individual stale-voice failures asynchronously.
    pub fn apply_automation_frame(
        &self,
        commands: Vec<AutomationCommand>,
        transition_seconds: f32,
    ) -> Result<(), String> {
        if commands.len() > MAX_AUTOMATION_COMMANDS {
            return Err(format!(
                "automation frames are limited to {MAX_AUTOMATION_COMMANDS} commands"
            ));
        }
        if !transition_seconds.is_finite() || !(0.0..=60.0).contains(&transition_seconds) {
            return Err("automation transition must be finite and within [0, 60] seconds".into());
        }
        for command in &commands {
            command.validate()?;
        }
        self.enqueue(Command::AutomationFrame {
            commands,
            transition_seconds,
        })
    }
    /// Smoothly tween all gains/effects included in a snapshot. Omitted buses are left unchanged.
    pub fn transition_to_snapshot(
        &self,
        snapshot: &MixerSnapshot,
        transition_seconds: f32,
    ) -> Result<(), String> {
        validate_transition_seconds(transition_seconds)?;
        snapshot.validate()?;
        let mut gains: Vec<_> = snapshot
            .gains
            .iter()
            .map(|(bus, gain)| (*bus, *gain))
            .collect();
        gains.sort_by_key(|(bus, _)| bus_order(*bus));
        let mut mutes: Vec<_> = snapshot
            .mutes
            .iter()
            .map(|(bus, muted)| (*bus, *muted))
            .collect();
        mutes.sort_by_key(|(bus, _)| bus_order(*bus));
        let mut zone_gains: Vec<_> = snapshot
            .zone_gains
            .iter()
            .map(|(bus, gain)| (*bus, *gain))
            .collect();
        zone_gains.sort_by_key(|(bus, _)| bus_order(*bus));
        let mut effects: Vec<_> = snapshot
            .effects
            .iter()
            .map(|(bus, effects)| (*bus, effects.clone()))
            .collect();
        effects.sort_by_key(|(bus, _)| bus_order(*bus));
        self.enqueue(Command::Snapshot {
            gains,
            mutes,
            zone_gains,
            effects,
            transition_seconds,
        })
    }

    /// Activate a registered named mixer profile. The snapshot itself remains backend-independent.
    pub fn transition_to_named_snapshot(
        &self,
        registry: &AudioSnapshotRegistry,
        name: &str,
        transition_seconds: f32,
    ) -> Result<(), String> {
        let snapshot = registry
            .get(name)
            .ok_or_else(|| format!("unknown audio snapshot `{name}`"))?;
        self.transition_to_snapshot(snapshot, transition_seconds)
    }

    /// Start/update an ambient layer independent of the legacy single-selected-ambience API.
    pub fn set_ambience_layer(
        &self,
        id: AmbienceId,
        gain: f32,
        fade_seconds: f32,
    ) -> Result<(), String> {
        self.enqueue(Command::AmbienceLayer {
            id,
            gain,
            fade_seconds,
        })
    }

    /// Update the listener-local HRTF direction of a voice created with `binaural_direction`.
    #[cfg(feature = "steam-audio")]
    pub fn set_voice_binaural_direction(
        &self,
        voice: VoiceId,
        direction: [f32; 3],
    ) -> Result<(), String> {
        if direction.iter().any(|value| !value.is_finite())
            || direction.iter().map(|value| value * value).sum::<f32>() <= f32::EPSILON
        {
            return Err("binaural direction must be finite and non-zero".into());
        }
        self.enqueue(Command::BinauralDirection { voice, direction })
    }

    /// Update a moving 3D voice with a material-aware transmission response.
    pub fn update_voice_spatial_acoustics(
        &self,
        voice: VoiceId,
        acoustics: ManagedEmitterAcoustics,
    ) -> Result<(), String> {
        if !acoustics.gain.is_finite()
            || !acoustics.pan.is_finite()
            || !acoustics.playback_rate.is_finite()
            || !acoustics.occlusion.is_finite()
            || !acoustics.transmission_gain.is_finite()
            || !acoustics.low_pass_cutoff_hz.is_finite()
        {
            return Err("spatial acoustic parameters must be finite".into());
        }
        self.enqueue(Command::SpatialUpdate {
            voice,
            acoustics: ManagedEmitterAcoustics {
                gain: acoustics.gain.clamp(0.0, 4.0),
                pan: sanitize_pan(acoustics.pan),
                playback_rate: sanitize_doppler(acoustics.playback_rate),
                occlusion: acoustics.occlusion.clamp(0.0, 1.0),
                transmission_gain: acoustics.transmission_gain.clamp(0.0, 1.0),
                low_pass_cutoff_hz: acoustics.low_pass_cutoff_hz.clamp(20.0, 20_000.0),
                #[cfg(feature = "steam-audio")]
                binaural_direction: acoustics.binaural_direction.filter(|direction| {
                    direction.iter().all(|value| value.is_finite())
                        && direction.iter().map(|value| value * value).sum::<f32>() > f32::EPSILON
                }),
            },
        })
    }
    pub fn stop_ambience_layer(&self, id: AmbienceId, fade_seconds: f32) -> Result<(), String> {
        self.enqueue(Command::StopAmbienceLayer { id, fade_seconds })
    }
    fn reserve_stream_bytes(&self, size: u64) -> Result<(), String> {
        let mut current = self.pending_stream_bytes.load(Ordering::Acquire);
        loop {
            let next = current
                .checked_add(size)
                .ok_or_else(|| "pending streaming byte budget overflow".to_owned())?;
            if next > MAX_PENDING_STREAM_BYTES {
                return Err("pending streaming command byte budget is full; use play_streaming_file for long tracks".into());
            }
            match self.pending_stream_bytes.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Ok(()),
                Err(observed) => current = observed,
            }
        }
    }
}

impl AudioSink for ThreadedAudioSink {
    fn supports_managed_voices(&self) -> bool {
        true
    }

    fn play(&mut self, id: SoundId, gain: f32, pan: f32) {
        let options = PlaybackOptions {
            gain,
            pan: sanitize_pan(pan),
            ..PlaybackOptions::default()
        };
        if let Err(error) = self.play_voice(id, options) {
            self.record_facade_failure(&error);
        }
    }
    fn play_spatial_advanced(&mut self, id: SoundId, gain: f32, pan: f32, doppler_ratio: f32) {
        if !gain.is_finite() || gain <= 0.0 {
            return;
        }
        let voice = match self.allocate_voice() {
            Ok(voice) => voice,
            Err(error) => {
                self.record_facade_failure(&error);
                return;
            }
        };
        let options = PlaybackOptions {
            gain,
            pan: sanitize_pan(pan),
            ..PlaybackOptions::default()
        };
        if let Err(error) = self.enqueue(Command::Play {
            voice,
            sound: id,
            options,
            doppler: sanitize_doppler(doppler_ratio),
        }) {
            self.record_facade_failure(&error);
        }
    }
    fn set_ambience(&mut self, ambience: Option<AmbienceId>, gain: f32) {
        let gain = if gain.is_finite() {
            gain.clamp(0.0, 4.0)
        } else {
            0.0
        };
        if let Err(error) = self.enqueue(Command::Ambience {
            id: if gain > 0.0 { ambience } else { None },
            gain,
        }) {
            self.record_facade_failure(&error);
        }
    }

    fn take_completed_voices(&mut self) -> VoiceCompletions {
        self.poll_completed_voices()
    }

    fn play_managed_emitter(&mut self, playback: crate::ManagedEmitterPlayback) -> Option<VoiceId> {
        let options = PlaybackOptions {
            gain: playback.gain,
            pan: sanitize_pan(playback.pan),
            playback_rate: if playback.playback_rate.is_finite() {
                playback.playback_rate.clamp(0.25, 4.0)
            } else {
                1.0
            },
            looping: playback.looping,
            priority: playback.priority,
            bus: playback.bus,
            #[cfg(feature = "steam-audio")]
            binaural_direction: playback.binaural_direction,
            ..PlaybackOptions::default()
        };
        match self.play_voice(playback.sound, options) {
            Ok(voice) => Some(voice),
            Err(error) => {
                self.record_facade_failure(&error);
                None
            }
        }
    }

    fn update_managed_emitter(
        &mut self,
        voice: VoiceId,
        gain: f32,
        pan: f32,
        playback_rate: f32,
        occlusion: f32,
    ) {
        let occlusion = if occlusion.is_finite() {
            occlusion.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let cutoff = 20_000.0 * (1.0 - occlusion) + 700.0 * occlusion;
        if let Err(error) = self.update_voice_spatial_acoustics(
            voice,
            ManagedEmitterAcoustics {
                gain: if gain.is_finite() { gain } else { 0.0 },
                pan,
                playback_rate,
                occlusion,
                transmission_gain: 1.0,
                low_pass_cutoff_hz: cutoff,
                #[cfg(feature = "steam-audio")]
                binaural_direction: None,
            },
        ) {
            self.record_facade_failure(&error);
        }
    }

    fn update_managed_emitter_acoustics(
        &mut self,
        voice: VoiceId,
        acoustics: ManagedEmitterAcoustics,
    ) {
        if let Err(error) = self.update_voice_spatial_acoustics(voice, acoustics) {
            self.record_facade_failure(&error);
        }
    }

    fn stop_managed_emitter(&mut self, voice: VoiceId, fade_seconds: f32) {
        if let Err(error) = self.stop_voice(voice, fade_seconds) {
            self.record_facade_failure(&error);
        }
    }

    fn set_bus_zone_gain(&mut self, bus: AudioBus, gain: f32, fade_seconds: f32) {
        let _ = self.try_set_bus_zone_gain(bus, gain, fade_seconds);
    }

    fn try_set_bus_zone_gain(
        &mut self,
        bus: AudioBus,
        gain: f32,
        fade_seconds: f32,
    ) -> Result<(), String> {
        let result = self.enqueue(Command::BusZoneGain {
            bus,
            gain,
            fade_seconds,
        });
        if let Err(error) = &result {
            self.record_facade_failure(error);
        }
        result
    }

    fn set_bus_effects(&mut self, bus: AudioBus, effects: &[AudioEffect]) {
        let _ = self.try_set_bus_effects(bus, effects);
    }

    fn try_set_bus_effects(
        &mut self,
        bus: AudioBus,
        effects: &[AudioEffect],
    ) -> Result<(), String> {
        validate_effect_chain(effects)?;
        let result = self.enqueue(Command::Effects {
            bus,
            effects: effects.to_vec(),
            transition_seconds: 0.12,
        });
        if let Err(error) = &result {
            self.record_facade_failure(error);
        }
        result
    }
}

impl Drop for ThreadedAudioSink {
    fn drop(&mut self) {
        // Dropping the final sender closes the queue. The worker exits and drops Kira on its owner thread.
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            if worker.thread().id() != thread::current().id() {
                let _ = worker.join();
            }
        }
    }
}

fn worker_main(
    bank: SoundBank,
    receiver: Receiver<Command>,
    ready: SyncSender<Result<(), String>>,
    failures: Arc<AtomicU64>,
    last_error: Arc<Mutex<Option<String>>>,
    pending_stream_bytes: Arc<AtomicU64>,
) {
    let mut backend = match KiraAudioBackend::new(bank) {
        Ok(backend) => backend,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    if ready.send(Ok(())).is_err() {
        return;
    }
    let mut recovery_pending = false;
    let mut recovery_attempts = 0u64;
    let mut successful_recoveries = 0u64;
    let mut last_device_error: Option<String> = None;
    let mut retry_at = Instant::now();
    let mut retry_delay = Duration::from_secs(1);
    loop {
        let command = match receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(command) => Some(command),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        if let Some(command) = command {
            let result: Result<(), String> = match command {
                Command::Play {
                    voice,
                    sound,
                    options,
                    doppler,
                } => {
                    let result = backend.play_voice(voice, sound, options, doppler);
                    if result.is_err() {
                        backend.report_voice_start_failed(voice);
                    }
                    result
                }
                Command::StreamFile {
                    voice,
                    path,
                    options,
                } => {
                    let result = backend.play_streaming_file(voice, path, options);
                    if result.is_err() {
                        backend.report_voice_start_failed(voice);
                    }
                    result
                }
                Command::StreamBytes {
                    voice,
                    bytes,
                    options,
                } => {
                    pending_stream_bytes.fetch_sub(bytes.len() as u64, Ordering::AcqRel);
                    let result = backend.play_streaming_bytes(voice, bytes, options);
                    if result.is_err() {
                        backend.report_voice_start_failed(voice);
                    }
                    result
                }
                Command::Stop {
                    voice,
                    fade_seconds,
                } => {
                    backend.stop_voice(voice, fade_seconds);
                    Ok(())
                }
                Command::Pause { voice } => {
                    if backend.pause_voice(voice) {
                        Ok(())
                    } else {
                        Err(format!("cannot pause unknown or stale voice {:?}", voice))
                    }
                }
                Command::Resume { voice } => {
                    if backend.resume_voice(voice) {
                        Ok(())
                    } else {
                        Err(format!("cannot resume unknown or stale voice {:?}", voice))
                    }
                }
                Command::Gain {
                    voice,
                    gain,
                    fade_seconds,
                } => {
                    if backend.set_voice_gain(voice, gain, fade_seconds) {
                        Ok(())
                    } else {
                        Err(format!(
                            "cannot set gain for unknown or stale voice {:?}",
                            voice
                        ))
                    }
                }
                Command::Rate {
                    voice,
                    rate,
                    fade_seconds,
                } => {
                    if backend.set_voice_rate(voice, rate, fade_seconds) {
                        Ok(())
                    } else {
                        Err(format!(
                            "cannot set playback rate for unknown or stale voice {:?}",
                            voice
                        ))
                    }
                }
                Command::Pan {
                    voice,
                    pan,
                    fade_seconds,
                } => {
                    if backend.set_voice_pan(voice, pan, fade_seconds) {
                        Ok(())
                    } else {
                        Err(format!(
                            "cannot set pan for unknown or stale voice {:?}",
                            voice
                        ))
                    }
                }
                Command::Looping { voice, looping } => {
                    if backend.set_voice_looping(voice, looping) {
                        Ok(())
                    } else {
                        Err(format!(
                            "cannot change looping for unknown or stale voice {:?}",
                            voice
                        ))
                    }
                }
                Command::StopBus { bus, fade_seconds } => {
                    backend.stop_bus(bus, fade_seconds);
                    Ok(())
                }
                Command::PauseBus { bus } => {
                    backend.pause_bus(bus);
                    Ok(())
                }
                Command::ResumeBus { bus } => {
                    backend.resume_bus(bus);
                    Ok(())
                }
                Command::StopAll { fade_seconds } => {
                    backend.stop_all_voices(fade_seconds);
                    Ok(())
                }
                Command::Ambience { id, gain } => backend.set_ambience_checked(id, gain),
                Command::AmbienceLayer {
                    id,
                    gain,
                    fade_seconds,
                } => backend.set_ambience_layer(id, gain, fade_seconds),
                Command::StopAmbienceLayer { id, fade_seconds } => {
                    backend.stop_ambience_layer(id, fade_seconds);
                    Ok(())
                }
                Command::BusGain {
                    bus,
                    gain,
                    fade_seconds,
                } => {
                    backend.set_bus_gain(bus, gain, fade_seconds);
                    Ok(())
                }
                Command::BusMute {
                    bus,
                    muted,
                    fade_seconds,
                } => {
                    backend.set_bus_muted(bus, muted, fade_seconds);
                    Ok(())
                }
                Command::BusZoneGain {
                    bus,
                    gain,
                    fade_seconds,
                } => {
                    backend.set_bus_zone_gain(bus, gain, fade_seconds);
                    Ok(())
                }
                Command::Effects {
                    bus,
                    effects,
                    transition_seconds,
                } => {
                    backend.set_bus_effects_faded(bus, &effects, transition_seconds);
                    Ok(())
                }
                Command::Snapshot {
                    gains,
                    mutes,
                    zone_gains,
                    effects,
                    transition_seconds,
                } => {
                    for (bus, gain) in gains {
                        backend.set_bus_gain(bus, gain, transition_seconds);
                    }
                    for (bus, muted) in mutes {
                        backend.set_bus_muted(bus, muted, transition_seconds);
                    }
                    for (bus, gain) in zone_gains {
                        backend.set_bus_zone_gain(bus, gain, transition_seconds);
                    }
                    for (bus, chain) in effects {
                        backend.set_bus_effects_faded(bus, &chain, transition_seconds);
                    }
                    Ok(())
                }
                Command::AutomationFrame {
                    commands,
                    transition_seconds,
                } => {
                    let mut first_error = None;
                    for command in commands {
                        let result = match command {
                            AutomationCommand::SetBusGain { bus, gain } => {
                                backend.set_bus_gain(bus, gain, transition_seconds);
                                Ok(())
                            }
                            AutomationCommand::SetVoiceGain { voice, gain } => {
                                if backend.set_voice_gain(voice, gain, transition_seconds) {
                                    Ok(())
                                } else {
                                    Err(format!("automation references stale voice {voice:?}"))
                                }
                            }
                            AutomationCommand::SetVoicePlaybackRate { voice, rate } => {
                                if backend.set_voice_rate(voice, rate, transition_seconds) {
                                    Ok(())
                                } else {
                                    Err(format!("automation references stale voice {voice:?}"))
                                }
                            }
                            AutomationCommand::SetVoicePan { voice, pan } => {
                                if backend.set_voice_pan(voice, pan, transition_seconds) {
                                    Ok(())
                                } else {
                                    Err(format!("automation references stale voice {voice:?}"))
                                }
                            }
                            AutomationCommand::SetBusEffects { bus, effects } => {
                                backend.set_bus_effects_faded(bus, &effects, transition_seconds);
                                Ok(())
                            }
                        };
                        if first_error.is_none() {
                            first_error = result.err();
                        }
                    }
                    first_error.map_or(Ok(()), Err)
                }
                Command::VoiceState { voice, reply } => {
                    let _ = reply.send(backend.voice_state(voice));
                    Ok(())
                }
                Command::CompletedVoices { reply } => {
                    let (voices, dropped_notifications_total, active_voices) =
                        backend.take_completed_voices();
                    let _ = reply.send(VoiceCompletions {
                        voices,
                        dropped_notifications_total,
                        active_voices: Some(active_voices),
                    });
                    Ok(())
                }
                Command::Diagnostics { reply } => {
                    let _ = reply.send(backend.diagnostics());
                    Ok(())
                }
                Command::SpatialUpdate { voice, acoustics } => {
                    if backend.update_voice_spatial_acoustics(voice, acoustics) {
                        Ok(())
                    } else {
                        Err(format!(
                            "cannot update spatial state for unknown or stale voice {:?}",
                            voice
                        ))
                    }
                }
                #[cfg(feature = "steam-audio")]
                Command::BinauralDirection { voice, direction } => {
                    if backend.set_voice_binaural_direction(voice, direction) {
                        Ok(())
                    } else {
                        Err(format!(
                            "voice {:?} does not have an active Steam Audio HRTF",
                            voice
                        ))
                    }
                }
                Command::CreateSend(config) => backend.create_audio_send(config),
                Command::RemoveSend(id) => backend.remove_audio_send(id),
                Command::SendGain {
                    id,
                    gain,
                    fade_seconds,
                } => backend.set_audio_send_gain(id, gain, fade_seconds),
                Command::CreateCustomBus(config) => backend.create_custom_bus(config),
                Command::RemoveCustomBus(id) => backend.remove_custom_bus(id),
                Command::CustomBusGain {
                    id,
                    gain,
                    fade_seconds,
                } => backend.set_custom_bus_gain(id, gain, fade_seconds),
                Command::CustomBusMute {
                    id,
                    muted,
                    fade_seconds,
                } => backend.set_custom_bus_muted(id, muted, fade_seconds),
                Command::CustomBusEffects {
                    id,
                    effects,
                    fade_seconds,
                } => backend.set_custom_bus_effects(id, &effects, fade_seconds),
                Command::CustomBusSendGain {
                    bus,
                    send,
                    gain,
                    fade_seconds,
                } => backend.set_custom_bus_send_gain(bus, send, gain, fade_seconds),
                Command::StopCustomBus { id, fade_seconds } => {
                    backend.stop_custom_bus(id, fade_seconds);
                    Ok(())
                }
                Command::PauseCustomBus(id) => {
                    if backend.pause_custom_bus(id) {
                        Ok(())
                    } else {
                        Err(format!("unknown custom bus {:?}", id))
                    }
                }
                Command::ResumeCustomBus(id) => {
                    if backend.resume_custom_bus(id) {
                        Ok(())
                    } else {
                        Err(format!("unknown custom bus {:?}", id))
                    }
                }
            };
            if let Err(error) = result {
                failures.fetch_add(1, Ordering::Relaxed);
                if let Ok(mut stored) = last_error.lock() {
                    if stored.as_deref() != Some(error.as_str()) {
                        *stored = Some(error);
                    }
                }
            }
        }

        if let Some(error) = backend.take_device_error() {
            last_device_error = Some(error);
            // A backend may report multiple queued errors while disconnected. Only the first
            // error starts the recovery cycle; later reports must not reset the backoff timer or
            // turn a persistent device failure into a tight reopen loop.
            if !recovery_pending {
                recovery_pending = true;
                retry_at = Instant::now();
                retry_delay = Duration::from_secs(1);
            }
        }
        if recovery_pending && Instant::now() >= retry_at {
            recovery_attempts = recovery_attempts.saturating_add(1);
            backend.set_device_recovery_state(
                recovery_attempts,
                successful_recoveries,
                last_device_error.clone(),
            );
            match backend.recover_output_device() {
                Ok(()) => {
                    successful_recoveries = successful_recoveries.saturating_add(1);
                    recovery_pending = false;
                    last_device_error = None;
                    retry_delay = Duration::from_secs(1);
                    if let Ok(mut stored) = last_error.lock() {
                        if stored.as_deref().is_some_and(|message| {
                            message.starts_with("audio device recovery failed:")
                        }) {
                            *stored = None;
                        }
                    }
                }
                Err(error) => {
                    last_device_error = Some(format!("audio device recovery failed: {error}"));
                    retry_at = Instant::now() + retry_delay;
                    retry_delay = next_device_recovery_delay(retry_delay);
                    failures.fetch_add(1, Ordering::Relaxed);
                    if let Ok(mut stored) = last_error.lock() {
                        *stored = last_device_error.clone();
                    }
                }
            }
            backend.set_device_recovery_state(
                recovery_attempts,
                successful_recoveries,
                last_device_error.clone(),
            );
        }
    }
    backend.stop_all_voices(0.05);
}

fn bus_order(bus: AudioBus) -> u8 {
    match bus {
        AudioBus::Master => 0,
        AudioBus::Music => 1,
        AudioBus::Sfx => 2,
        AudioBus::Ambience => 3,
        AudioBus::Dialogue => 4,
        AudioBus::Voice => 5,
        AudioBus::Ui => 6,
    }
}

fn next_device_recovery_delay(current: Duration) -> Duration {
    current
        .checked_mul(2)
        .unwrap_or(Duration::from_secs(30))
        .min(Duration::from_secs(30))
}

fn validate_transition_seconds(seconds: f32) -> Result<(), String> {
    if seconds.is_finite() && (0.0..=60.0).contains(&seconds) {
        Ok(())
    } else {
        Err("audio transition duration must be finite and within [0, 60] seconds".into())
    }
}

fn allocate_voice_id(counter: &AtomicU64) -> Result<VoiceId, String> {
    // `fetch_add` would wrap at u64::MAX and could eventually alias a still-live voice.
    // `try_update` leaves the counter unchanged on exhaustion and never returns ID zero.
    counter
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
            if next == 0 {
                None
            } else {
                next.checked_add(1)
            }
        })
        .map(VoiceId)
        .map_err(|_| "audio voice identifier space exhausted".to_owned())
}

fn sanitize_pan(pan: f32) -> f32 {
    if pan.is_finite() {
        pan.clamp(-1.0, 1.0)
    } else {
        0.0
    }
}
fn sanitize_doppler(ratio: f32) -> f32 {
    if ratio.is_finite() {
        ratio.clamp(0.5, 2.0)
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_queue_has_fixed_capacity_and_ids_are_stable() {
        // Observe actual backpressure instead of asserting only a constant.
        let (sender, _receiver) = mpsc::sync_channel(COMMAND_CAPACITY);
        for _ in 0..COMMAND_CAPACITY {
            sender
                .try_send(Command::StopAll { fade_seconds: 0.0 })
                .expect("queue has free capacity");
        }
        assert!(matches!(
            sender.try_send(Command::StopAll { fade_seconds: 0.0 }),
            Err(TrySendError::Full(_))
        ));
        let one = VoiceId(7);
        let two = VoiceId(8);
        assert_ne!(one, two);
        assert_eq!(sanitize_pan(5.0), 1.0);
        assert_eq!(sanitize_pan(f32::NAN), 0.0);
        assert_eq!(sanitize_doppler(9.0), 2.0);
    }

    #[test]
    fn device_recovery_backoff_doubles_and_caps_at_thirty_seconds() {
        let mut delay = Duration::from_secs(1);
        let mut observed = Vec::new();
        for _ in 0..8 {
            observed.push(delay.as_secs());
            delay = next_device_recovery_delay(delay);
        }
        assert_eq!(observed, vec![1, 2, 4, 8, 16, 30, 30, 30]);
    }

    #[test]
    fn voice_id_allocator_never_wraps_or_reuses_zero() {
        let counter = AtomicU64::new(1);
        assert_eq!(allocate_voice_id(&counter).unwrap(), VoiceId(1));
        assert_eq!(allocate_voice_id(&counter).unwrap(), VoiceId(2));

        let exhausted = AtomicU64::new(u64::MAX);
        assert!(allocate_voice_id(&exhausted).is_err());
        assert_eq!(exhausted.load(Ordering::Relaxed), u64::MAX);
        let invalid = AtomicU64::new(0);
        assert!(allocate_voice_id(&invalid).is_err());
        assert_eq!(invalid.load(Ordering::Relaxed), 0);
    }
}
