//! Kira-backed playback, mixer buses, voice lifecycle, and streaming.
//!
//! The backend is deliberately owner-thread-only. `threaded::ThreadedAudioSink` runs it on
//! a dedicated worker and exposes a bounded command channel to gameplay/ECS systems.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Cursor;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use kira::effect::compressor::{CompressorBuilder, CompressorHandle};
use kira::effect::distortion::{DistortionBuilder, DistortionHandle, DistortionKind};
use kira::effect::eq_filter::{EqFilterBuilder, EqFilterHandle, EqFilterKind};
use kira::effect::filter::{FilterBuilder, FilterHandle, FilterMode};
use kira::effect::reverb::{ReverbBuilder, ReverbHandle};
use kira::effect::{Effect, EffectBuilder};
use kira::info::Info;
use kira::sound::static_sound::{StaticSoundData, StaticSoundHandle};
use kira::sound::streaming::{StreamingSoundData, StreamingSoundHandle};
use kira::sound::FromFileError;
use kira::sound::PlaybackState;
use kira::track::{SendTrackBuilder, SendTrackHandle, SendTrackId, TrackBuilder, TrackHandle};
use kira::{AudioManager, AudioManagerSettings, DefaultBackend, Frame, Mix, Tween};

use crate::bank::{AmbienceId, SoundBank, SoundId};
use crate::debug::AudioDiagnostics;
use crate::decode_budget::estimate_static_pcm_bytes;
use crate::mixer::{AudioBackend, AudioDistortionKind, AudioEffect};
use crate::mixer_graph::{
    AudioSendConfig, AudioSendEffect, AudioSendId, CustomBusConfig, CustomBusId, MixerBusParent,
    MixerGraph, MAX_CUSTOM_BUSES, MAX_SEND_TRACKS,
};
use crate::registry::AudioBus;
#[cfg(feature = "steam-audio")]
use crate::steam_audio::{
    SteamAudioBinauralEffectBuilder, SteamAudioBinauralHandle, SteamAudioDirectEffectBuilder,
    SteamAudioDirectEffectHandle,
};
use crate::voices::{PlaybackStatus, VoiceId};
use crate::ManagedEmitterAcoustics;

const MAX_VOICES: usize = 128;
const MAX_STATIC_SOUND_BYTES: usize = 16 * 1024 * 1024;
const MAX_STATIC_BANK_BYTES: usize = 64 * 1024 * 1024;
// Encoded size is not a useful proxy for decoded memory (highly compressed music can expand
// dramatically). These limits bound the PCM retained by the static sound cache.
const MAX_STATIC_SOUND_PCM_BYTES: usize = 64 * 1024 * 1024;
const MAX_STATIC_BANK_PCM_BYTES: usize = 128 * 1024 * 1024;
// Includes the temporary encoded-byte cursor plus the new decode and all already-retained PCM.
const MAX_STATIC_DECODE_PEAK_BYTES: usize = 128 * 1024 * 1024;
const STATIC_DECODE_SAFETY_BYTES: usize = 512 * 1024;
const DEFAULT_FADE_SECONDS: f32 = 0.12;
const SILENCE_DB: f32 = -100.0;
const MAX_ENCODED_STREAM_BYTES: usize = 64 * 1024 * 1024;
const MAX_TOTAL_SUB_TRACKS: usize = MAX_VOICES * 2 + MAX_CUSTOM_BUSES + 48;
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
    /// Destination built-in mixer bus (also used as the custom bus's root for stop/pause controls).
    pub bus: AudioBus,
    /// Optional dynamic hierarchical mixer bus. The bus must be created before playback.
    pub custom_bus: Option<CustomBusId>,
    /// Fade-in length. Non-finite values are treated as zero.
    pub fade_in_seconds: f32,
    /// Optional Steam Audio HRTF direction in listener-local (right, up, behind) coordinates.
    /// Available only when the `steam-audio` feature is enabled.
    #[cfg(feature = "steam-audio")]
    pub binaural_direction: Option<[f32; 3]>,
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
            custom_bus: None,
            fade_in_seconds: 0.0,
            #[cfg(feature = "steam-audio")]
            binaural_direction: None,
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
            custom_bus: self.custom_bus,
            looping: self.looping,
            fade_in_seconds: clamp_finite(self.fade_in_seconds, 0.0, 60.0, 0.0),
            #[cfg(feature = "steam-audio")]
            binaural_direction: self.binaural_direction,
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

#[derive(Clone, Copy)]
enum ReplayVoiceSource {
    Sound(SoundId),
    Ambience(AmbienceId),
}

#[derive(Clone, Copy)]
struct ReplayVoiceState {
    source: ReplayVoiceSource,
    options: PlaybackOptions,
    doppler: f32,
    acoustics: Option<ManagedEmitterAcoustics>,
    paused: bool,
}

struct ActiveVoice {
    id: VoiceId,
    handle: LiveHandle,
    // Keeping this handle alive preserves the per-voice route and occlusion filter.
    _track: TrackHandle,
    occlusion_filter: FilterHandle,
    bus: AudioBus,
    custom_bus: Option<CustomBusId>,
    priority: u8,
    sequence: u64,
    retained_stream_bytes: usize,
    is_streaming: bool,
    replay: Option<ReplayVoiceState>,
    #[cfg(feature = "steam-audio")]
    binaural: Option<SteamAudioBinauralHandle>,
    #[cfg(feature = "steam-audio")]
    direct_acoustics: Option<SteamAudioDirectEffectHandle>,
}

#[derive(Clone)]
struct RealtimeDelayShared {
    delay_seconds_bits: Arc<AtomicU32>,
    feedback_db_bits: Arc<AtomicU32>,
    wet_bits: Arc<AtomicU32>,
}

struct RealtimeDelayHandle(RealtimeDelayShared);

impl RealtimeDelayHandle {
    fn set_delay_seconds(&self, seconds: f32) {
        self.0
            .delay_seconds_bits
            .store(seconds.to_bits(), Ordering::Relaxed);
    }
    fn set_feedback_db(&self, db: f32) {
        self.0
            .feedback_db_bits
            .store(db.to_bits(), Ordering::Relaxed);
    }
    fn set_mix(&self, wet: f32) {
        self.0.wet_bits.store(wet.to_bits(), Ordering::Relaxed);
    }
}

struct RealtimeDelayBuilder {
    delay_seconds: f32,
    feedback_db: f32,
    wet: f32,
    wet_only: bool,
}

impl RealtimeDelayBuilder {
    fn new(delay_seconds: f32, feedback_db: f32, wet: f32) -> Self {
        Self {
            delay_seconds,
            feedback_db,
            wet,
            wet_only: false,
        }
    }

    fn new_send(delay_seconds: f32, feedback_db: f32, wet: f32) -> Self {
        Self {
            delay_seconds,
            feedback_db,
            wet,
            wet_only: true,
        }
    }
}

struct RealtimeDelayEffect {
    shared: RealtimeDelayShared,
    sample_rate: u32,
    buffer: Vec<[f32; 2]>,
    cursor: usize,
    current_delay_seconds: f32,
    current_feedback_db: f32,
    current_wet: f32,
    wet_only: bool,
}

impl Effect for RealtimeDelayEffect {
    fn init(&mut self, sample_rate: u32, internal_buffer_size: usize) {
        self.sample_rate = sample_rate.max(1);
        let capacity = (self.sample_rate as usize)
            .saturating_add(internal_buffer_size.max(1))
            .max(2);
        self.buffer.clear();
        self.buffer.resize(capacity, [0.0; 2]);
        self.cursor = 0;
        self.current_delay_seconds =
            f32::from_bits(self.shared.delay_seconds_bits.load(Ordering::Relaxed)).clamp(0.01, 1.0);
        self.current_feedback_db =
            f32::from_bits(self.shared.feedback_db_bits.load(Ordering::Relaxed)).clamp(-60.0, 0.0);
        self.current_wet =
            f32::from_bits(self.shared.wet_bits.load(Ordering::Relaxed)).clamp(0.0, 1.0);
    }

    fn on_change_sample_rate(&mut self, sample_rate: u32) {
        self.sample_rate = sample_rate.max(1);
        let capacity = (self.sample_rate as usize).saturating_add(1).max(2);
        self.buffer.clear();
        self.buffer.resize(capacity, [0.0; 2]);
        self.cursor = 0;
    }

    fn process(&mut self, input: &mut [Frame], _dt: f64, _info: &Info<'_>) {
        let target_delay =
            f32::from_bits(self.shared.delay_seconds_bits.load(Ordering::Relaxed)).clamp(0.01, 1.0);
        let target_feedback_db =
            f32::from_bits(self.shared.feedback_db_bits.load(Ordering::Relaxed)).clamp(-60.0, 0.0);
        let target_wet =
            f32::from_bits(self.shared.wet_bits.load(Ordering::Relaxed)).clamp(0.0, 1.0);
        let capacity = self.buffer.len();
        if capacity < 2 {
            return;
        }
        // Smooth live controls over roughly 20 ms to avoid zipper noise and abrupt delay jumps.
        let smoothing = 1.0 - (-1.0f32 / (0.02 * self.sample_rate.max(1) as f32)).exp();
        for frame in input {
            self.current_delay_seconds += (target_delay - self.current_delay_seconds) * smoothing;
            self.current_feedback_db += (target_feedback_db - self.current_feedback_db) * smoothing;
            self.current_wet += (target_wet - self.current_wet) * smoothing;
            let feedback = 10.0f32
                .powf(self.current_feedback_db / 20.0)
                .clamp(0.0, 1.0);
            let delay_frames = ((self.current_delay_seconds * self.sample_rate as f32).round()
                as usize)
                .clamp(1, capacity - 1);
            let read = (self.cursor + capacity - delay_frames) % capacity;
            let delayed = self.buffer[read];
            let dry_left = frame.left;
            let dry_right = frame.right;
            self.buffer[self.cursor] = [
                dry_left + delayed[0] * feedback,
                dry_right + delayed[1] * feedback,
            ];
            if self.wet_only {
                // A send track already carries the dry signal through its parent route. Returning
                // dry audio here would duplicate it and create an unintended level increase.
                frame.left = delayed[0] * self.current_wet;
                frame.right = delayed[1] * self.current_wet;
            } else {
                frame.left = dry_left * (1.0 - self.current_wet) + delayed[0] * self.current_wet;
                frame.right = dry_right * (1.0 - self.current_wet) + delayed[1] * self.current_wet;
            }
            self.cursor += 1;
            if self.cursor == capacity {
                self.cursor = 0;
            }
        }
    }
}

impl EffectBuilder for RealtimeDelayBuilder {
    type Handle = RealtimeDelayHandle;

    fn build(self) -> (Box<dyn Effect>, Self::Handle) {
        let shared = RealtimeDelayShared {
            delay_seconds_bits: Arc::new(AtomicU32::new(
                self.delay_seconds.clamp(0.01, 1.0).to_bits(),
            )),
            feedback_db_bits: Arc::new(AtomicU32::new(
                self.feedback_db.clamp(-60.0, 0.0).to_bits(),
            )),
            wet_bits: Arc::new(AtomicU32::new(self.wet.clamp(0.0, 1.0).to_bits())),
        };
        let handle = RealtimeDelayHandle(shared.clone());
        let effect = RealtimeDelayEffect {
            shared,
            sample_rate: 48_000,
            buffer: Vec::new(),
            cursor: 0,
            current_delay_seconds: self.delay_seconds.clamp(0.01, 1.0),
            current_feedback_db: self.feedback_db.clamp(-60.0, 0.0),
            current_wet: self.wet.clamp(0.0, 1.0),
            wet_only: self.wet_only,
        };
        (Box::new(effect), handle)
    }
}

const CONVOLUTION_TAPS: usize = 48;

#[derive(Clone)]
struct RealtimeConvolutionShared {
    decay_seconds_bits: Arc<AtomicU32>,
    wet_bits: Arc<AtomicU32>,
}

struct RealtimeConvolutionHandle(RealtimeConvolutionShared);

impl RealtimeConvolutionHandle {
    fn set_decay_seconds(&self, seconds: f32) {
        self.0
            .decay_seconds_bits
            .store(seconds.to_bits(), Ordering::Relaxed);
    }

    fn set_mix(&self, wet: f32) {
        self.0.wet_bits.store(wet.to_bits(), Ordering::Relaxed);
    }
}

struct RealtimeConvolutionBuilder {
    decay_seconds: f32,
    wet: f32,
}

impl RealtimeConvolutionBuilder {
    fn new(decay_seconds: f32, wet: f32) -> Self {
        Self { decay_seconds, wet }
    }
}

struct RealtimeConvolutionEffect {
    shared: RealtimeConvolutionShared,
    sample_rate: u32,
    history: Vec<[f32; 2]>,
    cursor: usize,
    tap_offsets: [usize; CONVOLUTION_TAPS],
    tap_gains: [f32; CONVOLUTION_TAPS],
    prepared_decay_bits: u32,
    current_wet: f32,
}

impl RealtimeConvolutionEffect {
    fn prepare_kernel(&mut self, decay_seconds: f32) {
        let decay = decay_seconds.clamp(0.05, 1.0);
        let sample_rate = self.sample_rate.max(1) as f32;
        for index in 0..CONVOLUTION_TAPS {
            let progress = (index + 1) as f32 / (CONVOLUTION_TAPS + 1) as f32;
            let time = progress * decay;
            self.tap_offsets[index] = ((time * sample_rate).round() as usize)
                .clamp(1, self.history.len().saturating_sub(1).max(1));
            let polarity = if index % 7 == 0 { -1.0 } else { 1.0 };
            self.tap_gains[index] = polarity * (-5.5 * progress).exp() * 0.16;
        }
        self.prepared_decay_bits = decay.to_bits();
    }
}

impl Effect for RealtimeConvolutionEffect {
    fn init(&mut self, sample_rate: u32, internal_buffer_size: usize) {
        self.sample_rate = sample_rate.max(1);
        // The bounded history is allocated during effect initialization, never in process().
        let capacity = (self.sample_rate as usize)
            .saturating_add(internal_buffer_size.max(1))
            .max(2);
        self.history.clear();
        self.history.resize(capacity, [0.0; 2]);
        self.cursor = 0;
        let decay = f32::from_bits(self.shared.decay_seconds_bits.load(Ordering::Relaxed));
        self.prepare_kernel(decay);
        self.current_wet =
            f32::from_bits(self.shared.wet_bits.load(Ordering::Relaxed)).clamp(0.0, 1.0);
    }

    fn on_change_sample_rate(&mut self, sample_rate: u32) {
        self.sample_rate = sample_rate.max(1);
        let capacity = (self.sample_rate as usize).saturating_add(1).max(2);
        self.history.clear();
        self.history.resize(capacity, [0.0; 2]);
        self.cursor = 0;
        let decay = f32::from_bits(self.shared.decay_seconds_bits.load(Ordering::Relaxed));
        self.prepare_kernel(decay);
    }

    fn process(&mut self, input: &mut [Frame], _dt: f64, _info: &Info<'_>) {
        let target_wet =
            f32::from_bits(self.shared.wet_bits.load(Ordering::Relaxed)).clamp(0.0, 1.0);
        if self.history.len() < 2 {
            return;
        }
        let decay_bits = self.shared.decay_seconds_bits.load(Ordering::Relaxed);
        if decay_bits != self.prepared_decay_bits {
            self.prepare_kernel(f32::from_bits(decay_bits));
        }
        let capacity = self.history.len();
        let smoothing = 1.0 - (-1.0f32 / (0.02 * self.sample_rate.max(1) as f32)).exp();
        for frame in input {
            self.current_wet += (target_wet - self.current_wet) * smoothing;
            let dry = [frame.left, frame.right];
            self.history[self.cursor] = dry;
            if self.current_wet > f32::EPSILON {
                let mut convolved = [0.0f32; 2];
                for tap in 0..CONVOLUTION_TAPS {
                    let offset = self.tap_offsets[tap].min(capacity - 1);
                    let read = (self.cursor + capacity - offset) % capacity;
                    let sample = self.history[read];
                    let gain = self.tap_gains[tap];
                    convolved[0] += sample[0] * gain;
                    convolved[1] += sample[1] * gain;
                }
                frame.left = dry[0] * (1.0 - self.current_wet) + convolved[0] * self.current_wet;
                frame.right = dry[1] * (1.0 - self.current_wet) + convolved[1] * self.current_wet;
            }
            self.cursor += 1;
            if self.cursor == capacity {
                self.cursor = 0;
            }
        }
    }
}

impl EffectBuilder for RealtimeConvolutionBuilder {
    type Handle = RealtimeConvolutionHandle;

    fn build(self) -> (Box<dyn Effect>, Self::Handle) {
        let shared = RealtimeConvolutionShared {
            decay_seconds_bits: Arc::new(AtomicU32::new(
                self.decay_seconds.clamp(0.05, 1.0).to_bits(),
            )),
            wet_bits: Arc::new(AtomicU32::new(self.wet.clamp(0.0, 1.0).to_bits())),
        };
        let handle = RealtimeConvolutionHandle(shared.clone());
        let effect = RealtimeConvolutionEffect {
            shared,
            sample_rate: 48_000,
            history: Vec::new(),
            cursor: 0,
            tap_offsets: [1; CONVOLUTION_TAPS],
            tap_gains: [0.0; CONVOLUTION_TAPS],
            prepared_decay_bits: 0,
            current_wet: self.wet.clamp(0.0, 1.0),
        };
        (Box::new(effect), handle)
    }
}

struct BusTrack {
    track: TrackHandle,
    low_pass: FilterHandle,
    high_pass: FilterHandle,
    reverb: ReverbHandle,
    compressor: CompressorHandle,
    eq: [EqFilterHandle; 4],
    delay: RealtimeDelayHandle,
    distortion: DistortionHandle,
    convolution: RealtimeConvolutionHandle,
    // Master-only peak-protection compressor. It remains in the route when snapshots change.
    _peak_protection: Option<CompressorHandle>,
    gain: f32,
    zone_gain: f32,
    muted: bool,
    /// Track-level pause state also applies to future child voices and survives device rebuilds.
    paused: bool,
    effect_chain: Vec<AudioEffect>,
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
        for eq in &mut self.eq {
            eq.set_frequency(1_000.0_f64, tween);
            eq.set_gain(0.0_f32, tween);
            eq.set_q(1.0_f64, tween);
        }
        self.delay.set_delay_seconds(0.25);
        self.delay.set_feedback_db(-12.0);
        self.delay.set_mix(0.0);
        self.distortion.set_kind(DistortionKind::SoftClip);
        self.distortion.set_drive(0.0_f32, tween);
        self.distortion.set_mix(Mix(0.0), tween);
        self.convolution.set_decay_seconds(0.8);
        self.convolution.set_mix(0.0);

        self.effect_chain.clear();
        self.effect_chain
            .extend(effects.iter().copied().map(AudioEffect::sanitized));
        let mut eq_band = 0usize;
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
                AudioEffect::Eq {
                    frequency_hz,
                    gain_db,
                    q,
                } => {
                    if let Some(eq) = self.eq.get_mut(eq_band) {
                        eq.set_frequency(frequency_hz as f64, tween);
                        eq.set_gain(gain_db, tween);
                        eq.set_q(q as f64, tween);
                        eq_band += 1;
                    }
                }
                AudioEffect::Reverb { wet, decay_seconds } => {
                    self.reverb.set_mix(Mix(wet), tween);
                    // Kira feedback approaches infinite sustain at 1.0. Keep a finite margin.
                    self.reverb
                        .set_feedback((0.15 + decay_seconds as f64 * 0.078).min(0.93), tween);
                    self.reverb
                        .set_damping((1.0 - (decay_seconds as f64 / 12.0)).clamp(0.1, 1.0), tween);
                }
                AudioEffect::Delay {
                    delay_seconds,
                    feedback_db,
                    wet,
                } => {
                    self.delay.set_delay_seconds(delay_seconds);
                    self.delay.set_feedback_db(feedback_db);
                    self.delay.set_mix(wet);
                }
                AudioEffect::Distortion {
                    kind,
                    drive_db,
                    wet,
                } => {
                    self.distortion.set_kind(match kind {
                        AudioDistortionKind::SoftClip => DistortionKind::SoftClip,
                        AudioDistortionKind::HardClip => DistortionKind::HardClip,
                    });
                    self.distortion.set_drive(drive_db, tween);
                    self.distortion.set_mix(Mix(wet), tween);
                }
                AudioEffect::Convolution { decay_seconds, wet } => {
                    self.convolution.set_decay_seconds(decay_seconds);
                    self.convolution.set_mix(wet);
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
#[derive(Clone)]
struct StaticDecodeCache {
    // Kira stores decoded frames in Arc<[Frame]>, so cloning these handles retains the exact
    // same PCM allocation rather than copying decoded audio during device recovery.
    decoded: HashMap<SoundId, StaticSoundData>,
    decoded_static_pcm_bytes: usize,
    peak_static_decode_bytes: usize,
    decode_failures: usize,
    last_decode_error: Option<String>,
}

pub struct KiraAudioBackend {
    // Owns the device/backend thread; retained for its Drop/lifetime semantics.
    _manager: AudioManager<DefaultBackend>,
    bank: SoundBank,
    decoded: HashMap<SoundId, StaticSoundData>,
    buses: HashMap<AudioBus, BusTrack>,
    custom_buses: HashMap<CustomBusId, BusTrack>,
    send_tracks: HashMap<AudioSendId, SendTrackHandle>,
    mixer_graph: MixerGraph,
    voices: HashMap<VoiceId, ActiveVoice>,
    // Stolen voices retain their route until the stop tween finishes. This vector is capped.
    retiring_voices: Vec<ActiveVoice>,
    next_sequence: u64,
    next_internal_voice: u64,
    ambience_layers: HashMap<AmbienceId, VoiceId>,
    completed_voices: VecDeque<VoiceId>,
    dropped_completion_notifications: u64,
    decoded_static_pcm_bytes: usize,
    peak_static_decode_bytes: usize,
    decode_failures: usize,
    last_decode_error: Option<String>,
    device_recovery_attempts: u64,
    device_recoveries: u64,
    last_device_error: Option<String>,
}

impl KiraAudioBackend {
    /// Open the platform output device, build the mixer graph, and predecode short embedded clips.
    pub fn new(bank: SoundBank) -> Result<Self, String> {
        Self::new_with_decode_cache(bank, None)
    }

    fn new_with_decode_cache(
        bank: SoundBank,
        cached_decode: Option<StaticDecodeCache>,
    ) -> Result<Self, String> {
        bank.validate()
            .map_err(|error| format!("invalid audio sound bank: {error}"))?;
        let mut manager_settings = AudioManagerSettings::default();
        // A voice owns one child track; allow the active voice budget, its bounded retiring
        // tail, and the six standard buses to coexist in Kira's global track arena.
        manager_settings.capacities.sub_track_capacity = MAX_TOTAL_SUB_TRACKS;
        manager_settings.capacities.send_track_capacity = MAX_SEND_TRACKS;
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
            let track = add_child_bus_track(&mut master.track, &[])?;
            buses.insert(bus, track);
        }
        buses.insert(AudioBus::Master, master);

        let StaticDecodeCache {
            decoded,
            decoded_static_pcm_bytes,
            peak_static_decode_bytes,
            decode_failures,
            last_decode_error,
        } = cached_decode.unwrap_or_else(|| {
            let mut decoded = HashMap::new();
            // Content-keyed cache ensures different IDs referencing identical bytes share one decode.
            let mut unique: HashMap<&'static [u8], StaticSoundData> = HashMap::new();
            let mut encoded_bank_bytes = 0usize;
            let mut decoded_static_pcm_bytes = 0usize;
            let mut peak_static_decode_bytes = 0usize;
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
                    // Estimate decoded storage before asking Symphonia/Kira to allocate PCM. A
                    // malformed/unknown container is not admitted to the static cache; the public
                    // play path will use Kira's chunked streaming decoder instead.
                    let Some(estimated_pcm_bytes) = estimate_static_pcm_bytes(asset.data) else {
                        record_decode_failure(
                            &mut decode_failures,
                            &mut last_decode_error,
                            format!("sound {:?} has no safe static PCM estimate; deferred to streaming", asset.id),
                        );
                        continue;
                    };
                    let estimated_bank_total = decoded_static_pcm_bytes.saturating_add(estimated_pcm_bytes);
                    let estimated_peak = decoded_static_pcm_bytes
                        .saturating_add(asset.data.len())
                        .saturating_add(estimated_pcm_bytes)
                        .saturating_add(STATIC_DECODE_SAFETY_BYTES);
                    if estimated_pcm_bytes > MAX_STATIC_SOUND_PCM_BYTES
                        || estimated_bank_total > MAX_STATIC_BANK_PCM_BYTES
                        || estimated_peak > MAX_STATIC_DECODE_PEAK_BYTES
                    {
                        record_decode_failure(
                            &mut decode_failures,
                            &mut last_decode_error,
                            format!(
                                "sound {:?} preflight estimates {} MiB decoded PCM and {} MiB peak decode memory; static PCM limits are {} MiB per sound / {} MiB per bank / {} MiB peak; deferred to streaming",
                                asset.id,
                                estimated_pcm_bytes / (1024 * 1024),
                                estimated_peak / (1024 * 1024),
                                MAX_STATIC_SOUND_PCM_BYTES / (1024 * 1024),
                                MAX_STATIC_BANK_PCM_BYTES / (1024 * 1024),
                                MAX_STATIC_DECODE_PEAK_BYTES / (1024 * 1024),
                            ),
                        );
                        continue;
                    }
                    // Record the conservative transient peak before decoding, including attempts
                    // whose codec later rejects the stream after allocating scratch/PCM buffers.
                    peak_static_decode_bytes = peak_static_decode_bytes.max(estimated_peak);
                    match StaticSoundData::from_cursor(Cursor::new(asset.data.to_vec())) {
                        Ok(data) => {
                            let pcm_bytes = static_pcm_storage_bytes(&data);
                            let next_pcm_total = decoded_static_pcm_bytes.saturating_add(pcm_bytes);
                            let actual_peak = decoded_static_pcm_bytes
                                .saturating_add(asset.data.len())
                                .saturating_add(pcm_bytes)
                                .saturating_add(STATIC_DECODE_SAFETY_BYTES);
                            peak_static_decode_bytes = peak_static_decode_bytes.max(actual_peak);
                            if data.frames.is_empty()
                                || pcm_bytes > MAX_STATIC_SOUND_PCM_BYTES
                                || next_pcm_total > MAX_STATIC_BANK_PCM_BYTES
                                || actual_peak > MAX_STATIC_DECODE_PEAK_BYTES
                            {
                                record_decode_failure(
                                    &mut decode_failures,
                                    &mut last_decode_error,
                                    format!(
                                        "sound {:?} decoded to {} MiB PCM with an estimated {} MiB peak; static limits are {} MiB per sound / {} MiB per bank / {} MiB peak; use streaming",
                                        asset.id,
                                        pcm_bytes / (1024 * 1024),
                                        actual_peak / (1024 * 1024),
                                        MAX_STATIC_SOUND_PCM_BYTES / (1024 * 1024),
                                        MAX_STATIC_BANK_PCM_BYTES / (1024 * 1024),
                                        MAX_STATIC_DECODE_PEAK_BYTES / (1024 * 1024),
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
            StaticDecodeCache {
                decoded,
                decoded_static_pcm_bytes,
                peak_static_decode_bytes,
                decode_failures,
                last_decode_error,
            }
        });

        Ok(Self {
            _manager: manager,
            bank,
            decoded,
            buses,
            custom_buses: HashMap::with_capacity(MAX_CUSTOM_BUSES),
            send_tracks: HashMap::with_capacity(MAX_SEND_TRACKS),
            mixer_graph: MixerGraph::new(),
            voices: HashMap::new(),
            retiring_voices: Vec::new(),
            next_sequence: 1,
            next_internal_voice: u64::MAX,
            ambience_layers: HashMap::new(),
            decoded_static_pcm_bytes,
            peak_static_decode_bytes,
            completed_voices: VecDeque::with_capacity(MAX_COMPLETED_VOICE_NOTIFICATIONS),
            dropped_completion_notifications: 0,
            decode_failures,
            last_decode_error,
            device_recovery_attempts: 0,
            device_recoveries: 0,
            last_device_error: None,
        })
    }

    fn resolve_output_track(
        &mut self,
        options: PlaybackOptions,
        fallback: AudioBus,
    ) -> Result<(AudioBus, Option<CustomBusId>, &mut BusTrack), String> {
        if let Some(custom_id) = options.custom_bus {
            let root = self.mixer_graph.root_audio_bus(custom_id)?;
            let track = self
                .custom_buses
                .get_mut(&custom_id)
                .ok_or_else(|| format!("custom mixer bus {:?} is not available", custom_id))?;
            Ok((root, Some(custom_id), track))
        } else {
            let bus = if self.buses.contains_key(&options.bus) {
                options.bus
            } else {
                fallback
            };
            let track = self
                .buses
                .get_mut(&bus)
                .ok_or_else(|| "Kira mixer bus is unavailable".to_owned())?;
            Ok((bus, None, track))
        }
    }

    /// Create an effect send shared by any custom buses configured with a route to it.
    pub fn create_audio_send(&mut self, config: AudioSendConfig) -> Result<(), String> {
        self.mixer_graph.create_send(config.clone())?;
        let mut builder = SendTrackBuilder::new().volume(gain_to_db(config.gain));
        match config.effect {
            AudioSendEffect::Reverb { wet, decay_seconds } => {
                // A send is parallel to the dry bus route, so the send output must be wet-only.
                // Apply the user wet amount as send-return gain instead of mixing dry audio again.
                builder = builder.volume(gain_to_db(config.gain * wet));
                builder.add_effect(
                    ReverbBuilder::new()
                        .mix(Mix(1.0))
                        .feedback((0.15 + decay_seconds as f64 * 0.078).min(0.93))
                        .damping((1.0 - decay_seconds as f64 / 12.0).clamp(0.1, 1.0)),
                );
            }
            AudioSendEffect::Delay {
                delay_seconds,
                feedback_db,
                wet,
            } => {
                builder.add_effect(RealtimeDelayBuilder::new_send(
                    delay_seconds,
                    feedback_db,
                    wet,
                ));
            }
        }
        match self._manager.add_send_track(builder) {
            Ok(handle) => {
                self.send_tracks.insert(config.id, handle);
                Ok(())
            }
            Err(error) => {
                let _ = self.mixer_graph.remove_send(config.id);
                Err(format!("could not create Kira send track: {error}"))
            }
        }
    }

    pub fn remove_audio_send(&mut self, id: AudioSendId) -> Result<(), String> {
        self.mixer_graph.remove_send(id)?;
        self.send_tracks.remove(&id);
        Ok(())
    }

    pub fn set_audio_send_gain(
        &mut self,
        id: AudioSendId,
        gain: f32,
        fade_seconds: f32,
    ) -> Result<(), String> {
        if !gain.is_finite() || !(0.0..=2.0).contains(&gain) {
            return Err("send gain must be finite and within [0, 2]".into());
        }
        let handle = self
            .send_tracks
            .get_mut(&id)
            .ok_or_else(|| format!("unknown audio send {:?}", id))?;
        let wet_return = match self.mixer_graph.send(id).map(|send| send.effect) {
            Some(AudioSendEffect::Reverb { wet, .. }) => wet,
            Some(AudioSendEffect::Delay { .. }) => 1.0,
            None => return Err(format!("unknown audio send {:?}", id)),
        };
        handle.set_volume(gain_to_db(gain * wet_return), tween(fade_seconds));
        self.mixer_graph.set_send_master_gain(id, gain)
    }

    pub fn create_custom_bus(&mut self, config: CustomBusConfig) -> Result<(), String> {
        self.mixer_graph.create_bus(config.clone())?;
        let routes: Result<Vec<(SendTrackId, f32)>, String> = config
            .sends
            .iter()
            .map(|route| {
                self.send_tracks
                    .get(&route.send)
                    .map(|handle| (handle.id(), route.gain))
                    .ok_or_else(|| {
                        format!("custom bus references unavailable send {:?}", route.send)
                    })
            })
            .collect();
        let routes = match routes {
            Ok(routes) => routes,
            Err(error) => {
                let _ = self.mixer_graph.remove_bus(config.id);
                return Err(error);
            }
        };
        let created = match config.parent {
            MixerBusParent::BuiltIn(parent) => self
                .buses
                .get_mut(&parent)
                .ok_or_else(|| format!("built-in parent bus {:?} is unavailable", parent))
                .and_then(|parent| add_child_bus_track(&mut parent.track, &routes)),
            MixerBusParent::Custom(parent) => self
                .custom_buses
                .get_mut(&parent)
                .ok_or_else(|| format!("custom parent bus {:?} is unavailable", parent))
                .and_then(|parent| add_child_bus_track(&mut parent.track, &routes)),
        };
        match created {
            Ok(mut track) => {
                track.set_gain(config.gain, 0.0);
                track.set_muted(config.muted, 0.0);
                self.custom_buses.insert(config.id, track);
                Ok(())
            }
            Err(error) => {
                let _ = self.mixer_graph.remove_bus(config.id);
                Err(error)
            }
        }
    }

    pub fn remove_custom_bus(&mut self, id: CustomBusId) -> Result<(), String> {
        if self
            .voices
            .values()
            .any(|voice| voice.custom_bus == Some(id))
            || self
                .retiring_voices
                .iter()
                .any(|voice| voice.custom_bus == Some(id))
        {
            return Err(format!("custom bus {:?} still owns active voices", id));
        }
        self.mixer_graph.remove_bus(id)?;
        self.custom_buses.remove(&id);
        Ok(())
    }

    pub fn set_custom_bus_gain(
        &mut self,
        id: CustomBusId,
        gain: f32,
        fade_seconds: f32,
    ) -> Result<(), String> {
        self.mixer_graph.set_bus_gain(id, gain)?;
        let track = self
            .custom_buses
            .get_mut(&id)
            .ok_or_else(|| format!("unknown custom bus {:?}", id))?;
        track.set_gain(gain, fade_seconds);
        Ok(())
    }

    pub fn set_custom_bus_muted(
        &mut self,
        id: CustomBusId,
        muted: bool,
        fade_seconds: f32,
    ) -> Result<(), String> {
        self.mixer_graph.set_bus_muted(id, muted)?;
        let track = self
            .custom_buses
            .get_mut(&id)
            .ok_or_else(|| format!("unknown custom bus {:?}", id))?;
        track.set_muted(muted, fade_seconds);
        Ok(())
    }

    pub fn set_custom_bus_effects(
        &mut self,
        id: CustomBusId,
        effects: &[AudioEffect],
        fade_seconds: f32,
    ) -> Result<(), String> {
        crate::mixer::validate_effect_chain(effects)?;
        let track = self
            .custom_buses
            .get_mut(&id)
            .ok_or_else(|| format!("unknown custom bus {:?}", id))?;
        track.apply_effects(effects, fade_seconds);
        Ok(())
    }

    pub fn set_custom_bus_send_gain(
        &mut self,
        bus: CustomBusId,
        send: AudioSendId,
        gain: f32,
        fade_seconds: f32,
    ) -> Result<(), String> {
        if !gain.is_finite() || !(0.0..=2.0).contains(&gain) {
            return Err("send route gain must be finite and within [0, 2]".into());
        }
        let send_id = self
            .send_tracks
            .get(&send)
            .ok_or_else(|| format!("unknown audio send {:?}", send))?
            .id();
        let track = self
            .custom_buses
            .get_mut(&bus)
            .ok_or_else(|| format!("unknown custom bus {:?}", bus))?;
        track
            .track
            .set_send(send_id, gain_to_db(gain), tween(fade_seconds))
            .map_err(|_| {
                format!(
                    "custom bus {:?} has no predeclared route to send {:?}",
                    bus, send
                )
            })?;
        self.mixer_graph.set_send_gain(bus, send, gain)
    }

    pub fn stop_custom_bus(&mut self, id: CustomBusId, fade_seconds: f32) -> bool {
        let affected_buses = self.mixer_graph.buses_in_subtree(id);
        if affected_buses.is_empty() {
            return false;
        }
        let mut stopped = false;
        for voice in self.voices.values_mut().filter(|voice| {
            voice
                .custom_bus
                .is_some_and(|bus| affected_buses.contains(&bus))
        }) {
            voice.handle.stop(tween(fade_seconds));
            stopped = true;
        }
        for voice in &mut self.retiring_voices {
            if voice
                .custom_bus
                .is_some_and(|bus| affected_buses.contains(&bus))
            {
                voice.handle.stop(tween(fade_seconds));
                stopped = true;
            }
        }
        stopped
    }

    pub fn pause_custom_bus(&mut self, id: CustomBusId) -> bool {
        if !self.mixer_graph.contains_bus_in_subtree(id, id) {
            return false;
        }
        let Some(track) = self.custom_buses.get_mut(&id) else {
            return false;
        };
        track.track.pause(Tween::default());
        track.paused = true;
        true
    }

    pub fn resume_custom_bus(&mut self, id: CustomBusId) -> bool {
        if !self.mixer_graph.contains_bus_in_subtree(id, id) {
            return false;
        }
        let Some(track) = self.custom_buses.get_mut(&id) else {
            return false;
        };
        track.track.resume(Tween::default());
        track.paused = false;
        true
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
        let asset = self
            .bank
            .sound(sound)
            .ok_or_else(|| format!("unknown sound id {:?}", sound))?;
        if !self.decoded.contains_key(&sound) {
            // Long clips, formats without reliable duration metadata and clips which exceed the
            // retained-PCM budget stay playable: Kira streams them in bounded decoded chunks.
            let mut streaming_options = options;
            streaming_options.gain = clamp_finite(options.gain * asset.gain, 0.0, 4.0, 0.0);
            streaming_options.playback_rate = clamp_finite(
                options.playback_rate * asset.speed * doppler,
                0.25,
                4.0,
                1.0,
            );
            self.play_streaming_bytes(id, asset.data.to_vec(), streaming_options)?;
            if let Some(voice) = self.voices.get_mut(&id) {
                voice.replay = Some(ReplayVoiceState {
                    source: ReplayVoiceSource::Sound(sound),
                    options,
                    doppler,
                    acoustics: None,
                    paused: false,
                });
            }
            return Ok(());
        }
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
        let (route_bus, _custom_bus, bus) = self.resolve_output_track(options, AudioBus::Sfx)?;
        #[cfg(feature = "steam-audio")]
        let (mut voice_track, occlusion_filter, binaural, direct_acoustics) =
            add_voice_track(&mut bus.track, options.binaural_direction)?;
        #[cfg(not(feature = "steam-audio"))]
        let (mut voice_track, occlusion_filter) = add_voice_track(&mut bus.track)?;
        let mut handle = voice_track
            .play(data)
            .map_err(|error| format!("Kira could not start sound {:?}: {error}", sound))?;
        if options.looping {
            handle.set_loop_region(0.0..);
        }
        handle.set_panning(playback_output_pan(&options), Tween::default());
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
                custom_bus: options.custom_bus,
                priority: options.priority,
                sequence,
                retained_stream_bytes: 0,
                is_streaming: false,
                replay: Some(ReplayVoiceState {
                    source: ReplayVoiceSource::Sound(sound),
                    options,
                    doppler,
                    acoustics: None,
                    paused: false,
                }),
                #[cfg(feature = "steam-audio")]
                binaural,
                #[cfg(feature = "steam-audio")]
                direct_acoustics,
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
        let (route_bus, _custom_bus, bus) = self.resolve_output_track(options, AudioBus::Music)?;
        #[cfg(feature = "steam-audio")]
        let (mut voice_track, occlusion_filter, binaural, direct_acoustics) =
            add_voice_track(&mut bus.track, options.binaural_direction)?;
        #[cfg(not(feature = "steam-audio"))]
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
        handle.set_panning(playback_output_pan(&options), Tween::default());
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
                custom_bus: options.custom_bus,
                priority: options.priority,
                sequence,
                retained_stream_bytes: 0,
                is_streaming: true,
                replay: None,
                #[cfg(feature = "steam-audio")]
                binaural,
                #[cfg(feature = "steam-audio")]
                direct_acoustics,
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
        let (route_bus, _custom_bus, bus) = self.resolve_output_track(options, AudioBus::Music)?;
        #[cfg(feature = "steam-audio")]
        let (mut voice_track, occlusion_filter, binaural, direct_acoustics) =
            add_voice_track(&mut bus.track, options.binaural_direction)?;
        #[cfg(not(feature = "steam-audio"))]
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
        handle.set_panning(playback_output_pan(&options), Tween::default());
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
                custom_bus: options.custom_bus,
                priority: options.priority,
                sequence,
                retained_stream_bytes: source_bytes,
                is_streaming: true,
                replay: None,
                #[cfg(feature = "steam-audio")]
                binaural,
                #[cfg(feature = "steam-audio")]
                direct_acoustics,
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
                if let Some(replay) = v.replay.as_mut() {
                    replay.paused = true;
                }
            })
            .is_some()
    }
    pub fn resume_voice(&mut self, id: VoiceId) -> bool {
        self.voices
            .get_mut(&id)
            .map(|v| {
                v.handle.resume(Tween::default());
                if let Some(replay) = v.replay.as_mut() {
                    replay.paused = false;
                }
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
                let gain = gain.clamp(0.0, 4.0);
                voice.handle.set_gain(gain, tween(fade_seconds));
                if let Some(replay) = voice.replay.as_mut() {
                    replay.options.gain = gain;
                    if let Some(acoustics) = replay.acoustics.as_mut() {
                        acoustics.gain = gain;
                    }
                }
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
                let rate = rate.clamp(0.25, 4.0);
                voice.handle.set_rate(rate, tween(fade_seconds));
                if let Some(replay) = voice.replay.as_mut() {
                    replay.options.playback_rate = rate;
                    if let Some(acoustics) = replay.acoustics.as_mut() {
                        acoustics.playback_rate = rate;
                    }
                }
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
                let pan = pan.clamp(-1.0, 1.0);
                voice
                    .handle
                    .set_pan(active_voice_output_pan(pan, voice), tween(fade_seconds));
                if let Some(replay) = voice.replay.as_mut() {
                    replay.options.pan = pan;
                    if let Some(acoustics) = replay.acoustics.as_mut() {
                        acoustics.pan = pan;
                    }
                }
            })
            .is_some()
    }
    /// Update the listener-local direction of a voice already created with a Steam Audio HRTF.
    #[cfg(feature = "steam-audio")]
    pub fn set_voice_binaural_direction(&mut self, id: VoiceId, direction: [f32; 3]) -> bool {
        let Some(voice) = self.voices.get_mut(&id) else {
            return false;
        };
        let Some(handle) = voice.binaural.as_ref() else {
            return false;
        };
        if handle.set_direction(direction).is_err() {
            return false;
        }
        if let Some(replay) = voice.replay.as_mut() {
            replay.options.binaural_direction = Some(direction);
        }
        true
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
                #[cfg(feature = "steam-audio")]
                binaural_direction: None,
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
            #[cfg(feature = "steam-audio")]
            binaural_direction,
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
        voice
            .handle
            .set_pan(active_voice_output_pan(pan, voice), tween);
        voice.handle.set_rate(playback_rate.clamp(0.25, 4.0), tween);
        #[cfg(feature = "steam-audio")]
        if let Some(direct) = voice.direct_acoustics.as_ref() {
            // Steam Audio expects 1 = clear / 0 = fully occluded; the engine API uses the inverse.
            let _ = direct.set_occlusion(occlusion);
        }
        #[cfg(feature = "steam-audio")]
        if let Some(direction) = binaural_direction {
            let Some(handle) = voice.binaural.as_ref() else {
                return false;
            };
            if handle.set_direction(direction).is_err() {
                return false;
            }
            if let Some(replay) = voice.replay.as_mut() {
                replay.options.binaural_direction = Some(direction);
            }
        }
        voice.occlusion_filter.set_cutoff(cutoff as f64, tween);
        voice.occlusion_filter.set_mix(Mix(1.0), tween);
        if let Some(replay) = voice.replay.as_mut() {
            replay.acoustics = Some(acoustics);
        }
        true
    }

    pub fn set_voice_looping(&mut self, id: VoiceId, looping: bool) -> bool {
        self.voices
            .get_mut(&id)
            .map(|voice| {
                voice.handle.set_looping(looping);
                if let Some(replay) = voice.replay.as_mut() {
                    replay.options.looping = looping;
                }
            })
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
        if let Some(track) = self.buses.get_mut(&bus) {
            track.track.pause(Tween::default());
            track.paused = true;
        }
    }
    pub fn resume_bus(&mut self, bus: AudioBus) {
        if let Some(track) = self.buses.get_mut(&bus) {
            track.track.resume(Tween::default());
            track.paused = false;
        }
    }
    pub fn voice_state(&mut self, id: VoiceId) -> Option<PlaybackStatus> {
        self.cleanup_finished();
        self.voices
            .get(&id)
            .map(|voice| playback_status(voice.handle.state(&voice._track)))
    }
    /// Poll the platform audio backend for the oldest stream/device error.
    /// The caller must run this on the Kira owner thread.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn take_device_error(&mut self) -> Option<String> {
        self._manager
            .backend_mut()
            .pop_error()
            .map(|error| format!("{error:?}"))
    }

    #[cfg(target_arch = "wasm32")]
    pub fn take_device_error(&mut self) -> Option<String> {
        None
    }

    pub fn set_device_recovery_state(
        &mut self,
        attempts: u64,
        recoveries: u64,
        last_error: Option<String>,
    ) {
        self.device_recovery_attempts = attempts;
        self.device_recoveries = recoveries;
        self.last_device_error = last_error;
    }

    /// Recreate Kira's platform backend and restore the mixer graph. Embedded sound-bank voices
    /// are restarted from their source IDs; file/cursor streams are completed because their
    /// original stream source is not owned by this backend in a replayable form.
    pub fn recover_output_device(&mut self) -> Result<(), String> {
        let bank = self.bank.clone();
        let graph = self.mixer_graph.clone();
        let builtin_state: Vec<(AudioBus, f32, f32, bool, bool, Vec<AudioEffect>)> = self
            .buses
            .iter()
            .map(|(bus, track)| {
                (
                    *bus,
                    track.gain,
                    track.zone_gain,
                    track.muted,
                    track.paused,
                    track.effect_chain.clone(),
                )
            })
            .collect();
        let custom_effects: Vec<(CustomBusId, Vec<AudioEffect>)> = self
            .custom_buses
            .iter()
            .map(|(id, track)| (*id, track.effect_chain.clone()))
            .collect();
        let paused_custom_buses: HashSet<CustomBusId> = self
            .custom_buses
            .iter()
            .filter_map(|(id, track)| track.paused.then_some(*id))
            .collect();
        let replayable: Vec<(VoiceId, ReplayVoiceState)> = self
            .voices
            .iter()
            .filter_map(|(id, voice)| {
                let active = !matches!(
                    voice.handle.state(&voice._track),
                    PlaybackState::Stopped | PlaybackState::Stopping
                );
                voice.replay.filter(|_| active).map(|state| (*id, state))
            })
            .collect();
        let replayable_ids: HashSet<VoiceId> = replayable.iter().map(|(id, _)| *id).collect();
        // Any voice not selected for replay (external stream, already stopped/stopping, or
        // missing source metadata) must be completed exactly once after replacing the backend.
        let mut completed: Vec<VoiceId> = self
            .voices
            .keys()
            .filter(|id| !replayable_ids.contains(id))
            .copied()
            .collect();
        completed.extend(self.retiring_voices.iter().map(|voice| voice.id));

        let cached_decode = StaticDecodeCache {
            decoded: self.decoded.clone(),
            decoded_static_pcm_bytes: self.decoded_static_pcm_bytes,
            peak_static_decode_bytes: self.peak_static_decode_bytes,
            decode_failures: self.decode_failures,
            last_decode_error: self.last_decode_error.clone(),
        };
        // StaticSoundData clones share Arc-backed PCM. Retaining this cache across the output
        // device rebuild avoids a second full decode and prevents recovery-time PCM peak spikes.
        let mut replacement = Self::new_with_decode_cache(bank, Some(cached_decode))?;
        let mut sends: Vec<AudioSendConfig> = graph.sends().cloned().collect();
        sends.sort_by_key(|send| send.id);
        for send in sends {
            replacement.create_audio_send(send)?;
        }

        let mut pending: Vec<CustomBusConfig> = graph.buses().cloned().collect();
        pending.sort_by_key(|bus| bus.id);
        let mut custom_bus_order = Vec::with_capacity(pending.len());
        while !pending.is_empty() {
            let mut progressed = false;
            let mut index = 0;
            while index < pending.len() {
                let parent_ready = match pending[index].parent {
                    MixerBusParent::BuiltIn(_) => true,
                    MixerBusParent::Custom(parent) => {
                        replacement.custom_buses.contains_key(&parent)
                    }
                };
                if parent_ready {
                    let config = pending.remove(index);
                    let id = config.id;
                    replacement.create_custom_bus(config)?;
                    custom_bus_order.push(id);
                    progressed = true;
                } else {
                    index += 1;
                }
            }
            if !progressed {
                return Err(
                    "cannot restore mixer graph: custom bus parent cycle or missing parent".into(),
                );
            }
        }
        for (id, effects) in custom_effects {
            replacement.set_custom_bus_effects(id, &effects, 0.0)?;
        }

        for (bus, gain, zone_gain, muted, paused, effects) in builtin_state {
            replacement.set_bus_gain(bus, gain, 0.0);
            replacement.set_bus_zone_gain(bus, zone_gain, 0.0);
            replacement.set_bus_muted(bus, muted, 0.0);
            replacement.set_bus_effects_faded(bus, &effects, 0.0);
            if paused {
                if let Some(track) = replacement.buses.get_mut(&bus) {
                    track.track.pause(Tween::default());
                    track.paused = true;
                }
            }
        }
        for id in custom_bus_order {
            if paused_custom_buses.contains(&id) {
                if let Some(track) = replacement.custom_buses.get_mut(&id) {
                    track.track.pause(Tween::default());
                    track.paused = true;
                }
            }
        }
        for (id, state) in replayable {
            let replay_result = match state.source {
                ReplayVoiceSource::Sound(sound) => {
                    replacement.play_voice(id, sound, state.options, state.doppler)
                }
                ReplayVoiceSource::Ambience(ambience) => {
                    let encoded = replacement
                        .bank
                        .ambience(ambience)
                        .map(|asset| asset.data.to_vec());
                    match encoded {
                        Some(bytes) => replacement
                            .play_streaming_bytes(id, bytes, state.options)
                            .map(|()| {
                                replacement.ambience_layers.insert(ambience, id);
                            }),
                        None => Err(format!("ambience {:?} is no longer registered", ambience)),
                    }
                }
            };
            match replay_result {
                Ok(()) => {
                    if let Some(voice) = replacement.voices.get_mut(&id) {
                        voice.replay = Some(state);
                    }
                    if state.paused {
                        replacement.pause_voice(id);
                    }
                    if let Some(acoustics) = state.acoustics {
                        replacement.update_voice_spatial_acoustics(id, acoustics);
                    }
                }
                Err(_) => completed.push(id),
            }
        }
        for id in self.completed_voices.drain(..) {
            replacement.record_completed_voice(id);
        }
        for id in completed {
            replacement.record_completed_voice(id);
        }
        replacement.dropped_completion_notifications = replacement
            .dropped_completion_notifications
            .saturating_add(self.dropped_completion_notifications);
        replacement.next_sequence = self.next_sequence;
        replacement.next_internal_voice = self.next_internal_voice;
        replacement.device_recovery_attempts = self.device_recovery_attempts;
        replacement.device_recoveries = self.device_recoveries;
        replacement.last_device_error = self.last_device_error.clone();
        *self = replacement;
        Ok(())
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
            peak_static_decode_bytes: self.peak_static_decode_bytes,
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
            custom_mixer_buses: self.custom_buses.len(),
            mixer_send_tracks: self.send_tracks.len(),
            device_recovery_attempts: self.device_recovery_attempts,
            device_recoveries: self.device_recoveries,
            last_device_error: self.last_device_error.clone(),
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
        if let Some(active) = self.voices.get_mut(&voice) {
            active.replay = Some(ReplayVoiceState {
                source: ReplayVoiceSource::Ambience(id),
                options,
                doppler: 1.0,
                acoustics: None,
                paused: false,
            });
        }
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
    let mut builder = TrackBuilder::new().sub_track_capacity(MAX_CUSTOM_BUSES + 8);
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
    let eq = [
        builder.add_effect(EqFilterBuilder::new(EqFilterKind::Bell, 100.0, 0.0, 1.0)),
        builder.add_effect(EqFilterBuilder::new(EqFilterKind::Bell, 500.0, 0.0, 1.0)),
        builder.add_effect(EqFilterBuilder::new(EqFilterKind::Bell, 2_000.0, 0.0, 1.0)),
        builder.add_effect(EqFilterBuilder::new(EqFilterKind::Bell, 8_000.0, 0.0, 1.0)),
    ];
    let delay = builder.add_effect(RealtimeDelayBuilder::new(0.25, -12.0, 0.0));
    let distortion = builder.add_effect(
        DistortionBuilder::new()
            .kind(DistortionKind::SoftClip)
            .drive(0.0)
            .mix(Mix(0.0)),
    );
    let convolution = builder.add_effect(RealtimeConvolutionBuilder::new(0.8, 0.0));
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
        eq,
        delay,
        distortion,
        convolution,
        _peak_protection: peak_protection,
        gain: 1.0,
        zone_gain: 1.0,
        muted: false,
        paused: false,
        effect_chain: Vec::new(),
    })
}

fn add_child_bus_track(
    parent: &mut TrackHandle,
    sends: &[(SendTrackId, f32)],
) -> Result<BusTrack, String> {
    // Voice stealing overlaps the replacement with the retiring voice until its fade ends.
    let mut builder = TrackBuilder::new().sub_track_capacity(MAX_CUSTOM_BUSES + (MAX_VOICES * 2));
    for (send_id, gain) in sends {
        builder = builder.with_send(*send_id, gain_to_db(*gain));
    }
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
    let eq = [
        builder.add_effect(EqFilterBuilder::new(EqFilterKind::Bell, 100.0, 0.0, 1.0)),
        builder.add_effect(EqFilterBuilder::new(EqFilterKind::Bell, 500.0, 0.0, 1.0)),
        builder.add_effect(EqFilterBuilder::new(EqFilterKind::Bell, 2_000.0, 0.0, 1.0)),
        builder.add_effect(EqFilterBuilder::new(EqFilterKind::Bell, 8_000.0, 0.0, 1.0)),
    ];
    let delay = builder.add_effect(RealtimeDelayBuilder::new(0.25, -12.0, 0.0));
    let distortion = builder.add_effect(
        DistortionBuilder::new()
            .kind(DistortionKind::SoftClip)
            .drive(0.0)
            .mix(Mix(0.0)),
    );
    let convolution = builder.add_effect(RealtimeConvolutionBuilder::new(0.8, 0.0));
    let track = parent
        .add_sub_track(builder)
        .map_err(|error| format!("could not create Kira child mixer track: {error}"))?;
    Ok(BusTrack {
        track,
        low_pass,
        high_pass,
        reverb,
        compressor,
        eq,
        delay,
        distortion,
        convolution,
        _peak_protection: None,
        gain: 1.0,
        zone_gain: 1.0,
        muted: false,
        paused: false,
        effect_chain: Vec::new(),
    })
}

#[inline]
fn effective_output_pan(pan: f32, binaural_active: bool) -> f32 {
    if binaural_active {
        0.0
    } else if pan.is_finite() {
        pan.clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

fn playback_output_pan(options: &PlaybackOptions) -> f32 {
    #[cfg(feature = "steam-audio")]
    {
        effective_output_pan(options.pan, options.binaural_direction.is_some())
    }

    #[cfg(not(feature = "steam-audio"))]
    {
        effective_output_pan(options.pan, false)
    }
}

fn active_voice_output_pan(pan: f32, _voice: &ActiveVoice) -> f32 {
    #[cfg(feature = "steam-audio")]
    {
        effective_output_pan(pan, _voice.binaural.is_some())
    }

    #[cfg(not(feature = "steam-audio"))]
    {
        effective_output_pan(pan, false)
    }
}

#[cfg(test)]
mod binaural_pan_regression_tests {
    use super::effective_output_pan;

    #[test]
    fn binaural_processing_owns_spatial_direction() {
        for pan in [-1.0, -0.5, 0.0, 0.5, 1.0] {
            assert_eq!(effective_output_pan(pan, true), 0.0);
        }
    }

    #[test]
    fn ordinary_voices_preserve_stereo_pan() {
        assert_eq!(effective_output_pan(-0.75, false), -0.75);
        assert_eq!(effective_output_pan(0.65, false), 0.65);
    }

    #[test]
    fn ordinary_pan_is_clamped_and_sanitized() {
        assert_eq!(effective_output_pan(-2.0, false), -1.0);
        assert_eq!(effective_output_pan(2.0, false), 1.0);
        assert_eq!(effective_output_pan(f32::NAN, false), 0.0);
    }
}
#[cfg(feature = "steam-audio")]
fn add_voice_track(
    parent: &mut TrackHandle,
    binaural_direction: Option<[f32; 3]>,
) -> Result<
    (
        TrackHandle,
        FilterHandle,
        Option<SteamAudioBinauralHandle>,
        Option<SteamAudioDirectEffectHandle>,
    ),
    String,
> {
    let mut builder = TrackBuilder::new();
    let direct = SteamAudioDirectEffectBuilder::new().map_err(|error| {
        format!("could not initialize Steam Audio direct effect for voice: {error}")
    })?;
    let direct_acoustics = Some(builder.add_effect(direct));
    let occlusion_filter = builder.add_effect(
        FilterBuilder::new()
            .mode(FilterMode::LowPass)
            .cutoff(20_000.0)
            .mix(Mix(1.0)),
    );
    let binaural = match binaural_direction {
        Some(direction) => {
            let effect = SteamAudioBinauralEffectBuilder::new(direction).map_err(|error| {
                format!("could not initialize Steam Audio HRTF for voice: {error}")
            })?;
            Some(builder.add_effect(effect))
        }
        None => None,
    };
    let track = parent
        .add_sub_track(builder)
        .map_err(|error| format!("could not create Kira voice route: {error}"))?;
    Ok((track, occlusion_filter, binaural, direct_acoustics))
}

#[cfg(not(feature = "steam-audio"))]
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
    fn realtime_delay_emits_a_delayed_impulse_without_dry_duplication() {
        let info = kira::info::MockInfoBuilder::new().build();
        let (mut effect, _handle) = RealtimeDelayBuilder::new(0.01, -60.0, 1.0).build();
        effect.init(1_000, 128);
        let mut frames = vec![Frame::ZERO; 32];
        frames[0] = Frame::new(1.0, 0.5);

        effect.process(&mut frames, 0.001, &info);

        assert_eq!(frames[0].left, 0.0);
        assert_eq!(frames[0].right, 0.0);
        assert!((frames[10].left - 1.0).abs() < 1e-5);
        assert!((frames[10].right - 0.5).abs() < 1e-5);
        assert!(frames
            .iter()
            .all(|frame| frame.left.is_finite() && frame.right.is_finite()));
    }

    #[test]
    fn convolution_stage_renders_a_finite_decaying_impulse_response() {
        let info = kira::info::MockInfoBuilder::new().build();
        let (mut effect, _handle) = RealtimeConvolutionBuilder::new(0.1, 1.0).build();
        effect.init(1_000, 128);
        let mut frames = vec![Frame::ZERO; 128];
        frames[0] = Frame::new(1.0, 1.0);

        effect.process(&mut frames, 0.001, &info);

        assert_eq!(frames[0].left, 0.0);
        assert!(frames.iter().skip(1).any(|frame| frame.left.abs() > 1e-5));
        assert!(frames
            .iter()
            .all(|frame| frame.left.is_finite() && frame.right.is_finite()));
    }

    #[test]
    fn recovery_decode_cache_clones_share_the_original_pcm_allocation() {
        let encoded = include_bytes!("../examples/assets/tone.wav");
        let data = StaticSoundData::from_cursor(Cursor::new(encoded.to_vec()))
            .expect("bundled test WAV must decode");
        let cache = StaticDecodeCache {
            decoded: HashMap::from([(SoundId(1), data.clone())]),
            decoded_static_pcm_bytes: static_pcm_storage_bytes(&data),
            peak_static_decode_bytes: static_pcm_storage_bytes(&data),
            decode_failures: 0,
            last_decode_error: None,
        };

        let recovered = cache.clone();
        let original_frames = &cache.decoded.get(&SoundId(1)).unwrap().frames;
        let recovered_frames = &recovered.decoded.get(&SoundId(1)).unwrap().frames;
        assert!(Arc::ptr_eq(original_frames, recovered_frames));
        assert_eq!(
            recovered.decoded_static_pcm_bytes,
            cache.decoded_static_pcm_bytes
        );
    }

    #[test]
    fn stream_byte_limits_are_explicit_and_finite() {
        assert_eq!(MAX_ENCODED_STREAM_BYTES, 64 * 1024 * 1024);
        assert_eq!(MAX_STATIC_SOUND_BYTES, 16 * 1024 * 1024);
        assert_eq!(MAX_STATIC_BANK_BYTES, 64 * 1024 * 1024);
        assert_eq!(MAX_STATIC_SOUND_PCM_BYTES, 64 * 1024 * 1024);
        assert_eq!(MAX_STATIC_BANK_PCM_BYTES, 128 * 1024 * 1024);
        assert_eq!(MAX_STATIC_DECODE_PEAK_BYTES, 128 * 1024 * 1024);
        assert_eq!(MAX_CUSTOM_BUSES, 16);
        const { assert!(MAX_TOTAL_SUB_TRACKS >= MAX_VOICES * 2) };
    }
}
