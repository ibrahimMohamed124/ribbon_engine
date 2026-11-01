//! Kira-backed playback, mixer buses, voice lifecycle, and streaming.
//!
//! The backend is deliberately owner-thread-only. `threaded::ThreadedAudioSink` runs it on
//! a dedicated worker and exposes a bounded command channel to gameplay/ECS systems.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Cursor;
use std::path::Path;
use std::time::Duration;

use kira::effect::compressor::{CompressorBuilder, CompressorHandle};
use kira::effect::filter::{FilterBuilder, FilterHandle, FilterMode};
use kira::effect::reverb::{ReverbBuilder, ReverbHandle};
use kira::sound::static_sound::{StaticSoundData, StaticSoundHandle};
use kira::sound::streaming::{StreamingSoundData, StreamingSoundHandle};
use kira::sound::FromFileError;
use kira::sound::PlaybackState;
use kira::track::{TrackBuilder, TrackHandle};
use kira::{AudioManager, AudioManagerSettings, DefaultBackend, Mix, Tween};

use crate::bank::{AmbienceId, SoundBank, SoundId};
use crate::debug::AudioDiagnostics;
use crate::mixer::{AudioBackend, AudioEffect};
use crate::registry::AudioBus;
use crate::voices::{PlaybackStatus, VoiceId};
use crate::ManagedEmitterAcoustics;

const MAX_VOICES: usize = 128;
const MAX_STATIC_SOUND_BYTES: usize = 16 * 1024 * 1024;
const MAX_STATIC_BANK_BYTES: usize = 64 * 1024 * 1024;
// Encoded size is not a useful proxy for decoded memory (highly compressed music can expand
// dramatically). These limits bound the PCM retained by the static sound cache.
const MAX_STATIC_SOUND_PCM_BYTES: usize = 64 * 1024 * 1024;
const MAX_STATIC_BANK_PCM_BYTES: usize = 128 * 1024 * 1024;
const DEFAULT_FADE_SECONDS: f32 = 0.12;
const SILENCE_DB: f32 = -100.0;
const MAX_ENCODED_STREAM_BYTES: usize = 64 * 1024 * 1024;
const MAX_TOTAL_SUB_TRACKS: usize = MAX_VOICES * 2 + 32;
const MAX_COMPLETED_VOICE_NOTIFICATIONS: usize = 256;

/// Settings for a single playback instance. Numeric values are sanitized at the boundary.
#[derive(Clone, Copy, Debug)]
pub struct PlaybackOptions {
    /// Linear gain multiplier. Values are clamped to `[0, 4]`.
    pub gain: f32,
    /// Playback rate/pitch multiplier, clamped to `[0.25, 4]`.
    pub playback_rate: f32,
    /// Stereo pan in `[-1, 1]`.
    pub pan: f32,
    /// When true, restart the whole sound when it reaches the end.
    pub looping: bool,
    /// Voice priority. Higher values may replace lower-priority voices at the voice limit.
    pub priority: u8,
    /// Destination mixer bus.
    pub bus: AudioBus,
    /// Fade-in length. Non-finite values are treated as zero.
    pub fade_in_seconds: f32,
}

impl Default for PlaybackOptions {
    fn default() -> Self {
        Self {
            gain: 1.0,
            playback_rate: 1.0,
            pan: 0.0,
            looping: false,
            priority: 128,
            bus: AudioBus::Sfx,
            fade_in_seconds: 0.0,
        }
    }
}

impl PlaybackOptions {
    fn sanitized(self) -> Self {
        Self {
            gain: clamp_finite(self.gain, 0.0, 4.0, 1.0),
            playback_rate: clamp_finite(self.playback_rate, 0.25, 4.0, 1.0),
            pan: clamp_finite(self.pan, -1.0, 1.0, 0.0),
            priority: self.priority,
            bus: self.bus,
            looping: self.looping,
            fade_in_seconds: clamp_finite(self.fade_in_seconds, 0.0, 60.0, 0.0),
        }
    }
}

/// A live voice. The handle is only ever touched by the Kira owner thread.
enum LiveHandle {
    Static(StaticSoundHandle),
    Streaming {
        handle: StreamingSoundHandle<FromFileError>,
        state: PlaybackState,
    },
}

impl LiveHandle {
    fn set_gain(&mut self, gain: f32, tween: Tween) {
        let db = gain_to_db(gain);
        match self {
            Self::Static(handle) => handle.set_volume(db, tween),
            Self::Streaming { handle, .. } => handle.set_volume(db, tween),
        }
    }
    fn set_rate(&mut self, rate: f32, tween: Tween) {
        match self {
            Self::Static(handle) => handle.set_playback_rate(rate as f64, tween),
            Self::Streaming { handle, .. } => handle.set_playback_rate(rate as f64, tween),
        }
    }
    fn set_pan(&mut self, pan: f32, tween: Tween) {
        match self {
            Self::Static(handle) => handle.set_panning(pan, tween),
            Self::Streaming { handle, .. } => handle.set_panning(pan, tween),
        }
    }
    fn set_looping(&mut self, looping: bool) {
        match self {
            Self::Static(handle) => {
                if looping {
                    handle.set_loop_region(0.0..);
                } else {
                    handle.set_loop_region(None);
                }
            }
            Self::Streaming { handle, .. } => {
                if looping {
                    handle.set_loop_region(0.0..);
                } else {
                    handle.set_loop_region(None);
                }
            }
        }
    }
    fn pause(&mut self, tween: Tween) {
        match self {
            Self::Static(handle) => handle.pause(tween),
            Self::Streaming { handle, state } => {
                handle.pause(tween);
                *state = if tween.duration.is_zero() {
                    PlaybackState::Paused
                } else {
                    PlaybackState::Pausing
                };
            }
        }
    }
    fn resume(&mut self, tween: Tween) {
        match self {
            Self::Static(handle) => handle.resume(tween),
            Self::Streaming { handle, state } => {
                handle.resume(tween);
                *state = if tween.duration.is_zero() {
                    PlaybackState::Playing
                } else {
                    PlaybackState::Resuming
                };
            }
        }
    }
    fn stop(&mut self, tween: Tween) {
        match self {
            Self::Static(handle) => handle.stop(tween),
            Self::Streaming { handle, state } => {
                handle.stop(tween);
                *state = if tween.duration.is_zero() {
                    PlaybackState::Stopped
                } else {
                    PlaybackState::Stopping
                };
            }
        }
    }
    fn state(&self, track: &TrackHandle) -> PlaybackState {
        match self {
            Self::Static(handle) => handle.state(),
            // Kira 0.12.5 does not expose playback state on streaming handles. A per-voice
            // track reports zero sounds after natural completion, while command-driven state is
            // tracked here. This also preserves fade-out lifetime until Kira removes the sound.
            Self::Streaming { .. } if track.num_sounds() == 0 => PlaybackState::Stopped,
            Self::Streaming { state, .. } => *state,
        }
    }
}

struct ActiveVoice {
    id: VoiceId,
    handle: LiveHandle,
    // Keeping this handle alive preserves the per-voice route and occlusion filter.
    _track: TrackHandle,
    occlusion_filter: FilterHandle,
    bus: AudioBus,
    priority: u8,
    sequence: u64,
    retained_stream_bytes: usize,
    is_streaming: bool,
}

struct BusTrack {
    track: TrackHandle,
    low_pass: FilterHandle,
    high_pass: FilterHandle,
    reverb: ReverbHandle,
    compressor: CompressorHandle,
    // Master-only peak-protection compressor. It remains in the route when snapshots change.
    _peak_protection: Option<CompressorHandle>,
    gain: f32,
    zone_gain: f32,
    muted: bool,
}

impl BusTrack {
    fn apply_effects(&mut self, effects: &[AudioEffect], transition_seconds: f32) {
        let tween = tween(transition_seconds);
        // Defaults are neutral. Each effect kind has its own handle so low- and high-pass
        // filters can coexist in one real Kira track, followed by the shared reverb stage.
        self.low_pass.set_cutoff(20_000.0_f64, tween);
        self.low_pass.set_mix(Mix(1.0), tween);
        self.high_pass.set_cutoff(20.0_f64, tween);
        self.high_pass.set_mix(Mix(0.0), tween);
        self.reverb.set_mix(Mix(0.0), tween);
        self.reverb.set_feedback(0.25_f64, tween);
        self.reverb.set_damping(0.25_f64, tween);
        self.compressor.set_threshold(-12.0_f64, tween);
        self.compressor.set_ratio(4.0_f64, tween);
        self.compressor.set_mix(Mix(0.0), tween);

        for effect in effects.iter().copied().map(AudioEffect::sanitized) {
            match effect {
                AudioEffect::LowPass { cutoff_hz } => {
                    self.low_pass.set_cutoff(cutoff_hz as f64, tween);
                    self.low_pass.set_mix(Mix(1.0), tween);
                }
                AudioEffect::HighPass { cutoff_hz } => {
                    self.high_pass.set_cutoff(cutoff_hz as f64, tween);
                    self.high_pass.set_mix(Mix(1.0), tween);
                }
                AudioEffect::Reverb { wet, decay_seconds } => {
                    self.reverb.set_mix(Mix(wet), tween);
                    // Kira feedback approaches infinite sustain at 1.0. Keep a finite margin.
                    self.reverb
                        .set_feedback((0.15 + decay_seconds as f64 * 0.078).min(0.93), tween);
                    self.reverb
                        .set_damping((1.0 - (decay_seconds as f64 / 12.0)).clamp(0.1, 1.0), tween);
                }
                AudioEffect::Compressor {
                    threshold_db,
                    ratio,
                } => {
                    self.compressor.set_threshold(threshold_db as f64, tween);
                    self.compressor.set_ratio(ratio as f64, tween);
                    self.compressor.set_mix(Mix(1.0), tween);
                }
            }
        }
    }

    fn set_gain(&mut self, gain: f32, duration: f32) {
        self.gain = clamp_finite(gain, 0.0, 2.0, 1.0);
        self.update_output_gain(duration);
    }

    fn set_muted(&mut self, muted: bool, duration: f32) {
        self.muted = muted;
        self.update_output_gain(duration);
    }

    fn set_zone_gain(&mut self, gain: f32, duration: f32) {
        self.zone_gain = clamp_finite(gain, 0.0, 2.0, 1.0);
        self.update_output_gain(duration);
    }

    fn update_output_gain(&mut self, duration: f32) {
        let effective = if self.muted {
            0.0
        } else {
            self.gain * self.zone_gain
        };
        self.track
            .set_volume(gain_to_db(effective), tween(duration));
    }
}

/// Kira implementation for use on one owner thread.
///
/// `StaticSoundData` is decoded once per unique embedded buffer and cheaply cloned for playback.
/// File/cursor streaming is decoded in chunks by Kira, without retaining a full decoded PCM clip.
pub struct KiraAudioBackend {
    // Owns the device/backend thread; retained for its Drop/lifetime semantics.
    _manager: AudioManager<DefaultBackend>,
    bank: SoundBank,
    decoded: HashMap<SoundId, StaticSoundData>,
    buses: HashMap<AudioBus, BusTrack>,
    voices: HashMap<VoiceId, ActiveVoice>,
    // Stolen voices retain their route until the stop tween finishes. This vector is capped.
    retiring_voices: Vec<ActiveVoice>,
    next_sequence: u64,
    next_internal_voice: u64,
    ambience_layers: HashMap<AmbienceId, VoiceId>,
    completed_voices: VecDeque<VoiceId>,
    dropped_completion_notifications: u64,
    decoded_static_pcm_bytes: usize,
    decode_failures: usize,
    last_decode_error: Option<String>,
}

impl KiraAudioBackend {
    /// Open the platform output device, build the mixer graph, and predecode short embedded clips.
    pub fn new(bank: SoundBank) -> Result<Self, String> {
        bank.validate()
            .map_err(|error| format!("invalid audio sound bank: {error}"))?;
        let mut manager_settings = AudioManagerSettings::default();
        // A voice owns one child track; allow the active voice budget, its bounded retiring
        // tail, and the six standard buses to coexist in Kira's global track arena.
        manager_settings.capacities.sub_track_capacity = MAX_TOTAL_SUB_TRACKS;
        let mut manager = AudioManager::<DefaultBackend>::new(manager_settings)
            .map_err(|error| format!("Kira output initialization failed: {error}"))?;
        // A real hierarchy: output -> master (with soft peak protection) -> child buses.
        let mut master = add_bus_track(&mut manager, true)?;
        let mut buses = HashMap::new();
        for bus in [
            AudioBus::Music,
            AudioBus::Ambience,
            AudioBus::Sfx,
            AudioBus::Ui,
            AudioBus::Dialogue,
            AudioBus::Voice,
        ] {
            let track = add_child_bus_track(&mut master.track)?;
            buses.insert(bus, track);
        }
        buses.insert(AudioBus::Master, master);

        let mut decoded = HashMap::new();
        // Content-keyed cache ensures different IDs referencing identical bytes share one decode.
        let mut unique: HashMap<&'static [u8], StaticSoundData> = HashMap::new();
        let mut encoded_bank_bytes = 0usize;
        let mut decoded_static_pcm_bytes = 0usize;
        let mut decode_failures = 0usize;
        let mut last_decode_error = None;
        for asset in bank.sounds() {
            let data = if let Some(data) = unique.get(asset.data) {
                data.clone()
            } else {
                if asset.data.len() > MAX_STATIC_SOUND_BYTES {
                    record_decode_failure(
                        &mut decode_failures,
                        &mut last_decode_error,
                        format!(
                            "sound {:?} is {} MiB; static clips are limited to {} MiB; use streaming for long assets",
                            asset.id,
                            asset.data.len() / (1024 * 1024),
                            MAX_STATIC_SOUND_BYTES / (1024 * 1024),
                        ),
                    );
                    continue;
                }
                let next_total = encoded_bank_bytes.saturating_add(asset.data.len());
                if next_total > MAX_STATIC_BANK_BYTES {
                    record_decode_failure(
                        &mut decode_failures,
                        &mut last_decode_error,
                        format!(
                            "sound {:?} skipped because the 64 MiB static encoded-data budget is full; use streaming",
                            asset.id
                        ),
                    );
                    continue;
                }
                match StaticSoundData::from_cursor(Cursor::new(asset.data.to_vec())) {
                    Ok(data) => {
                        let pcm_bytes = static_pcm_storage_bytes(&data);
                        let next_pcm_total = decoded_static_pcm_bytes.saturating_add(pcm_bytes);
                        if data.frames.is_empty()
                            || pcm_bytes > MAX_STATIC_SOUND_PCM_BYTES
                            || next_pcm_total > MAX_STATIC_BANK_PCM_BYTES
                        {
                            record_decode_failure(
                                &mut decode_failures,
                                &mut last_decode_error,
                                format!(
                                    "sound {:?} decoded to {} MiB PCM; static PCM limits are {} MiB per sound / {} MiB per bank; use streaming",
                                    asset.id,
                                    pcm_bytes / (1024 * 1024),
                                    MAX_STATIC_SOUND_PCM_BYTES / (1024 * 1024),
                                    MAX_STATIC_BANK_PCM_BYTES / (1024 * 1024),
                                ),
                            );
                            continue;
                        }
                        encoded_bank_bytes = next_total;
                        decoded_static_pcm_bytes = next_pcm_total;
                        unique.insert(asset.data, data.clone());
                        data
                    }
                    Err(error) => {
                        record_decode_failure(
                            &mut decode_failures,
                            &mut last_decode_error,
                            format!("failed to decode embedded sound {:?}: {error}", asset.id),
                        );
                        continue;
                    }
                }
            };
            decoded.insert(asset.id, data);
        }

        Ok(Self {
            _manager: manager,
            bank,
            decoded,
            buses,
            voices: HashMap::new(),
            retiring_voices: Vec::new(),
            next_sequence: 1,
            next_internal_voice: u64::MAX,
            ambience_layers: HashMap::new(),
            decoded_static_pcm_bytes,
            completed_voices: VecDeque::with_capacity(MAX_COMPLETED_VOICE_NOTIFICATIONS),
            dropped_completion_notifications: 0,
            decode_failures,
            last_decode_error,
        })
    }

    /// Play a registered sound and return the stable playback ID used for subsequent controls.
    pub fn play_voice(
        &mut self,
        id: VoiceId,
        sound: SoundId,
        options: PlaybackOptions,
        doppler: f32,
    ) -> Result<(), String> {
        let options = options.sanitized();
        if !self.decoded.contains_key(&sound) {
            return Err(format!("unknown or undecodable sound id {:?}", sound));
        }
        let asset = self
            .bank
            .sound(sound)
            .ok_or_else(|| format!("unknown sound id {:?}", sound))?;
        // Copy scalar metadata before mutably borrowing the backend for voice admission.
        let (asset_gain, asset_speed) = (asset.gain, asset.speed);
        let data = self
            .decoded
            .get(&sound)
            .cloned()
            .ok_or_else(|| format!("sound {:?} is registered but not decoded", sound))?;
        let victim = self.select_voice_victim(id, options.priority)?;
        let base_gain = clamp_finite(options.gain * asset_gain, 0.0, 4.0, 0.0);
        let rate = clamp_finite(
            options.playback_rate * asset_speed * doppler,
            0.25,
            4.0,
            1.0,
        );
        let route_bus = if self.buses.contains_key(&options.bus) {
            options.bus
        } else {
            AudioBus::Sfx
        };
        let bus = self
            .buses
            .get_mut(&route_bus)
            .ok_or_else(|| "Kira mixer bus is unavailable".to_owned())?;
        let (mut voice_track, occlusion_filter) = add_voice_track(&mut bus.track)?;
        let mut handle = voice_track
            .play(data)
            .map_err(|error| format!("Kira could not start sound {:?}: {error}", sound))?;
        if options.looping {
            handle.set_loop_region(0.0..);
        }
        handle.set_panning(options.pan, Tween::default());
        handle.set_playback_rate(rate as f64, Tween::default());
        if options.fade_in_seconds > 0.0 {
            handle.set_volume(SILENCE_DB, Tween::default());
            handle.set_volume(gain_to_db(base_gain), tween(options.fade_in_seconds));
        } else {
            handle.set_volume(gain_to_db(base_gain), Tween::default());
        }
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);
        self.voices.insert(
            id,
            ActiveVoice {
                id,
                handle: LiveHandle::Static(handle),
                _track: voice_track,
                occlusion_filter,
                bus: route_bus,
                priority: options.priority,
                sequence,
                retained_stream_bytes: 0,
                is_streaming: false,
            },
        );
        if let Some(victim) = victim {
            self.retire_voice(victim, DEFAULT_FADE_SECONDS);
        }
        Ok(())
    }

    /// Start a file-backed track using Kira's chunked streaming decoder.
    pub fn play_streaming_file(
        &mut self,
        id: VoiceId,
        path: impl AsRef<Path>,
        options: PlaybackOptions,
    ) -> Result<(), String> {
        let options = options.sanitized();
        let data = StreamingSoundData::from_file(path.as_ref()).map_err(|error| {
            format!(
                "could not open streaming audio `{}`: {error}",
                path.as_ref().display()
            )
        })?;
        let victim = self.select_voice_victim(id, options.priority)?;
        let route_bus = if self.buses.contains_key(&options.bus) {
            options.bus
        } else {
            AudioBus::Music
        };
        let bus = self
            .buses
            .get_mut(&route_bus)
            .ok_or_else(|| "Kira mixer bus is unavailable".to_owned())?;
        let (mut voice_track, occlusion_filter) = add_voice_track(&mut bus.track)?;
        let mut handle = voice_track
            .play(data)
            .map_err(|error| format!("Kira could not start streaming audio: {error}"))?;
        if options.looping {
            handle.set_loop_region(0.0..);
        }
        handle.set_volume(
            if options.fade_in_seconds > 0.0 {
                SILENCE_DB
            } else {
                gain_to_db(options.gain)
            },
            Tween::default(),
        );
        if options.fade_in_seconds > 0.0 {
            handle.set_volume(gain_to_db(options.gain), tween(options.fade_in_seconds));
        }
        handle.set_playback_rate(options.playback_rate as f64, Tween::default());
        handle.set_panning(options.pan, Tween::default());
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);
        self.voices.insert(
            id,
            ActiveVoice {
                id,
                handle: LiveHandle::Streaming {
                    handle,
                    state: PlaybackState::Playing,
                },
                _track: voice_track,
                occlusion_filter,
                bus: route_bus,
                priority: options.priority,
                sequence,
                retained_stream_bytes: 0,
                is_streaming: true,
            },
        );
        if let Some(victim) = victim {
            self.retire_voice(victim, DEFAULT_FADE_SECONDS);
        }
        Ok(())
    }

    /// Start a streaming track from encoded bytes. The byte cursor holds compressed source bytes;
    /// Kira decodes small chunks instead of retaining the entire decoded PCM track.
    pub fn play_streaming_bytes(
        &mut self,
        id: VoiceId,
        bytes: Vec<u8>,
        options: PlaybackOptions,
    ) -> Result<(), String> {
        let options = options.sanitized();
        if bytes.is_empty() {
            return Err("streaming audio payload is empty".into());
        }
        if bytes.len() > MAX_ENCODED_STREAM_BYTES {
            return Err(format!("streaming byte payload exceeds the {} MiB limit; use play_streaming_file for large tracks", MAX_ENCODED_STREAM_BYTES / (1024 * 1024)));
        }
        let source_bytes = bytes.len();
        let currently_retained: usize = self
            .voices
            .values()
            .chain(self.retiring_voices.iter())
            .map(|voice| voice.retained_stream_bytes)
            .sum();
        if currently_retained.saturating_add(source_bytes) > MAX_ENCODED_STREAM_BYTES {
            return Err("total retained streaming byte budget is full; use file-backed streaming for large tracks".into());
        }
        let data = StreamingSoundData::from_cursor(Cursor::new(bytes))
            .map_err(|error| format!("could not create streaming decoder: {error}"))?;
        let victim = self.select_voice_victim(id, options.priority)?;
        let route_bus = if self.buses.contains_key(&options.bus) {
            options.bus
        } else {
            AudioBus::Music
        };
        let bus = self
            .buses
            .get_mut(&route_bus)
            .ok_or_else(|| "Kira mixer bus is unavailable".to_owned())?;
        let (mut voice_track, occlusion_filter) = add_voice_track(&mut bus.track)?;
        let mut handle = voice_track
            .play(data)
            .map_err(|error| format!("Kira could not start streaming bytes: {error}"))?;
        if options.looping {
            handle.set_loop_region(0.0..);
        }
        handle.set_volume(
            if options.fade_in_seconds > 0.0 {
                SILENCE_DB
            } else {
                gain_to_db(options.gain)
            },
            Tween::default(),
        );
        if options.fade_in_seconds > 0.0 {
            handle.set_volume(gain_to_db(options.gain), tween(options.fade_in_seconds));
        }
        handle.set_playback_rate(options.playback_rate as f64, Tween::default());
        handle.set_panning(options.pan, Tween::default());
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);
        self.voices.insert(
            id,
            ActiveVoice {
                id,
                handle: LiveHandle::Streaming {
                    handle,
                    state: PlaybackState::Playing,
                },
                _track: voice_track,
                occlusion_filter,
                bus: route_bus,
                priority: options.priority,
                sequence,
                retained_stream_bytes: source_bytes,
                is_streaming: true,
            },
        );
        if let Some(victim) = victim {
            self.retire_voice(victim, DEFAULT_FADE_SECONDS);
        }
        Ok(())
    }

    pub fn stop_voice(&mut self, id: VoiceId, fade_seconds: f32) -> bool {
        let stopped = if let Some(voice) = self.voices.get_mut(&id) {
            voice.handle.stop(tween(fade_seconds));
            true
        } else {
            false
        };
        if stopped {
            self.ambience_layers.retain(|_, voice_id| *voice_id != id);
        }
        stopped
    }
    pub fn pause_voice(&mut self, id: VoiceId) -> bool {
        self.voices
            .get_mut(&id)
            .map(|v| {
                v.handle.pause(Tween::default());
            })
            .is_some()
    }
    pub fn resume_voice(&mut self, id: VoiceId) -> bool {
        self.voices
            .get_mut(&id)
            .map(|v| {
                v.handle.resume(Tween::default());
            })
            .is_some()
    }
    pub fn set_voice_gain(&mut self, id: VoiceId, gain: f32, fade_seconds: f32) -> bool {
        if !gain.is_finite() {
            return false;
        }
        self.voices
            .get_mut(&id)
            .map(|voice| {
                voice
                    .handle
                    .set_gain(gain.clamp(0.0, 4.0), tween(fade_seconds))
            })
            .is_some()
    }
    pub fn set_voice_rate(&mut self, id: VoiceId, rate: f32, fade_seconds: f32) -> bool {
        if !rate.is_finite() || rate <= 0.0 {
            return false;
        }
        self.voices
            .get_mut(&id)
            .map(|voice| {
                voice
                    .handle
                    .set_rate(rate.clamp(0.25, 4.0), tween(fade_seconds))
            })
            .is_some()
    }
    pub fn set_voice_pan(&mut self, id: VoiceId, pan: f32, fade_seconds: f32) -> bool {
        if !pan.is_finite() {
            return false;
        }
        self.voices
            .get_mut(&id)
            .map(|voice| {
                voice
                    .handle
                    .set_pan(pan.clamp(-1.0, 1.0), tween(fade_seconds))
            })
            .is_some()
    }
    /// Update a moving voice's spatial controls and its per-voice occlusion filter.
    pub fn update_voice_spatial(
        &mut self,
        id: VoiceId,
        gain: f32,
        pan: f32,
        rate: f32,
        occlusion: f32,
    ) -> bool {
        if !occlusion.is_finite() {
            return false;
        }
        let occlusion = occlusion.clamp(0.0, 1.0);
        let cutoff = 20_000.0 * (1.0 - occlusion) + 700.0 * occlusion;
        self.update_voice_spatial_acoustics(
            id,
            ManagedEmitterAcoustics {
                gain,
                pan,
                playback_rate: rate,
                occlusion,
                transmission_gain: 1.0,
                low_pass_cutoff_hz: cutoff,
            },
        )
    }

    /// Update listener-relative controls plus material transmission and explicit low-pass cutoff.
    pub fn update_voice_spatial_acoustics(
        &mut self,
        id: VoiceId,
        acoustics: ManagedEmitterAcoustics,
    ) -> bool {
        let ManagedEmitterAcoustics {
            gain,
            pan,
            playback_rate,
            occlusion,
            transmission_gain,
            low_pass_cutoff_hz,
        } = acoustics;
        if !gain.is_finite()
            || !pan.is_finite()
            || !playback_rate.is_finite()
            || !occlusion.is_finite()
            || !transmission_gain.is_finite()
            || !low_pass_cutoff_hz.is_finite()
        {
            return false;
        }
        let Some(voice) = self.voices.get_mut(&id) else {
            return false;
        };
        let effective_gain = gain.clamp(0.0, 4.0) * transmission_gain.clamp(0.0, 1.0);
        let cutoff = low_pass_cutoff_hz.clamp(20.0, 20_000.0);
        let tween = tween(0.05);
        voice.handle.set_gain(effective_gain.clamp(0.0, 4.0), tween);
        voice.handle.set_pan(pan.clamp(-1.0, 1.0), tween);
        voice.handle.set_rate(playback_rate.clamp(0.25, 4.0), tween);
        voice.occlusion_filter.set_cutoff(cutoff as f64, tween);
        voice.occlusion_filter.set_mix(Mix(1.0), tween);
        true
    }

    pub fn set_voice_looping(&mut self, id: VoiceId, looping: bool) -> bool {
        self.voices
            .get_mut(&id)
            .map(|voice| voice.handle.set_looping(looping))
            .is_some()
    }
    pub fn stop_bus(&mut self, bus: AudioBus, fade_seconds: f32) {
        // Master is the root of the bus hierarchy and therefore controls every descendant.
        for voice in self.voices.values_mut() {
            if bus == AudioBus::Master || voice.bus == bus {
                voice.handle.stop(tween(fade_seconds));
            }
        }
        for voice in &mut self.retiring_voices {
            if bus == AudioBus::Master || voice.bus == bus {
                voice.handle.stop(tween(fade_seconds));
            }
        }
        let stopped: HashSet<VoiceId> = self
            .voices
            .values()
            .filter(|voice| bus == AudioBus::Master || voice.bus == bus)
            .map(|voice| voice.id)
            .collect();
        self.ambience_layers
            .retain(|_, voice_id| !stopped.contains(voice_id));
    }
    pub fn stop_all_voices(&mut self, fade_seconds: f32) {
        for voice in self.voices.values_mut() {
            voice.handle.stop(tween(fade_seconds));
        }
        for voice in self.retiring_voices.iter_mut() {
            voice.handle.stop(tween(fade_seconds));
        }
        self.ambience_layers.clear();
    }
    pub fn active_voice_count(&mut self) -> usize {
        self.cleanup_finished();
        self.voices.len() + self.retiring_voices.len()
    }
    pub fn set_bus_gain(&mut self, bus: AudioBus, gain: f32, fade_seconds: f32) {
        if let Some(track) = self.buses.get_mut(&bus) {
            track.set_gain(gain, fade_seconds);
        }
    }
    pub fn set_bus_muted(&mut self, bus: AudioBus, muted: bool, fade_seconds: f32) {
        if let Some(track) = self.buses.get_mut(&bus) {
            track.set_muted(muted, fade_seconds);
        }
    }
    pub fn set_bus_zone_gain(&mut self, bus: AudioBus, gain: f32, fade_seconds: f32) {
        if let Some(track) = self.buses.get_mut(&bus) {
            track.set_zone_gain(gain, fade_seconds);
        }
    }
    pub fn pause_bus(&mut self, bus: AudioBus) {
        for voice in self
            .voices
            .values_mut()
            .filter(|voice| bus == AudioBus::Master || voice.bus == bus)
        {
            voice.handle.pause(Tween::default());
        }
    }
    pub fn resume_bus(&mut self, bus: AudioBus) {
        for voice in self
            .voices
            .values_mut()
            .filter(|voice| bus == AudioBus::Master || voice.bus == bus)
        {
            voice.handle.resume(Tween::default());
        }
    }
    pub fn voice_state(&mut self, id: VoiceId) -> Option<PlaybackStatus> {
        self.cleanup_finished();
        self.voices
            .get(&id)
            .map(|voice| playback_status(voice.handle.state(&voice._track)))
    }
    pub fn diagnostics(&mut self) -> AudioDiagnostics {
        self.cleanup_finished();
        AudioDiagnostics {
            active_voices: self.voices.len(),
            retiring_voices: self.retiring_voices.len(),
            max_voices: MAX_VOICES,
            registered_sounds: self.bank.sound_count(),
            registered_ambiences: self.bank.ambience_count(),
            embedded_encoded_bytes: self.bank.estimated_encoded_bytes(),
            decoded_static_pcm_bytes: self.decoded_static_pcm_bytes,
            active_streaming_voices: self
                .voices
                .values()
                .chain(self.retiring_voices.iter())
                .filter(|voice| voice.is_streaming)
                .count(),
            active_stream_source_bytes: self
                .voices
                .values()
                .chain(self.retiring_voices.iter())
                .map(|voice| voice.retained_stream_bytes)
                .sum(),
            active_ambience_layers: self.ambience_layers.len(),
            decode_failures: self.decode_failures,
            last_decode_error: self.last_decode_error.clone(),
        }
    }

    /// Drain handles whose playback reached its stopped state since the previous drain.
    /// The queue is bounded; `dropped_completion_notifications` reports any overflow so the
    /// application can reconcile its event bookkeeping rather than silently assuming completeness.
    pub fn take_completed_voices(&mut self) -> (Vec<VoiceId>, u64, Vec<VoiceId>) {
        self.cleanup_finished();
        let voices = self.completed_voices.drain(..).collect();
        let mut active: Vec<VoiceId> = self.voices.keys().copied().collect();
        active.extend(self.retiring_voices.iter().map(|voice| voice.id));
        active.sort_by_key(|voice| voice.0);
        (voices, self.dropped_completion_notifications, active)
    }
    pub fn set_bus_effects(&mut self, bus: AudioBus, effects: &[AudioEffect]) {
        self.set_bus_effects_faded(bus, effects, 0.08);
    }

    pub fn set_bus_effects_faded(
        &mut self,
        bus: AudioBus,
        effects: &[AudioEffect],
        transition_seconds: f32,
    ) {
        if let Some(track) = self.buses.get_mut(&bus) {
            track.apply_effects(effects, transition_seconds);
        }
    }

    /// Select a victim without mutating the active set. The caller starts the new Kira sound first
    /// and retires this candidate only after Kira has accepted and registered that sound.
    fn select_voice_victim(
        &mut self,
        id: VoiceId,
        priority: u8,
    ) -> Result<Option<VoiceId>, String> {
        self.cleanup_finished();
        if self.voices.contains_key(&id) || self.retiring_voices.iter().any(|voice| voice.id == id)
        {
            return Err(format!("voice ID {:?} is already in use", id));
        }
        if self.voices.len() < MAX_VOICES {
            return Ok(None);
        }
        let victim = self
            .voices
            .iter()
            .min_by_key(|(voice_id, voice)| (voice.priority, voice.sequence, voice_id.0))
            .map(|(voice_id, voice)| (*voice_id, voice.priority));
        let Some((victim_id, victim_priority)) = victim else {
            return Err("audio voice limit reached".into());
        };
        if priority <= victim_priority {
            return Err("audio voice limit reached; incoming priority is not high enough".into());
        }
        Ok(Some(victim_id))
    }

    fn retire_voice(&mut self, id: VoiceId, fade_seconds: f32) {
        if let Some(mut voice) = self.voices.remove(&id) {
            voice.handle.stop(tween(fade_seconds));
            self.ambience_layers.retain(|_, voice_id| *voice_id != id);
            if self.retiring_voices.len() >= MAX_VOICES {
                let mut oldest = self.retiring_voices.remove(0);
                oldest.handle.stop(Tween::default());
                self.record_completed_voice(oldest.id);
            }
            self.retiring_voices.push(voice);
        }
    }

    fn record_completed_voice(&mut self, voice: VoiceId) {
        if self.completed_voices.len() >= MAX_COMPLETED_VOICE_NOTIFICATIONS {
            self.completed_voices.pop_front();
            self.dropped_completion_notifications =
                self.dropped_completion_notifications.saturating_add(1);
        }
        self.completed_voices.push_back(voice);
    }

    /// Report a failed asynchronous start as a completion so clients can release reservations
    /// made when the command was accepted by the facade queue.
    pub fn report_voice_start_failed(&mut self, voice: VoiceId) {
        if !self.voices.contains_key(&voice)
            && !self.retiring_voices.iter().any(|active| active.id == voice)
        {
            self.record_completed_voice(voice);
        }
    }

    fn allocate_internal_voice(&mut self) -> Result<VoiceId, String> {
        while self.next_internal_voice != 0 {
            let candidate = VoiceId(self.next_internal_voice);
            self.next_internal_voice = self.next_internal_voice.saturating_sub(1);
            if !self.voices.contains_key(&candidate)
                && !self
                    .retiring_voices
                    .iter()
                    .any(|voice| voice.id == candidate)
            {
                return Ok(candidate);
            }
        }
        Err("internal audio voice identifier space exhausted".into())
    }

    /// Start or update one independently controlled looping ambience layer. Passing zero gain
    /// fades out and removes the layer; distinct ambience IDs may play simultaneously.
    pub fn set_ambience_layer(
        &mut self,
        id: AmbienceId,
        gain: f32,
        fade_seconds: f32,
    ) -> Result<(), String> {
        let gain = clamp_finite(gain, 0.0, 4.0, 0.0);
        let fade_seconds = clamp_finite(fade_seconds, 0.0, 60.0, 0.18);
        if gain <= 0.0 {
            self.stop_ambience_layer(id, fade_seconds);
            return Ok(());
        }
        let asset = self
            .bank
            .ambience(id)
            .ok_or_else(|| format!("unknown ambience ID {:?}", id))?;
        // Detach the encoded bytes and gain from the bank borrow before issuing mutable backend
        // operations. The source remains bounded by MAX_ENCODED_STREAM_BYTES on submission.
        let asset_gain = asset.gain;
        if asset.data.len() > MAX_ENCODED_STREAM_BYTES {
            return Err(format!(
                "ambience {:?} exceeds the encoded streaming budget; use file-backed streaming",
                id
            ));
        }
        let encoded = asset.data.to_vec();
        if let Some(voice) = self.ambience_layers.get(&id).copied() {
            if self.set_voice_gain(voice, gain * asset_gain, fade_seconds) {
                return Ok(());
            }
            self.ambience_layers.remove(&id);
        }
        let voice = self.allocate_internal_voice()?;
        let options = PlaybackOptions {
            gain: gain * asset_gain,
            looping: true,
            priority: u8::MAX,
            bus: AudioBus::Ambience,
            fade_in_seconds: fade_seconds,
            ..PlaybackOptions::default()
        };
        self.play_streaming_bytes(voice, encoded, options)?;
        self.ambience_layers.insert(id, voice);
        Ok(())
    }

    pub fn stop_ambience_layer(&mut self, id: AmbienceId, fade_seconds: f32) -> bool {
        if let Some(voice) = self.ambience_layers.remove(&id) {
            self.stop_voice(voice, fade_seconds)
        } else {
            false
        }
    }

    pub fn stop_all_ambience(&mut self, fade_seconds: f32) {
        let ids: Vec<AmbienceId> = self.ambience_layers.keys().copied().collect();
        for id in ids {
            self.stop_ambience_layer(id, fade_seconds);
        }
    }

    /// Compatibility operation that selects one ambience ID, crossfading away from other layers.
    pub fn set_ambience_checked(
        &mut self,
        ambience: Option<AmbienceId>,
        gain: f32,
    ) -> Result<(), String> {
        let gain = clamp_finite(gain, 0.0, 4.0, 0.0);
        let Some(id) = ambience.filter(|_| gain > 0.0) else {
            self.stop_all_ambience(0.18);
            return Ok(());
        };
        let old_ids: Vec<AmbienceId> = self
            .ambience_layers
            .keys()
            .copied()
            .filter(|old| *old != id)
            .collect();
        for old in old_ids {
            self.stop_ambience_layer(old, 0.18);
        }
        self.set_ambience_layer(id, gain, 0.18)
    }

    fn cleanup_finished(&mut self) {
        let finished_active: Vec<VoiceId> = self
            .voices
            .iter()
            .filter_map(|(id, voice)| {
                (voice.handle.state(&voice._track) == PlaybackState::Stopped).then_some(*id)
            })
            .collect();
        for id in finished_active {
            self.voices.remove(&id);
            self.record_completed_voice(id);
        }

        let mut index = 0;
        while index < self.retiring_voices.len() {
            if self.retiring_voices[index]
                .handle
                .state(&self.retiring_voices[index]._track)
                == PlaybackState::Stopped
            {
                let voice = self.retiring_voices.remove(index);
                self.record_completed_voice(voice.id);
            } else {
                index += 1;
            }
        }
        let active_ids: HashSet<VoiceId> = self.voices.keys().copied().collect();
        self.ambience_layers
            .retain(|_, voice_id| active_ids.contains(voice_id));
    }
}

impl AudioBackend for KiraAudioBackend {
    fn play(&mut self, sound: SoundId, volume: f32) {
        let id = VoiceId(self.next_sequence.max(1));
        let _ = self.play_voice(
            id,
            sound,
            PlaybackOptions {
                gain: volume,
                ..PlaybackOptions::default()
            },
            1.0,
        );
    }
    fn play_spatial(&mut self, sound: SoundId, volume: f32, pan: f32) {
        let id = VoiceId(self.next_sequence.max(1));
        let _ = self.play_voice(
            id,
            sound,
            PlaybackOptions {
                gain: volume,
                pan,
                ..PlaybackOptions::default()
            },
            1.0,
        );
    }
    fn play_spatial_rate(&mut self, sound: SoundId, volume: f32, pan: f32, doppler_ratio: f32) {
        let id = VoiceId(self.next_sequence.max(1));
        let _ = self.play_voice(
            id,
            sound,
            PlaybackOptions {
                gain: volume,
                pan,
                ..PlaybackOptions::default()
            },
            doppler_ratio,
        );
    }
    fn set_ambience(&mut self, ambience: Option<AmbienceId>, volume: f32) {
        let _ = self.set_ambience_checked(ambience, volume);
    }
    fn stop_all(&mut self) {
        self.stop_all_voices(0.05);
    }
    fn set_bus_gain(&mut self, bus: AudioBus, gain: f32) {
        KiraAudioBackend::set_bus_gain(self, bus, gain, 0.05);
    }
    fn set_effects(&mut self, bus: AudioBus, effects: &[AudioEffect]) {
        self.set_bus_effects(bus, effects);
    }
    fn set_effects_faded(&mut self, bus: AudioBus, effects: &[AudioEffect], fade_seconds: f32) {
        self.set_bus_effects_faded(bus, effects, fade_seconds);
    }
}

fn add_bus_track(
    manager: &mut AudioManager<DefaultBackend>,
    protect_peaks: bool,
) -> Result<BusTrack, String> {
    let mut builder = TrackBuilder::new().sub_track_capacity(8);
    let low_pass = builder.add_effect(
        FilterBuilder::new()
            .mode(FilterMode::LowPass)
            .cutoff(20_000.0)
            .mix(Mix(1.0)),
    );
    let high_pass = builder.add_effect(
        FilterBuilder::new()
            .mode(FilterMode::HighPass)
            .cutoff(20.0)
            .mix(Mix(0.0)),
    );
    let reverb = builder.add_effect(ReverbBuilder::new().mix(Mix(0.0)));
    let compressor = builder.add_effect(
        CompressorBuilder::new()
            .threshold(-12.0)
            .ratio(4.0)
            .mix(Mix(0.0)),
    );
    let peak_protection = if protect_peaks {
        // A high-ratio compressor protects against sustained peaks; it is not advertised as
        // a brick-wall true-peak limiter.
        Some(
            builder.add_effect(
                CompressorBuilder::new()
                    .threshold(-1.0)
                    .ratio(20.0)
                    .attack_duration(Duration::from_millis(5))
                    .release_duration(Duration::from_millis(80))
                    .mix(Mix(1.0)),
            ),
        )
    } else {
        None
    };
    let track = manager
        .add_sub_track(builder)
        .map_err(|error| format!("could not create Kira master mixer track: {error}"))?;
    Ok(BusTrack {
        track,
        low_pass,
        high_pass,
        reverb,
        compressor,
        _peak_protection: peak_protection,
        gain: 1.0,
        zone_gain: 1.0,
        muted: false,
    })
}

fn add_child_bus_track(parent: &mut TrackHandle) -> Result<BusTrack, String> {
    let mut builder = TrackBuilder::new().sub_track_capacity(MAX_VOICES);
    let low_pass = builder.add_effect(
        FilterBuilder::new()
            .mode(FilterMode::LowPass)
            .cutoff(20_000.0)
            .mix(Mix(1.0)),
    );
    let high_pass = builder.add_effect(
        FilterBuilder::new()
            .mode(FilterMode::HighPass)
            .cutoff(20.0)
            .mix(Mix(0.0)),
    );
    let reverb = builder.add_effect(ReverbBuilder::new().mix(Mix(0.0)));
    let compressor = builder.add_effect(
        CompressorBuilder::new()
            .threshold(-12.0)
            .ratio(4.0)
            .mix(Mix(0.0)),
    );
    let track = parent
        .add_sub_track(builder)
        .map_err(|error| format!("could not create Kira child mixer track: {error}"))?;
    Ok(BusTrack {
        track,
        low_pass,
        high_pass,
        reverb,
        compressor,
        _peak_protection: None,
        gain: 1.0,
        zone_gain: 1.0,
        muted: false,
    })
}

fn add_voice_track(parent: &mut TrackHandle) -> Result<(TrackHandle, FilterHandle), String> {
    let mut builder = TrackBuilder::new();
    let occlusion_filter = builder.add_effect(
        FilterBuilder::new()
            .mode(FilterMode::LowPass)
            .cutoff(20_000.0)
            .mix(Mix(1.0)),
    );
    let track = parent
        .add_sub_track(builder)
        .map_err(|error| format!("could not create Kira voice route: {error}"))?;
    Ok((track, occlusion_filter))
}

fn playback_status(state: PlaybackState) -> PlaybackStatus {
    match state {
        PlaybackState::Playing | PlaybackState::Resuming => PlaybackStatus::Playing,
        PlaybackState::Pausing => PlaybackStatus::Pausing,
        PlaybackState::Paused | PlaybackState::WaitingToResume => PlaybackStatus::Paused,
        PlaybackState::Stopping => PlaybackStatus::Stopping,
        PlaybackState::Stopped => PlaybackStatus::Stopped,
    }
}

fn static_pcm_storage_bytes(data: &StaticSoundData) -> usize {
    std::mem::size_of_val(data.frames.as_ref())
}

fn record_decode_failure(count: &mut usize, last: &mut Option<String>, message: String) {
    *count = (*count).saturating_add(1);
    // Keep diagnostics bounded even if an upstream decoder includes a long malformed-input dump.
    *last = Some(message.chars().take(512).collect());
}

fn clamp_finite(value: f32, min: f32, max: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        fallback
    }
}
fn gain_to_db(gain: f32) -> f32 {
    let gain = clamp_finite(gain, 0.0, 8.0, 0.0);
    if gain <= 0.000_01 {
        SILENCE_DB
    } else {
        (20.0 * gain.log10()).clamp(SILENCE_DB, 18.0)
    }
}
fn tween(seconds: f32) -> Tween {
    let seconds = clamp_finite(seconds, 0.0, 3600.0, 0.0);
    Tween {
        duration: Duration::from_secs_f32(seconds),
        ..Tween::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playback_options_and_gain_mapping_reject_non_finite_values() {
        let options = PlaybackOptions {
            gain: f32::NAN,
            playback_rate: f32::INFINITY,
            pan: f32::NEG_INFINITY,
            fade_in_seconds: f32::NAN,
            ..PlaybackOptions::default()
        }
        .sanitized();
        assert_eq!(options.gain, 1.0);
        assert_eq!(options.playback_rate, 1.0);
        assert_eq!(options.pan, 0.0);
        assert_eq!(options.fade_in_seconds, 0.0);
        assert_eq!(gain_to_db(0.0), SILENCE_DB);
        assert!(gain_to_db(1.0).abs() < 1e-6);
        assert!(gain_to_db(f32::INFINITY).is_finite());
        assert!(tween(f32::NAN).duration.is_zero());
    }

    #[test]
    fn kira_playback_states_map_to_public_voice_states() {
        assert_eq!(
            playback_status(PlaybackState::Playing),
            PlaybackStatus::Playing
        );
        assert_eq!(
            playback_status(PlaybackState::Resuming),
            PlaybackStatus::Playing
        );
        assert_eq!(
            playback_status(PlaybackState::Pausing),
            PlaybackStatus::Pausing
        );
        assert_eq!(
            playback_status(PlaybackState::Paused),
            PlaybackStatus::Paused
        );
        assert_eq!(
            playback_status(PlaybackState::WaitingToResume),
            PlaybackStatus::Paused
        );
        assert_eq!(
            playback_status(PlaybackState::Stopping),
            PlaybackStatus::Stopping
        );
        assert_eq!(
            playback_status(PlaybackState::Stopped),
            PlaybackStatus::Stopped
        );
    }

    #[test]
    fn decode_diagnostics_are_bounded_and_counted() {
        let mut count = 0;
        let mut last = None;
        record_decode_failure(&mut count, &mut last, "x".repeat(2_000));
        assert_eq!(count, 1);
        assert_eq!(last.as_ref().map(String::len), Some(512));
        record_decode_failure(&mut count, &mut last, "decoder error".to_owned());
        assert_eq!(count, 2);
        assert_eq!(last.as_deref(), Some("decoder error"));
    }

    #[test]
    fn static_pcm_accounting_measures_decoded_frames_not_encoded_source_bytes() {
        let encoded = include_bytes!("../examples/assets/tone.wav");
        let decoded = StaticSoundData::from_cursor(Cursor::new(encoded.to_vec()))
            .expect("bundled test WAV must decode");
        assert!(!decoded.frames.is_empty());
        assert_eq!(
            static_pcm_storage_bytes(&decoded),
            decoded.frames.len() * std::mem::size_of_val(&decoded.frames[0])
        );
        assert!(static_pcm_storage_bytes(&decoded) > 0);
    }

    #[test]
    fn stream_byte_limits_are_explicit_and_finite() {
        assert_eq!(MAX_ENCODED_STREAM_BYTES, 64 * 1024 * 1024);
        assert_eq!(MAX_STATIC_SOUND_BYTES, 16 * 1024 * 1024);
        assert_eq!(MAX_STATIC_BANK_BYTES, 64 * 1024 * 1024);
        assert_eq!(MAX_STATIC_SOUND_PCM_BYTES, 64 * 1024 * 1024);
        assert_eq!(MAX_STATIC_BANK_PCM_BYTES, 128 * 1024 * 1024);
        const { assert!(MAX_TOTAL_SUB_TRACKS >= MAX_VOICES * 2) };
    }
}
