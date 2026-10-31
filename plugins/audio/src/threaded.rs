//! Bounded, non-blocking command bridge to the Kira backend.
//!
//! All `AudioManager`, sound handles, decoder and mixer-track operations stay on the named
//! worker thread. Gameplay receives a stable `VoiceId` immediately after a command is accepted
//! into the bounded queue; backend rejection is exposed by `backend_failures()`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use crate::bank::{AmbienceId, SoundBank, SoundId};
use crate::debug::AudioDiagnostics;
use crate::kira_backend::{KiraAudioBackend, PlaybackOptions};
use crate::mixer::{AudioEffect, MixerSnapshot};
use crate::registry::{AudioBus, ResolvedAudioEvent};
use crate::voices::{PlaybackStatus, VoiceId};
use crate::AudioSink;

const COMMAND_CAPACITY: usize = 256;
const MAX_PENDING_STREAM_BYTES: u64 = 64 * 1024 * 1024;

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
        effects: Vec<(AudioBus, Vec<AudioEffect>)>,
        transition_seconds: f32,
    },
    VoiceState {
        voice: VoiceId,
        reply: SyncSender<Option<PlaybackStatus>>,
    },
    Diagnostics {
        reply: SyncSender<AudioDiagnostics>,
    },
    SpatialUpdate {
        voice: VoiceId,
        gain: f32,
        pan: f32,
        rate: f32,
        occlusion: f32,
    },
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

    fn allocate_voice(&self) -> VoiceId {
        let id = self.next_voice.fetch_add(1, Ordering::Relaxed);
        VoiceId(id.max(1))
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
        let voice = self.allocate_voice();
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
        let voice = self.allocate_voice();
        self.enqueue(Command::StreamFile {
            voice,
            path: path.as_ref().to_path_buf(),
            options,
        })?;
        Ok(voice)
    }

    /// Play an encoded byte source through Kira's streaming decoder (encoded bytes are retained;
    /// decoded PCM is produced in chunks). Useful for packaged long tracks.
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
        let voice = self.allocate_voice();
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
    /// Request owner-thread metrics without forcing Kira handles to cross threads.
    pub fn request_diagnostics(&self) -> Result<Receiver<AudioDiagnostics>, String> {
        let (reply, result) = mpsc::sync_channel(1);
        self.enqueue(Command::Diagnostics { reply })?;
        Ok(result)
    }
    pub fn set_bus_gain(&self, bus: AudioBus, gain: f32, fade_seconds: f32) -> Result<(), String> {
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
        self.enqueue(Command::BusZoneGain {
            bus,
            gain,
            fade_seconds,
        })
    }
    pub fn set_bus_effects(&self, bus: AudioBus, effects: Vec<AudioEffect>) -> Result<(), String> {
        self.enqueue(Command::Effects {
            bus,
            effects,
            transition_seconds: 0.08,
        })
    }
    /// Smoothly tween all gains/effects included in a snapshot. Omitted buses are left unchanged.
    pub fn transition_to_snapshot(
        &self,
        snapshot: &MixerSnapshot,
        transition_seconds: f32,
    ) -> Result<(), String> {
        let mut gains: Vec<_> = snapshot
            .gains
            .iter()
            .map(|(bus, gain)| (*bus, *gain))
            .collect();
        gains.sort_by_key(|(bus, _)| bus_order(*bus));
        let mut effects: Vec<_> = snapshot
            .effects
            .iter()
            .map(|(bus, effects)| (*bus, effects.clone()))
            .collect();
        effects.sort_by_key(|(bus, _)| bus_order(*bus));
        self.enqueue(Command::Snapshot {
            gains,
            effects,
            transition_seconds,
        })
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
        let voice = self.allocate_voice();
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
        let command = Command::SpatialUpdate {
            voice,
            gain: if gain.is_finite() {
                gain.clamp(0.0, 4.0)
            } else {
                0.0
            },
            pan: sanitize_pan(pan),
            rate: sanitize_doppler(playback_rate),
            occlusion: if occlusion.is_finite() {
                occlusion.clamp(0.0, 1.0)
            } else {
                0.0
            },
        };
        if let Err(error) = self.enqueue(command) {
            self.record_facade_failure(&error);
        }
    }

    fn stop_managed_emitter(&mut self, voice: VoiceId, fade_seconds: f32) {
        if let Err(error) = self.stop_voice(voice, fade_seconds) {
            self.record_facade_failure(&error);
        }
    }

    fn set_bus_effects(&mut self, bus: AudioBus, effects: &[AudioEffect]) {
        if let Err(error) = self.enqueue(Command::Effects {
            bus,
            effects: effects.to_vec(),
            transition_seconds: 0.12,
        }) {
            self.record_facade_failure(&error);
        }
    }

    fn set_bus_zone_gain(&mut self, bus: AudioBus, gain: f32, fade_seconds: f32) {
        if let Err(error) = self.enqueue(Command::BusZoneGain {
            bus,
            gain,
            fade_seconds,
        }) {
            self.record_facade_failure(&error);
        }
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
    while let Ok(command) = receiver.recv() {
        let result: Result<(), String> = match command {
            Command::Play {
                voice,
                sound,
                options,
                doppler,
            } => backend.play_voice(voice, sound, options, doppler),
            Command::StreamFile {
                voice,
                path,
                options,
            } => backend.play_streaming_file(voice, path, options),
            Command::StreamBytes {
                voice,
                bytes,
                options,
            } => {
                pending_stream_bytes.fetch_sub(bytes.len() as u64, Ordering::AcqRel);
                backend.play_streaming_bytes(voice, bytes, options)
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
                effects,
                transition_seconds,
            } => {
                for (bus, gain) in gains {
                    backend.set_bus_gain(bus, gain, transition_seconds);
                }
                for (bus, chain) in effects {
                    backend.set_bus_effects_faded(bus, &chain, transition_seconds);
                }
                Ok(())
            }
            Command::VoiceState { voice, reply } => {
                let _ = reply.send(backend.voice_state(voice));
                Ok(())
            }
            Command::Diagnostics { reply } => {
                let _ = reply.send(backend.diagnostics());
                Ok(())
            }
            Command::SpatialUpdate {
                voice,
                gain,
                pan,
                rate,
                occlusion,
            } => {
                if backend.update_voice_spatial(voice, gain, pan, rate, occlusion) {
                    Ok(())
                } else {
                    Err(format!(
                        "cannot update spatial state for unknown or stale voice {:?}",
                        voice
                    ))
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
}
