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
    fn set_effects(&mut self, _bus: AudioBus, _effects: &[AudioEffect]) {}
}

pub struct NullBackend;

/// Backend-neutral effect descriptors. They are safe to store in snapshots; backends may
/// implement supported effects and should ignore unsupported ones explicitly.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AudioEffect {
    LowPass { cutoff_hz: f32 },
    HighPass { cutoff_hz: f32 },
    Reverb { wet: f32, decay_seconds: f32 },
    Compressor { threshold_db: f32, ratio: f32 },
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
            Self::Reverb { wet, decay_seconds } => Self::Reverb {
                wet: finite(wet, 0.0, 1.0, 0.0),
                decay_seconds: finite(decay_seconds, 0.0, 10.0, 0.0),
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

#[derive(Clone, Debug, Default)]
pub struct MixerSnapshot {
    pub gains: HashMap<AudioBus, f32>,
    pub effects: HashMap<AudioBus, Vec<AudioEffect>>,
}

impl MixerSnapshot {
    pub fn set_gain(&mut self, bus: AudioBus, gain: f32) {
        self.gains.insert(bus, finite(gain, 0.0, 2.0, 1.0));
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
        self.bus_mutes.insert(bus, muted);
        self.update_ambience();
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
        self.effective_master_volume() * leaf_gain
    }

    /// Start or replace a snapshot gain transition. Calling this during a transition
    /// begins from the current gain, so transitions can be interrupted without a jump.
    /// The backend-neutral wrapper sends effect descriptors to the backend, which is responsible
    /// for smoothing supported parameters; Kira's threaded façade has an explicit-duration API.
    pub fn transition_to_snapshot(&mut self, snapshot: &MixerSnapshot, duration_seconds: f32) {
        self.transitions.clear();
        self.transition_elapsed = 0.0;
        self.transition_duration = if duration_seconds.is_finite() {
            duration_seconds.clamp(0.0, 3600.0)
        } else {
            0.0
        };
        for (bus, target) in &snapshot.gains {
            let target = finite(*target, 0.0, 2.0, 1.0);
            let start = self.bus_gain(*bus);
            if self.transition_duration == 0.0 || (start - target).abs() <= f32::EPSILON {
                self.set_bus_gain(*bus, target);
            } else {
                self.transitions.insert(*bus, (start, target));
            }
        }
        for (bus, effects) in &snapshot.effects {
            self.set_bus_effects(*bus, effects.clone());
        }
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
        let effects: Vec<_> = effects.into_iter().map(AudioEffect::sanitized).collect();
        self.backend.set_effects(bus, &effects);
        self.bus_effects.insert(bus, effects);
    }
    pub fn apply_snapshot(&mut self, snapshot: &MixerSnapshot) {
        for (bus, gain) in &snapshot.gains {
            self.set_bus_gain(*bus, *gain);
        }
        for (bus, effects) in &snapshot.effects {
            self.set_bus_effects(*bus, effects.clone());
        }
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
    use super::{AmbienceId, Audio, AudioBackend, SoundId};
    use crate::bank::{AmbienceAsset, SoundAsset, SoundBank};
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Default)]
    struct Calls {
        sounds: Vec<(SoundId, f32, f32)>,
        ambience: Vec<(Option<AmbienceId>, f32)>,
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
}
