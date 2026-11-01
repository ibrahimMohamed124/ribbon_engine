//! Platform audio mixing and backend interface.

use crate::bank::{AmbienceId, SoundId};
use crate::registry::AudioBus;
use std::collections::HashMap;

pub trait AudioBackend {
    fn play(&mut self, sound: SoundId, volume: f32);

    fn play_spatial(&mut self, sound: SoundId, volume: f32, _pan: f32) {
        self.play(sound, volume);
    }
    fn play_spatial_rate(&mut self, sound: SoundId, volume: f32, pan: f32, _doppler_ratio: f32) {
        self.play_spatial(sound, volume, pan);
    }

    fn set_ambience(&mut self, _ambience: Option<AmbienceId>, _volume: f32) {}
    fn stop_all(&mut self) {}
    fn set_bus_gain(&mut self, _bus: AudioBus, _gain: f32) {}
    fn set_bus_muted(&mut self, _bus: AudioBus, _muted: bool, _fade_seconds: f32) {}
    fn set_bus_zone_gain(&mut self, _bus: AudioBus, _gain: f32, _fade_seconds: f32) {}
    fn set_effects(&mut self, _bus: AudioBus, _effects: &[AudioEffect]) {}
    /// Apply an effect-chain change over the requested transition interval. Backends that do
    /// not support effect parameter tweening retain the immediate default behavior.
    fn set_effects_faded(&mut self, bus: AudioBus, effects: &[AudioEffect], _fade_seconds: f32) {
        self.set_effects(bus, effects);
    }
}

/// Bounded named collection of mixer states for gameplay transitions.
///
/// Snapshots are immutable after registration; replace one explicitly when updating a profile.
pub struct AudioSnapshotRegistry {
    snapshots: HashMap<String, MixerSnapshot>,
}

impl Default for AudioSnapshotRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioSnapshotRegistry {
    pub const MAX_SNAPSHOTS: usize = 64;
    pub const MAX_NAME_BYTES: usize = 64;

    pub fn new() -> Self {
        Self {
            snapshots: HashMap::new(),
        }
    }

    pub fn register(
        &mut self,
        name: impl Into<String>,
        snapshot: MixerSnapshot,
    ) -> Result<(), String> {
        let name = name.into();
        validate_snapshot_name(&name)?;
        snapshot.validate()?;
        if self.snapshots.contains_key(&name) {
            return Err(format!("audio snapshot `{name}` is already registered"));
        }
        if self.snapshots.len() >= Self::MAX_SNAPSHOTS {
            return Err(format!(
                "audio snapshot registry is limited to {} entries",
                Self::MAX_SNAPSHOTS
            ));
        }
        self.snapshots.insert(name, snapshot);
        Ok(())
    }

    pub fn replace(
        &mut self,
        name: impl Into<String>,
        snapshot: MixerSnapshot,
    ) -> Result<Option<MixerSnapshot>, String> {
        let name = name.into();
        validate_snapshot_name(&name)?;
        snapshot.validate()?;
        if !self.snapshots.contains_key(&name) && self.snapshots.len() >= Self::MAX_SNAPSHOTS {
            return Err(format!(
                "audio snapshot registry is limited to {} entries",
                Self::MAX_SNAPSHOTS
            ));
        }
        Ok(self.snapshots.insert(name, snapshot))
    }

    pub fn get(&self, name: &str) -> Option<&MixerSnapshot> {
        self.snapshots.get(name)
    }

    pub fn remove(&mut self, name: &str) -> Option<MixerSnapshot> {
        self.snapshots.remove(name)
    }

    pub fn len(&self) -> usize {
        self.snapshots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.snapshots.is_empty()
    }
}

fn validate_snapshot_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() || name.len() > AudioSnapshotRegistry::MAX_NAME_BYTES {
        return Err(format!(
            "audio snapshot name must contain 1..={} bytes",
            AudioSnapshotRegistry::MAX_NAME_BYTES
        ));
    }
    Ok(())
}

pub struct NullBackend;

/// Backend-neutral effect descriptors. They are safe to store in snapshots; backends may
/// implement supported effects and should ignore unsupported ones explicitly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioDistortionKind {
    SoftClip,
    HardClip,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AudioEffect {
    LowPass {
        cutoff_hz: f32,
    },
    HighPass {
        cutoff_hz: f32,
    },
    /// One of up to four independent bell-shaped parametric EQ bands per bus.
    Eq {
        frequency_hz: f32,
        gain_db: f32,
        q: f32,
    },
    Reverb {
        wet: f32,
        decay_seconds: f32,
    },
    Delay {
        delay_seconds: f32,
        feedback_db: f32,
        wet: f32,
    },
    Distortion {
        kind: AudioDistortionKind,
        drive_db: f32,
        wet: f32,
    },
    /// Bounded synthetic-room FIR convolution; `decay_seconds` controls the generated impulse.
    Convolution {
        decay_seconds: f32,
        wet: f32,
    },
    Compressor {
        threshold_db: f32,
        ratio: f32,
    },
}

impl AudioEffect {
    pub fn sanitized(self) -> Self {
        match self {
            Self::LowPass { cutoff_hz } => Self::LowPass {
                cutoff_hz: finite(cutoff_hz, 20.0, 20_000.0, 20_000.0),
            },
            Self::HighPass { cutoff_hz } => Self::HighPass {
                cutoff_hz: finite(cutoff_hz, 20.0, 20_000.0, 20.0),
            },
            Self::Eq {
                frequency_hz,
                gain_db,
                q,
            } => Self::Eq {
                frequency_hz: finite(frequency_hz, 20.0, 20_000.0, 1_000.0),
                gain_db: finite(gain_db, -24.0, 24.0, 0.0),
                q: finite(q, 0.1, 18.0, 1.0),
            },
            Self::Reverb { wet, decay_seconds } => Self::Reverb {
                wet: finite(wet, 0.0, 1.0, 0.0),
                decay_seconds: finite(decay_seconds, 0.0, 10.0, 0.0),
            },
            Self::Delay {
                delay_seconds,
                feedback_db,
                wet,
            } => Self::Delay {
                delay_seconds: finite(delay_seconds, 0.01, 1.0, 0.25),
                feedback_db: finite(feedback_db, -60.0, 0.0, -12.0),
                wet: finite(wet, 0.0, 1.0, 0.0),
            },
            Self::Distortion {
                kind,
                drive_db,
                wet,
            } => Self::Distortion {
                kind,
                drive_db: finite(drive_db, 0.0, 36.0, 0.0),
                wet: finite(wet, 0.0, 1.0, 0.0),
            },
            Self::Convolution { decay_seconds, wet } => Self::Convolution {
                decay_seconds: finite(decay_seconds, 0.05, 1.0, 0.8),
                wet: finite(wet, 0.0, 1.0, 0.0),
            },
            Self::Compressor {
                threshold_db,
                ratio,
            } => Self::Compressor {
                threshold_db: finite(threshold_db, -60.0, 0.0, -12.0),
                ratio: finite(ratio, 1.0, 20.0, 4.0),
            },
        }
    }
}

fn finite(value: f32, min: f32, max: f32, default: f32) -> f32 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        default
    }
}

/// Validate the representable backend effect chain. Kira currently owns one handle per effect
/// kind on a bus, so duplicates would otherwise be accepted while only the last value wins.
pub(crate) fn validate_effect_chain(effects: &[AudioEffect]) -> Result<(), String> {
    // One each of the seven fixed effect stages plus up to four independent parametric EQ bands.
    const MAX_EFFECTS_PER_BUS: usize = 11;
    const MAX_EQ_BANDS: usize = 4;
    if effects.len() > MAX_EFFECTS_PER_BUS {
        return Err(format!(
            "effect chains are limited to {MAX_EFFECTS_PER_BUS} effect stages"
        ));
    }
    let mut kinds = std::collections::HashSet::with_capacity(effects.len());
    let mut eq_bands = 0usize;
    for effect in effects {
        if *effect != effect.sanitized() {
            return Err("effect chain contains non-finite or out-of-range parameters".into());
        }
        let kind = match effect {
            AudioEffect::LowPass { .. } => 0,
            AudioEffect::HighPass { .. } => 1,
            AudioEffect::Eq { .. } => {
                eq_bands += 1;
                if eq_bands > MAX_EQ_BANDS {
                    return Err(format!(
                        "parametric EQ supports at most {MAX_EQ_BANDS} bands per bus"
                    ));
                }
                continue;
            }
            AudioEffect::Reverb { .. } => 3,
            AudioEffect::Delay { .. } => 4,
            AudioEffect::Distortion { .. } => 5,
            AudioEffect::Convolution { .. } => 6,
            AudioEffect::Compressor { .. } => 7,
        };
        if !kinds.insert(kind) {
            return Err("effect chains cannot contain duplicate non-EQ effect types".into());
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Default)]
pub struct MixerSnapshot {
    pub gains: HashMap<AudioBus, f32>,
    /// Optional per-bus mute overrides. Omitted buses retain their current state.
    pub mutes: HashMap<AudioBus, bool>,
    /// Acoustic-zone gain multipliers, independent of user-configured bus gains.
    pub zone_gains: HashMap<AudioBus, f32>,
    pub effects: HashMap<AudioBus, Vec<AudioEffect>>,
}

impl MixerSnapshot {
    /// Validate a snapshot even when callers have modified its public maps directly.
    pub fn validate(&self) -> Result<(), String> {
        for (bus, gain) in &self.gains {
            if !gain.is_finite() || !(0.0..=2.0).contains(gain) {
                return Err(format!("snapshot contains invalid gain for {bus:?}"));
            }
        }
        for (bus, gain) in &self.zone_gains {
            if !gain.is_finite() || !(0.0..=2.0).contains(gain) {
                return Err(format!("snapshot contains invalid zone gain for {bus:?}"));
            }
        }
        for (bus, effects) in &self.effects {
            validate_effect_chain(effects)
                .map_err(|error| format!("snapshot has invalid effects for {bus:?}: {error}"))?;
        }
        Ok(())
    }

    pub fn set_gain(&mut self, bus: AudioBus, gain: f32) {
        self.gains.insert(bus, finite(gain, 0.0, 2.0, 1.0));
    }

    pub fn set_muted(&mut self, bus: AudioBus, muted: bool) {
        self.mutes.insert(bus, muted);
    }

    pub fn set_zone_gain(&mut self, bus: AudioBus, gain: f32) {
        self.zone_gains.insert(bus, finite(gain, 0.0, 2.0, 1.0));
    }

    pub fn set_effects(&mut self, bus: AudioBus, effects: Vec<AudioEffect>) {
        self.effects.insert(
            bus,
            effects.into_iter().map(AudioEffect::sanitized).collect(),
        );
    }
}

impl AudioBackend for NullBackend {
    fn play(&mut self, _sound: SoundId, _volume: f32) {}
}

pub struct Audio {
    backend: Box<dyn AudioBackend>,
    master_volume: f32,
    effects_volume: f32,
    ambient_volume: f32,
    muted: bool,
    ambience: Option<(AmbienceId, f32)>,
    bus_gains: HashMap<AudioBus, f32>,
    bus_effects: HashMap<AudioBus, Vec<AudioEffect>>,
    bus_mutes: HashMap<AudioBus, bool>,
    bus_zone_gains: HashMap<AudioBus, f32>,
    transitions: HashMap<AudioBus, (f32, f32)>,
    transition_elapsed: f32,
    transition_duration: f32,
}

impl Audio {
    pub fn new(backend: Box<dyn AudioBackend>, master_volume: f32) -> Self {
        Self {
            backend,
            master_volume: finite(master_volume, 0.0, 1.0, 1.0),
            effects_volume: 1.0,
            ambient_volume: 1.0,
            muted: false,
            ambience: None,
            bus_gains: HashMap::new(),
            bus_effects: HashMap::new(),
            bus_mutes: HashMap::new(),
            bus_zone_gains: HashMap::new(),
            transitions: HashMap::new(),
            transition_elapsed: 0.0,
            transition_duration: 0.0,
        }
    }

    pub fn null(master_volume: f32) -> Self {
        Self::new(Box::new(NullBackend), master_volume)
    }

    /// Set a gain for a mixer bus. Gains are finite and clamped to [0, 2].
    pub fn set_bus_gain(&mut self, bus: AudioBus, gain: f32) {
        let gain = finite(gain, 0.0, 2.0, 1.0);
        self.bus_gains.insert(bus, gain);
        self.backend.set_bus_gain(bus, gain);
        self.update_ambience();
    }
    pub fn bus_gain(&self, bus: AudioBus) -> f32 {
        self.bus_gains.get(&bus).copied().unwrap_or(1.0)
    }

    /// Mute a bus without overwriting its configured gain. Unmuting restores that gain.
    pub fn set_bus_muted(&mut self, bus: AudioBus, muted: bool) {
        self.set_bus_muted_faded(bus, muted, 0.0);
    }

    pub fn set_bus_muted_faded(&mut self, bus: AudioBus, muted: bool, fade_seconds: f32) {
        self.bus_mutes.insert(bus, muted);
        self.backend
            .set_bus_muted(bus, muted, finite(fade_seconds, 0.0, 60.0, 0.0));
        self.update_ambience();
    }

    /// Change the acoustic-zone multiplier without modifying the configured user bus gain.
    pub fn set_bus_zone_gain(&mut self, bus: AudioBus, gain: f32) {
        self.set_bus_zone_gain_faded(bus, gain, 0.0);
    }

    pub fn set_bus_zone_gain_faded(&mut self, bus: AudioBus, gain: f32, fade_seconds: f32) {
        let gain = finite(gain, 0.0, 2.0, 1.0);
        self.bus_zone_gains.insert(bus, gain);
        self.backend
            .set_bus_zone_gain(bus, gain, finite(fade_seconds, 0.0, 60.0, 0.0));
        self.update_ambience();
    }

    pub fn bus_zone_gain(&self, bus: AudioBus) -> f32 {
        self.bus_zone_gains.get(&bus).copied().unwrap_or(1.0)
    }

    pub fn is_bus_muted(&self, bus: AudioBus) -> bool {
        self.bus_mutes.get(&bus).copied().unwrap_or(false)
    }

    /// The global master multiplier, including the legacy global mute and the Master-bus mute.
    fn effective_master_volume(&self) -> f32 {
        if self.muted || self.is_bus_muted(AudioBus::Master) {
            0.0
        } else {
            self.master_volume * self.bus_gain(AudioBus::Master)
        }
    }

    /// Return the configured gain after applying parent/master and leaf-bus mute states.
    /// The method is shared by sound and ambience paths so gains cannot drift between them.
    pub fn effective_bus_gain(&self, bus: AudioBus) -> f32 {
        if self.is_bus_muted(bus) {
            return 0.0;
        }
        let leaf_gain = if bus == AudioBus::Master {
            1.0
        } else {
            self.bus_gain(bus)
        };
        self.effective_master_volume() * leaf_gain * self.bus_zone_gain(bus)
    }

    /// Start or replace a snapshot gain transition. Calling this during a transition
    /// begins from the current gain, so transitions can be interrupted without a jump.
    /// The backend-neutral wrapper sends effect descriptors to the backend, which is responsible
    /// for smoothing supported parameters; Kira's threaded façade has an explicit-duration API.
    pub fn transition_to_snapshot(&mut self, snapshot: &MixerSnapshot, duration_seconds: f32) {
        let _ = self.try_transition_to_snapshot(snapshot, duration_seconds);
    }

    /// Checked variant of [`transition_to_snapshot`](Self::transition_to_snapshot). Invalid
    /// snapshots or timing values are rejected before any backend state is changed.
    pub fn try_transition_to_snapshot(
        &mut self,
        snapshot: &MixerSnapshot,
        duration_seconds: f32,
    ) -> Result<(), String> {
        snapshot.validate()?;
        if !duration_seconds.is_finite() || !(0.0..=60.0).contains(&duration_seconds) {
            return Err(
                "snapshot transition duration must be finite and within [0, 60] seconds".into(),
            );
        }
        self.transitions.clear();
        self.transition_elapsed = 0.0;
        self.transition_duration = duration_seconds;
        let fade_seconds = self.transition_duration;
        for (bus, target) in &snapshot.gains {
            let start = self.bus_gain(*bus);
            if self.transition_duration == 0.0 || (start - *target).abs() <= f32::EPSILON {
                self.set_bus_gain(*bus, *target);
            } else {
                self.transitions.insert(*bus, (start, *target));
            }
        }
        for (bus, effects) in &snapshot.effects {
            // The snapshot duration applies to DSP parameters as well as gain/mute state.
            // Validate the whole snapshot above before issuing any backend operations.
            self.backend.set_effects_faded(*bus, effects, fade_seconds);
            self.bus_effects.insert(*bus, effects.clone());
        }
        for (bus, muted) in &snapshot.mutes {
            self.set_bus_muted_faded(*bus, *muted, fade_seconds);
        }
        for (bus, gain) in &snapshot.zone_gains {
            self.set_bus_zone_gain_faded(*bus, *gain, fade_seconds);
        }
        Ok(())
    }

    /// Advance mixer transitions using the game's chosen time step. Invalid or negative
    /// deltas are ignored; large deltas safely complete the active transition.
    pub fn advance_transitions(&mut self, delta_seconds: f32) {
        if self.transitions.is_empty() || !delta_seconds.is_finite() || delta_seconds <= 0.0 {
            return;
        }
        self.transition_elapsed =
            (self.transition_elapsed + delta_seconds).min(self.transition_duration);
        let t = if self.transition_duration > 0.0 {
            (self.transition_elapsed / self.transition_duration).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let updates: Vec<_> = self
            .transitions
            .iter()
            .map(|(bus, (start, target))| (*bus, start + (target - start) * t))
            .collect();
        for (bus, gain) in updates {
            self.set_bus_gain(bus, gain);
        }
        if t >= 1.0 {
            self.transitions.clear();
        }
    }

    pub fn transition_active(&self) -> bool {
        !self.transitions.is_empty()
    }
    pub fn set_bus_effects(&mut self, bus: AudioBus, effects: Vec<AudioEffect>) {
        let _ = self.try_set_bus_effects(bus, effects);
    }

    /// Set a bus effect chain and return a validation error instead of silently accepting a
    /// chain that the current backend cannot represent.
    pub fn try_set_bus_effects(
        &mut self,
        bus: AudioBus,
        effects: Vec<AudioEffect>,
    ) -> Result<(), String> {
        validate_effect_chain(&effects)?;
        self.backend.set_effects(bus, &effects);
        self.bus_effects.insert(bus, effects);
        Ok(())
    }
    pub fn apply_snapshot(&mut self, snapshot: &MixerSnapshot) {
        let _ = self.try_apply_snapshot(snapshot);
    }

    /// Validate and apply the complete state before changing any mixer value.
    pub fn try_apply_snapshot(&mut self, snapshot: &MixerSnapshot) -> Result<(), String> {
        snapshot.validate()?;
        for (bus, gain) in &snapshot.gains {
            self.set_bus_gain(*bus, *gain);
        }
        for (bus, effects) in &snapshot.effects {
            self.try_set_bus_effects(*bus, effects.clone())?;
        }
        for (bus, muted) in &snapshot.mutes {
            self.set_bus_muted(*bus, *muted);
        }
        for (bus, gain) in &snapshot.zone_gains {
            self.set_bus_zone_gain(*bus, *gain);
        }
        Ok(())
    }
    pub fn stop_all(&mut self) {
        self.backend.stop_all();
    }

    pub fn volume(&self) -> f32 {
        self.master_volume
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.master_volume = finite(volume, 0.0, 1.0, 1.0);
        self.update_ambience();
    }

    pub fn set_effects_volume(&mut self, volume: f32) {
        self.effects_volume = finite(volume, 0.0, 1.0, 1.0);
    }

    pub fn set_ambient_volume(&mut self, volume: f32) {
        self.ambient_volume = finite(volume, 0.0, 1.0, 1.0);
        self.update_ambience();
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        self.update_ambience();
    }

    fn update_ambience(&mut self) {
        let (ambience, relative_volume) = self
            .ambience
            .map(|(ambience, volume)| (Some(ambience), volume))
            .unwrap_or((None, 0.0));
        self.backend.set_ambience(
            ambience,
            relative_volume * self.ambient_volume * self.effective_bus_gain(AudioBus::Ambience),
        );
    }

    pub fn set_ambience(&mut self, ambience: Option<AmbienceId>, volume: f32) {
        self.ambience = ambience.map(|ambience| (ambience, finite(volume, 0.0, 1.0, 0.0)));
        self.update_ambience();
    }

    pub fn play(&mut self, sound: SoundId) {
        let volume = self.effective_bus_gain(AudioBus::Sfx) * self.effects_volume;
        if volume > 0.0 {
            self.backend.play(sound, volume);
        }
    }

    pub fn play_spatial(&mut self, sound: SoundId, pan: f32) {
        let volume = self.effective_bus_gain(AudioBus::Sfx) * self.effects_volume;
        if volume > 0.0 {
            self.backend
                .play_spatial(sound, volume, pan.clamp(-1.0, 1.0));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{validate_effect_chain, AmbienceId, Audio, AudioBackend, AudioEffect, SoundId};
    use crate::bank::{AmbienceAsset, SoundAsset, SoundBank};
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Default)]
    struct Calls {
        sounds: Vec<(SoundId, f32, f32)>,
        ambience: Vec<(Option<AmbienceId>, f32)>,
        bus_mutes: Vec<(super::AudioBus, bool, f32)>,
        zone_gains: Vec<(super::AudioBus, f32, f32)>,
        effect_fades: Vec<(super::AudioBus, f32)>,
    }

    struct Recorder(Rc<RefCell<Calls>>);

    impl AudioBackend for Recorder {
        fn play(&mut self, sound: SoundId, volume: f32) {
            self.0.borrow_mut().sounds.push((sound, volume, 0.0));
        }

        fn play_spatial(&mut self, sound: SoundId, volume: f32, pan: f32) {
            self.0.borrow_mut().sounds.push((sound, volume, pan));
        }

        fn set_ambience(&mut self, ambience: Option<AmbienceId>, volume: f32) {
            self.0.borrow_mut().ambience.push((ambience, volume));
        }

        fn set_bus_muted(&mut self, bus: super::AudioBus, muted: bool, fade_seconds: f32) {
            self.0
                .borrow_mut()
                .bus_mutes
                .push((bus, muted, fade_seconds));
        }

        fn set_bus_zone_gain(&mut self, bus: super::AudioBus, gain: f32, fade_seconds: f32) {
            self.0
                .borrow_mut()
                .zone_gains
                .push((bus, gain, fade_seconds));
        }

        fn set_effects_faded(
            &mut self,
            bus: super::AudioBus,
            _effects: &[super::AudioEffect],
            fade_seconds: f32,
        ) {
            self.0.borrow_mut().effect_fades.push((bus, fade_seconds));
        }
    }

    #[test]
    fn bus_gains_and_snapshot_values_are_sanitized() {
        use super::{AudioBus, AudioEffect, MixerSnapshot};
        let mut audio = Audio::null(1.0);
        audio.set_bus_gain(AudioBus::Sfx, f32::NAN);
        assert_eq!(audio.bus_gain(AudioBus::Sfx), 1.0);
        let mut snapshot = MixerSnapshot::default();
        snapshot.set_gain(AudioBus::Sfx, 8.0);
        snapshot.set_effects(
            AudioBus::Ambience,
            vec![AudioEffect::Reverb {
                wet: f32::NAN,
                decay_seconds: 99.0,
            }],
        );
        assert_eq!(snapshot.gains.get(&AudioBus::Sfx), Some(&2.0));
        assert_eq!(
            snapshot.effects.get(&AudioBus::Ambience).unwrap()[0],
            AudioEffect::Reverb {
                wet: 0.0,
                decay_seconds: 10.0
            }
        );
        audio.apply_snapshot(&snapshot);
        assert_eq!(audio.bus_gain(AudioBus::Sfx), 2.0);
    }

    #[test]
    fn snapshot_gain_transitions_are_smooth_interruptible_and_deterministic() {
        use super::{AudioBus, MixerSnapshot};
        let mut audio = Audio::null(1.0);
        audio.set_bus_gain(AudioBus::Music, 0.0);
        let mut chase = MixerSnapshot::default();
        chase.set_gain(AudioBus::Music, 1.0);
        audio.transition_to_snapshot(&chase, 2.0);
        audio.advance_transitions(1.0);
        assert!((audio.bus_gain(AudioBus::Music) - 0.5).abs() < 1e-6);
        assert!(audio.transition_active());

        let mut pause = MixerSnapshot::default();
        pause.set_gain(AudioBus::Music, 0.2);
        audio.transition_to_snapshot(&pause, 1.0);
        audio.advance_transitions(0.5);
        assert!((audio.bus_gain(AudioBus::Music) - 0.35).abs() < 1e-6);
        audio.advance_transitions(0.5);
        assert!((audio.bus_gain(AudioBus::Music) - 0.2).abs() < 1e-6);
        assert!(!audio.transition_active());
    }

    #[test]
    fn bus_mute_preserves_gain_and_silences_that_bus_only() {
        use super::AudioBus;
        let calls = Rc::new(RefCell::new(Calls::default()));
        let mut audio = Audio::new(Box::new(Recorder(Rc::clone(&calls))), 1.0);
        audio.set_bus_gain(AudioBus::Sfx, 0.7);
        audio.set_bus_muted(AudioBus::Sfx, true);
        audio.play(SoundId(1));
        assert!(calls.borrow().sounds.is_empty());
        audio.set_bus_muted(AudioBus::Sfx, false);
        audio.play(SoundId(1));
        assert!((calls.borrow().sounds[0].1 - 0.7).abs() < 1e-6);
        assert_eq!(audio.bus_gain(AudioBus::Sfx), 0.7);
    }

    #[test]
    fn snapshot_restores_mutes_and_zone_gains_without_overwriting_user_gain() {
        use super::{AudioBus, MixerSnapshot};
        let calls = Rc::new(RefCell::new(Calls::default()));
        let mut audio = Audio::new(Box::new(Recorder(Rc::clone(&calls))), 1.0);
        audio.set_bus_gain(AudioBus::Ambience, 0.65);

        let mut snapshot = MixerSnapshot::default();
        snapshot.set_muted(AudioBus::Ambience, true);
        snapshot.set_zone_gain(AudioBus::Ambience, 0.25);
        audio.transition_to_snapshot(&snapshot, 0.4);

        assert!(audio.is_bus_muted(AudioBus::Ambience));
        assert_eq!(audio.bus_gain(AudioBus::Ambience), 0.65);
        assert_eq!(audio.bus_zone_gain(AudioBus::Ambience), 0.25);
        assert_eq!(audio.effective_bus_gain(AudioBus::Ambience), 0.0);
        let calls = calls.borrow();
        assert_eq!(
            calls.bus_mutes.last(),
            Some(&(AudioBus::Ambience, true, 0.4))
        );
        assert_eq!(
            calls.zone_gains.last(),
            Some(&(AudioBus::Ambience, 0.25, 0.4))
        );
    }

    #[test]
    fn snapshot_transition_duration_is_applied_to_effect_chains() {
        use super::{AudioBus, AudioEffect, MixerSnapshot};
        let calls = Rc::new(RefCell::new(Calls::default()));
        let mut audio = Audio::new(Box::new(Recorder(Rc::clone(&calls))), 1.0);
        let effects = vec![AudioEffect::LowPass { cutoff_hz: 1_200.0 }];
        let mut snapshot = MixerSnapshot::default();
        snapshot.set_effects(AudioBus::Music, effects.clone());

        audio.try_transition_to_snapshot(&snapshot, 1.75).unwrap();

        assert_eq!(calls.borrow().effect_fades, vec![(AudioBus::Music, 1.75)]);
        assert_eq!(audio.bus_effects.get(&AudioBus::Music), Some(&effects));
    }

    #[test]
    fn invalid_duplicate_effect_chain_is_rejected_without_replacing_last_good_chain() {
        use super::{AudioBus, AudioEffect};
        let mut audio = Audio::null(1.0);
        let valid = vec![AudioEffect::LowPass { cutoff_hz: 2_000.0 }];
        audio
            .try_set_bus_effects(AudioBus::Music, valid.clone())
            .unwrap();
        let invalid = vec![
            AudioEffect::LowPass { cutoff_hz: 1_000.0 },
            AudioEffect::LowPass { cutoff_hz: 500.0 },
        ];
        assert!(audio.try_set_bus_effects(AudioBus::Music, invalid).is_err());
        assert_eq!(audio.bus_effects.get(&AudioBus::Music), Some(&valid));
    }

    #[test]
    fn effect_chain_accepts_eq_delay_distortion_and_bounded_convolution() {
        let effects = vec![
            AudioEffect::LowPass { cutoff_hz: 8_000.0 },
            AudioEffect::HighPass { cutoff_hz: 40.0 },
            AudioEffect::Eq {
                frequency_hz: 100.0,
                gain_db: 2.0,
                q: 0.8,
            },
            AudioEffect::Eq {
                frequency_hz: 500.0,
                gain_db: -1.0,
                q: 1.2,
            },
            AudioEffect::Eq {
                frequency_hz: 2_000.0,
                gain_db: 1.5,
                q: 1.0,
            },
            AudioEffect::Eq {
                frequency_hz: 8_000.0,
                gain_db: -2.0,
                q: 1.4,
            },
            AudioEffect::Reverb {
                wet: 0.25,
                decay_seconds: 1.8,
            },
            AudioEffect::Delay {
                delay_seconds: 0.5,
                feedback_db: -9.0,
                wet: 0.2,
            },
            AudioEffect::Distortion {
                kind: super::AudioDistortionKind::SoftClip,
                drive_db: 8.0,
                wet: 0.1,
            },
            AudioEffect::Convolution {
                decay_seconds: 0.75,
                wet: 0.15,
            },
            AudioEffect::Compressor {
                threshold_db: -18.0,
                ratio: 3.0,
            },
        ];
        validate_effect_chain(&effects).unwrap();

        let mut too_many_eq = effects.clone();
        too_many_eq.push(AudioEffect::Eq {
            frequency_hz: 12_000.0,
            gain_db: 1.0,
            q: 1.0,
        });
        assert!(validate_effect_chain(&too_many_eq).is_err());

        let too_long_delay = vec![AudioEffect::Delay {
            delay_seconds: 1.5,
            feedback_db: -12.0,
            wet: 0.3,
        }];
        assert!(validate_effect_chain(&too_long_delay).is_err());
    }

    #[test]
    fn effective_bus_gain_multiplies_master_and_child_without_losing_state_on_mute() {
        use super::AudioBus;
        let mut audio = Audio::null(0.5);
        audio.set_bus_gain(AudioBus::Master, 0.8);
        audio.set_bus_gain(AudioBus::Sfx, 0.5);
        assert!((audio.effective_bus_gain(AudioBus::Sfx) - 0.2).abs() < 1e-6);
        audio.set_bus_muted(AudioBus::Sfx, true);
        assert_eq!(audio.effective_bus_gain(AudioBus::Sfx), 0.0);
        audio.set_bus_muted(AudioBus::Sfx, false);
        assert!((audio.effective_bus_gain(AudioBus::Sfx) - 0.2).abs() < 1e-6);
        audio.set_muted(true);
        assert_eq!(audio.effective_bus_gain(AudioBus::Sfx), 0.0);
        audio.set_muted(false);
        assert!((audio.effective_bus_gain(AudioBus::Sfx) - 0.2).abs() < 1e-6);
    }

    #[test]
    fn opaque_application_ids_work_in_banks_and_mixers() {
        let sound = SoundId(99);
        let ambience = AmbienceId(99);
        let bank = SoundBank::new(
            vec![SoundAsset {
                id: sound,
                data: b"custom",
                gain: 0.5,
                speed: 1.0,
                spatial: false,
            }],
            vec![AmbienceAsset {
                id: ambience,
                data: b"custom ambience",
                gain: 0.5,
            }],
        );
        assert_eq!(bank.sound(sound).unwrap().id, sound);
        assert_eq!(bank.ambience(ambience).unwrap().id, ambience);

        let calls = Rc::new(RefCell::new(Calls::default()));
        let mut audio = Audio::new(Box::new(Recorder(Rc::clone(&calls))), 0.8);
        audio.set_effects_volume(0.5);
        audio.play(sound);
        audio.play_spatial(sound, 4.0);
        audio.set_muted(true);
        audio.play(sound);
        audio.set_ambience(Some(ambience), 1.0);

        let calls = calls.borrow();
        assert_eq!(calls.sounds, vec![(sound, 0.4, 0.0), (sound, 0.4, 1.0)]);
        assert_eq!(calls.ambience.last(), Some(&(Some(ambience), 0.0)));
    }

    #[test]
    fn named_snapshot_registry_validates_names_values_and_replacement() {
        use super::{AudioBus, AudioSnapshotRegistry, MixerSnapshot};

        let mut registry = AudioSnapshotRegistry::new();
        let mut normal = MixerSnapshot::default();
        normal.set_gain(AudioBus::Music, 0.8);
        registry.register("Normal", normal).unwrap();
        assert_eq!(registry.len(), 1);
        assert!(registry
            .register("Normal", MixerSnapshot::default())
            .is_err());
        assert!(registry.register("  ", MixerSnapshot::default()).is_err());

        let mut invalid = MixerSnapshot::default();
        invalid.gains.insert(AudioBus::Sfx, f32::NAN);
        assert!(registry.register("Invalid", invalid).is_err());

        let mut chase = MixerSnapshot::default();
        chase.set_gain(AudioBus::Music, 0.3);
        assert!(registry.replace("Normal", chase).unwrap().is_some());
        assert_eq!(registry.get("Normal").unwrap().gains[&AudioBus::Music], 0.3);
        assert!(registry.get("missing").is_none());
    }

    #[test]
    fn parametric_eq_supports_four_independent_bands() {
        let bands = vec![
            AudioEffect::Eq {
                frequency_hz: 80.0,
                gain_db: -3.0,
                q: 0.8,
            },
            AudioEffect::Eq {
                frequency_hz: 250.0,
                gain_db: 2.0,
                q: 1.0,
            },
            AudioEffect::Eq {
                frequency_hz: 2_000.0,
                gain_db: 1.5,
                q: 1.2,
            },
            AudioEffect::Eq {
                frequency_hz: 8_000.0,
                gain_db: -2.0,
                q: 0.7,
            },
        ];
        assert!(validate_effect_chain(&bands).is_ok());
        let mut five = bands;
        five.push(AudioEffect::Eq {
            frequency_hz: 12_000.0,
            gain_db: 1.0,
            q: 1.0,
        });
        assert!(validate_effect_chain(&five).is_err());
    }
}
