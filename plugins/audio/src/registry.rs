//! Named event registry and bounded runtime parameter store.
use std::collections::HashMap;

use crate::bank::{AmbienceId, SoundId};
use crate::voices::{VoiceCompletions, VoiceId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioEventKind {
    OneShot,
    Loop,
    Ambience,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AudioEventDef {
    pub sound: SoundId,
    pub kind: AudioEventKind,
    pub gain: f32,
    pub priority: u8,
    pub max_instances: u16,
    pub bus: AudioBus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AudioBus {
    Master,
    Music,
    Ambience,
    Sfx,
    Ui,
    Dialogue,
    Voice,
}

/// Variant-aware sound-event descriptor. Ranges are inclusive and sampled deterministically
/// from the seed supplied to [`AudioEventRuntime`]. Event cooldown and instance limits are game-clock
/// based so tests do not depend on wall-clock sleeps.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioEventDescriptor {
    pub variants: Vec<SoundId>,
    pub gain_min: f32,
    pub gain_max: f32,
    pub playback_rate_min: f32,
    pub playback_rate_max: f32,
    pub priority: u8,
    pub max_instances: u16,
    pub cooldown_seconds: f32,
    pub bus: AudioBus,
    pub looping: bool,
}

impl AudioEventDescriptor {
    pub fn validate(&self, known_sounds: &[SoundId]) -> Result<(), String> {
        if self.variants.is_empty() {
            return Err("audio event must register at least one sound variant".into());
        }
        if self.variants.len() > 256 {
            return Err("audio event may contain at most 256 variants".into());
        }
        let mut unique = std::collections::HashSet::with_capacity(self.variants.len());
        for sound in &self.variants {
            if !unique.insert(*sound) {
                return Err(format!(
                    "audio event contains duplicate variant ID {sound:?}"
                ));
            }
            if !known_sounds.contains(sound) {
                return Err(format!(
                    "audio event references sound ID {sound:?} missing from the sound bank"
                ));
            }
        }
        if !self.gain_min.is_finite()
            || !self.gain_max.is_finite()
            || self.gain_min < 0.0
            || self.gain_max > 4.0
            || self.gain_min > self.gain_max
        {
            return Err("audio event gain range must be finite, ordered, and within [0, 4]".into());
        }
        if !self.playback_rate_min.is_finite()
            || !self.playback_rate_max.is_finite()
            || self.playback_rate_min < 0.25
            || self.playback_rate_max > 4.0
            || self.playback_rate_min > self.playback_rate_max
        {
            return Err(
                "audio event playback-rate range must be finite, ordered, and within [0.25, 4]"
                    .into(),
            );
        }
        if self.max_instances == 0 {
            return Err("audio event max_instances must be greater than zero".into());
        }
        if !self.cooldown_seconds.is_finite() || !(0.0..=3600.0).contains(&self.cooldown_seconds) {
            return Err("audio event cooldown must be finite and within [0, 3600] seconds".into());
        }
        Ok(())
    }
}

/// Result of resolving an event prior to submitting it to a backend.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedAudioEvent {
    pub name: String,
    pub sound: SoundId,
    pub gain: f32,
    pub playback_rate: f32,
    pub priority: u8,
    pub bus: AudioBus,
    pub looping: bool,
}

#[derive(Default)]
pub struct AudioEventRegistry {
    by_name: HashMap<String, AudioEventDef>,
    variants_by_name: HashMap<String, AudioEventDescriptor>,
}

impl AudioEventRegistry {
    pub fn register(&mut self, name: impl Into<String>, def: AudioEventDef) -> Result<(), String> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err("audio event name cannot be empty".into());
        }
        if name.len() > 128 {
            return Err("audio event names are limited to 128 bytes".into());
        }
        if self.len() >= 4096 {
            return Err("audio event registry is limited to 4096 entries".into());
        }
        if !def.gain.is_finite() || !(0.0..=4.0).contains(&def.gain) {
            return Err(format!("invalid gain for audio event `{name}`"));
        }
        if def.max_instances == 0 {
            return Err(format!("event `{name}` must allow at least one instance"));
        }
        if self.by_name.contains_key(&name) || self.variants_by_name.contains_key(&name) {
            return Err(format!("duplicate audio event `{name}`"));
        }
        self.by_name.insert(name, def);
        Ok(())
    }

    /// Register an event with a validated variant list, gain/rate randomization, cooldown and
    /// concurrency budget. `known_sounds` is normally built from `SoundBank::sounds()`.
    pub fn register_variants(
        &mut self,
        name: impl Into<String>,
        definition: AudioEventDescriptor,
        known_sounds: &[SoundId],
    ) -> Result<(), String> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err("audio event name cannot be empty".into());
        }
        if name.len() > 128 {
            return Err("audio event names are limited to 128 bytes".into());
        }
        if self.len() >= 4096 {
            return Err("audio event registry is limited to 4096 entries".into());
        }
        if self.by_name.contains_key(&name) || self.variants_by_name.contains_key(&name) {
            return Err(format!("duplicate audio event `{name}`"));
        }
        definition.validate(known_sounds)?;
        self.variants_by_name.insert(name, definition);
        Ok(())
    }

    pub fn get_variant_definition(&self, name: &str) -> Option<&AudioEventDescriptor> {
        self.variants_by_name.get(name)
    }

    pub fn get(&self, name: &str) -> Option<AudioEventDef> {
        self.by_name.get(name).copied()
    }

    pub fn len(&self) -> usize {
        self.by_name.len() + self.variants_by_name.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty() && self.variants_by_name.is_empty()
    }
}

/// Deterministic runtime for event choice, cooldowns, and per-event concurrency limits.
/// Call [`finish`](Self::finish) when the corresponding voice finishes or is stopped.
pub struct AudioEventRuntime {
    rng_state: u64,
    last_played: HashMap<String, f64>,
    active_instances: HashMap<String, u16>,
    previous_variant: HashMap<String, SoundId>,
    /// Playback handles bound to the event slots they own. This lets completion notifications
    /// release exactly the corresponding slot instead of requiring callers to match names.
    voice_events: HashMap<VoiceId, VoiceEventBinding>,
}

#[derive(Clone, Debug)]
struct VoiceEventBinding {
    event_name: String,
    consecutive_missing_snapshots: u8,
}

impl AudioEventRuntime {
    pub fn new(seed: u64) -> Self {
        Self {
            rng_state: if seed == 0 {
                0x9E37_79B9_7F4A_7C15
            } else {
                seed
            },
            last_played: HashMap::new(),
            active_instances: HashMap::new(),
            previous_variant: HashMap::new(),
            voice_events: HashMap::new(),
        }
    }

    /// Resolve a named event against deterministic game time. Unknown events, active-instance
    /// saturation, invalid time, and cooldown rejection return an explanatory error.
    pub fn trigger(
        &mut self,
        registry: &AudioEventRegistry,
        name: &str,
        now_seconds: f64,
    ) -> Result<ResolvedAudioEvent, String> {
        if !now_seconds.is_finite() || now_seconds < 0.0 {
            return Err("audio event time must be finite and non-negative".into());
        }
        let definition = registry
            .variants_by_name
            .get(name)
            .ok_or_else(|| format!("unknown variant audio event `{name}`"))?;
        if self.active_instances.get(name).copied().unwrap_or(0) >= definition.max_instances {
            return Err(format!("audio event `{name}` reached its instance limit"));
        }
        if let Some(last) = self.last_played.get(name) {
            if now_seconds < *last || now_seconds - *last < definition.cooldown_seconds as f64 {
                return Err(format!("audio event `{name}` is on cooldown"));
            }
        }

        let last_variant = self.previous_variant.get(name).copied();
        let candidates: Vec<SoundId> = if definition.variants.len() > 1 {
            definition
                .variants
                .iter()
                .copied()
                .filter(|sound| Some(*sound) != last_variant)
                .collect()
        } else {
            definition.variants.clone()
        };
        let sound = candidates[self.next_u32() as usize % candidates.len()];
        let gain_t = self.next_unit();
        let rate_t = self.next_unit();
        let result = ResolvedAudioEvent {
            name: name.to_owned(),
            sound,
            gain: definition.gain_min + (definition.gain_max - definition.gain_min) * gain_t,
            playback_rate: definition.playback_rate_min
                + (definition.playback_rate_max - definition.playback_rate_min) * rate_t,
            priority: definition.priority,
            bus: definition.bus,
            looping: definition.looping,
        };
        self.last_played.insert(name.to_owned(), now_seconds);
        self.previous_variant.insert(name.to_owned(), sound);
        *self.active_instances.entry(name.to_owned()).or_insert(0) += 1;
        Ok(result)
    }

    /// Resolve and submit a registered event in one operation, binding the returned playback ID
    /// to its concurrency slot. Submission or binding failure releases the reserved slot; a voice
    /// that could not be bound is stopped rather than leaked.
    #[cfg(feature = "audio")]
    pub fn trigger_and_play(
        &mut self,
        registry: &AudioEventRegistry,
        name: &str,
        now_seconds: f64,
        sink: &crate::threaded::ThreadedAudioSink,
    ) -> Result<VoiceId, String> {
        let previous_rng = self.rng_state;
        let previous_last_played = self.last_played.get(name).copied();
        let previous_variant = self.previous_variant.get(name).copied();
        let event = self.trigger(registry, name, now_seconds)?;
        let voice = match sink.play_resolved_event(&event) {
            Ok(voice) => voice,
            Err(error) => {
                self.release_event_slot(name);
                self.rng_state = previous_rng;
                restore_map_value(&mut self.last_played, name, previous_last_played);
                restore_map_value(&mut self.previous_variant, name, previous_variant);
                return Err(error);
            }
        };
        if let Err(error) = self.bind_voice(name, voice) {
            let _ = sink.stop_voice(voice, 0.0);
            self.release_event_slot(name);
            self.rng_state = previous_rng;
            restore_map_value(&mut self.last_played, name, previous_last_played);
            restore_map_value(&mut self.previous_variant, name, previous_variant);
            return Err(error);
        }
        Ok(voice)
    }

    /// Associate a successfully submitted voice with a previously resolved event.
    /// A caller should bind immediately after `play_resolved_event` succeeds.
    pub fn bind_voice(&mut self, event_name: &str, voice: VoiceId) -> Result<(), String> {
        if self.active_instances.get(event_name).copied().unwrap_or(0) == 0 {
            return Err(format!(
                "audio event `{event_name}` has no reserved instance slot"
            ));
        }
        if self.voice_events.contains_key(&voice) {
            return Err(format!(
                "voice {voice:?} is already bound to an audio event"
            ));
        }
        if self.voice_events.len() >= 4096 {
            return Err("audio event voice-binding limit reached".into());
        }
        self.voice_events.insert(
            voice,
            VoiceEventBinding {
                event_name: event_name.to_owned(),
                consecutive_missing_snapshots: 0,
            },
        );
        Ok(())
    }

    /// Release the exact event slot associated with a completed/stopped/failed playback voice.
    pub fn finish_voice(&mut self, voice: VoiceId) -> bool {
        let Some(binding) = self.voice_events.remove(&voice) else {
            return false;
        };
        self.release_event_slot(&binding.event_name)
    }

    /// Consume one worker completion batch and release all matching event slots.
    /// Returns the number of slots actually released. Inspect
    /// `VoiceCompletions::dropped_notifications_total` and reconcile voice state if it is non-zero.
    pub fn finish_completions(&mut self, completions: &VoiceCompletions) -> usize {
        let mut released = completions
            .voices
            .iter()
            .filter(|voice| self.finish_voice(**voice))
            .count();

        // A bounded completion notification queue may overflow during a long period without
        // polling. The backend's active-ID snapshot makes missed completions recoverable without
        // retaining event slots indefinitely. Legacy sinks return `None` and remain event-driven.
        if let Some(active_voices) = &completions.active_voices {
            let active: std::collections::HashSet<VoiceId> =
                active_voices.iter().copied().collect();
            let mut finished = Vec::new();
            for (voice, binding) in &mut self.voice_events {
                if active.contains(voice) {
                    binding.consecutive_missing_snapshots = 0;
                } else {
                    binding.consecutive_missing_snapshots =
                        binding.consecutive_missing_snapshots.saturating_add(1);
                    // A completion snapshot may have been queued before a just-submitted Play
                    // command. Require two consecutive authoritative misses before reconciling
                    // an absent voice, while explicit completion notifications remain immediate.
                    if binding.consecutive_missing_snapshots >= 2 {
                        finished.push(*voice);
                    }
                }
            }
            for voice in finished {
                if self.finish_voice(voice) {
                    released += 1;
                }
            }
        }
        released
    }

    /// Release one instance slot by event name for legacy integrations.
    /// When voices are bound, the oldest stable voice ID is detached as well.
    pub fn finish(&mut self, event_name: &str) -> bool {
        if let Some(voice) = self
            .voice_events
            .iter()
            .filter_map(|(voice, binding)| (binding.event_name == event_name).then_some(*voice))
            .min_by_key(|voice| voice.0)
        {
            self.voice_events.remove(&voice);
        }
        self.release_event_slot(event_name)
    }

    fn release_event_slot(&mut self, event_name: &str) -> bool {
        let Some(active_count) = self.active_instances.get(event_name).copied() else {
            return false;
        };
        if active_count > 1 {
            if let Some(active) = self.active_instances.get_mut(event_name) {
                *active -= 1;
            }
        } else {
            self.active_instances.remove(event_name);
        }
        true
    }

    /// Number of voice-to-event bindings still waiting for a completion notification.
    pub fn bound_voice_count(&self) -> usize {
        self.voice_events.len()
    }

    pub fn active_instances(&self, event_name: &str) -> u16 {
        self.active_instances.get(event_name).copied().unwrap_or(0)
    }

    fn next_u32(&mut self) -> u32 {
        // xorshift64*: deterministic, compact and adequate for cosmetic variation (not security).
        let mut x = self.rng_state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng_state = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 32) as u32
    }

    fn next_unit(&mut self) -> f32 {
        self.next_u32() as f32 / u32::MAX as f32
    }
}

#[cfg(feature = "audio")]
fn restore_map_value<V>(map: &mut HashMap<String, V>, key: &str, value: Option<V>) {
    if let Some(value) = value {
        map.insert(key.to_owned(), value);
    } else {
        map.remove(key);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AudioParameter {
    pub value: f32,
    pub min: f32,
    pub max: f32,
}

impl AudioParameter {
    pub fn new(value: f32, min: f32, max: f32) -> Result<Self, String> {
        if !value.is_finite() || !min.is_finite() || !max.is_finite() || min > max {
            return Err("audio parameter bounds/value must be finite and min <= max".into());
        }
        Ok(Self {
            value: value.clamp(min, max),
            min,
            max,
        })
    }

    pub fn set(&mut self, value: f32) -> bool {
        if !value.is_finite() {
            return false;
        }
        self.value = value.clamp(self.min, self.max);
        true
    }
}

#[derive(Default)]
pub struct AudioParameters {
    values: HashMap<String, AudioParameter>,
}

impl AudioParameters {
    pub fn insert(
        &mut self,
        name: impl Into<String>,
        parameter: AudioParameter,
    ) -> Result<(), String> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err("parameter name cannot be empty".into());
        }
        if name.len() > 128 {
            return Err("parameter names are limited to 128 bytes".into());
        }
        if self.values.len() >= 256 {
            return Err("audio parameter registry is limited to 256 entries".into());
        }
        if self.values.contains_key(&name) {
            return Err(format!("duplicate audio parameter `{name}`"));
        }
        self.values.insert(name, parameter);
        Ok(())
    }

    pub fn set(&mut self, name: &str, value: f32) -> bool {
        self.values
            .get_mut(name)
            .is_some_and(|parameter| parameter.set(value))
    }

    pub fn get(&self, name: &str) -> Option<f32> {
        self.values.get(name).map(|parameter| parameter.value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AmbienceState {
    pub id: Option<AmbienceId>,
    pub gain: f32,
    pub fade_seconds: f32,
}

impl Default for AmbienceState {
    fn default() -> Self {
        Self {
            id: None,
            gain: 0.0,
            fade_seconds: 0.25,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_duplicate_and_invalid_defs() {
        let mut registry = AudioEventRegistry::default();
        let definition = AudioEventDef {
            sound: SoundId(1),
            kind: AudioEventKind::OneShot,
            gain: 1.0,
            priority: 1,
            max_instances: 2,
            bus: AudioBus::Sfx,
        };
        assert!(registry.register("hit", definition).is_ok());
        assert!(registry.register("hit", definition).is_err());
        assert!(registry
            .register(
                "bad",
                AudioEventDef {
                    gain: f32::NAN,
                    ..definition
                }
            )
            .is_err());
    }

    #[test]
    fn parameters_clamp_and_reject_nan() {
        let mut parameter = AudioParameter::new(0.5, 0.0, 1.0).unwrap();
        assert!(parameter.set(3.0));
        assert_eq!(parameter.value, 1.0);
        assert!(!parameter.set(f32::NAN));
    }

    #[test]
    fn parameter_registry_rejects_duplicates_without_overwriting_values() {
        let mut parameters = AudioParameters::default();
        parameters
            .insert(
                "horror_intensity",
                AudioParameter::new(0.25, 0.0, 1.0).unwrap(),
            )
            .unwrap();
        assert!(parameters
            .insert(
                "horror_intensity",
                AudioParameter::new(0.9, 0.0, 1.0).unwrap()
            )
            .is_err());
        assert_eq!(parameters.get("horror_intensity"), Some(0.25));
    }

    #[test]
    fn variant_events_are_seeded_non_repeating_and_respect_cooldown_and_instance_limits() {
        let known = [SoundId(1), SoundId(2), SoundId(3)];
        let definition = AudioEventDescriptor {
            variants: known.to_vec(),
            gain_min: 0.4,
            gain_max: 0.8,
            playback_rate_min: 0.9,
            playback_rate_max: 1.1,
            priority: 40,
            max_instances: 1,
            cooldown_seconds: 0.5,
            bus: AudioBus::Sfx,
            looping: false,
        };
        let mut registry = AudioEventRegistry::default();
        registry
            .register_variants("footstep", definition, &known)
            .unwrap();
        assert!(registry
            .register(
                "footstep",
                AudioEventDef {
                    sound: SoundId(1),
                    kind: AudioEventKind::OneShot,
                    gain: 1.0,
                    priority: 1,
                    max_instances: 1,
                    bus: AudioBus::Sfx,
                }
            )
            .is_err());

        let mut one = AudioEventRuntime::new(99);
        let first = one.trigger(&registry, "footstep", 1.0).unwrap();
        assert!((0.4..=0.8).contains(&first.gain));
        assert!((0.9..=1.1).contains(&first.playback_rate));
        assert!(
            one.trigger(&registry, "footstep", 2.0).is_err(),
            "instance limit must be enforced"
        );
        assert!(one.finish("footstep"));
        assert!(
            one.trigger(&registry, "footstep", 1.2).is_err(),
            "cooldown must be enforced"
        );
        let second = one.trigger(&registry, "footstep", 1.6).unwrap();
        assert_ne!(
            first.sound, second.sound,
            "multi-variant events should avoid immediate repeats"
        );

        let mut two = AudioEventRuntime::new(99);
        assert_eq!(
            first.sound,
            two.trigger(&registry, "footstep", 1.0).unwrap().sound
        );
    }

    #[test]
    fn variant_events_reject_unknown_sound_references_and_invalid_ranges() {
        let mut registry = AudioEventRegistry::default();
        let invalid = AudioEventDescriptor {
            variants: vec![SoundId(404)],
            gain_min: 1.0,
            gain_max: 0.5,
            playback_rate_min: 1.0,
            playback_rate_max: 1.0,
            priority: 0,
            max_instances: 1,
            cooldown_seconds: 0.0,
            bus: AudioBus::Sfx,
            looping: false,
        };
        assert!(registry
            .register_variants("bad", invalid, &[SoundId(1)])
            .is_err());
    }

    #[test]
    fn voice_completion_releases_the_matching_event_slot_once() {
        let known = [SoundId(11), SoundId(12)];
        let descriptor = AudioEventDescriptor {
            variants: known.to_vec(),
            gain_min: 1.0,
            gain_max: 1.0,
            playback_rate_min: 1.0,
            playback_rate_max: 1.0,
            priority: 10,
            max_instances: 2,
            cooldown_seconds: 0.0,
            bus: AudioBus::Sfx,
            looping: false,
        };
        let mut registry = AudioEventRegistry::default();
        registry
            .register_variants("impact", descriptor, &known)
            .unwrap();
        let mut runtime = AudioEventRuntime::new(5);
        runtime.trigger(&registry, "impact", 1.0).unwrap();
        runtime.bind_voice("impact", VoiceId(42)).unwrap();
        assert_eq!(runtime.active_instances("impact"), 1);
        assert_eq!(runtime.bound_voice_count(), 1);
        assert!(runtime.finish_voice(VoiceId(42)));
        assert!(!runtime.finish_voice(VoiceId(42)));
        assert_eq!(runtime.active_instances("impact"), 0);
        assert_eq!(runtime.bound_voice_count(), 0);
    }

    #[test]
    fn active_voice_snapshot_repairs_slots_after_completion_queue_overflow() {
        let known = [SoundId(21)];
        let descriptor = AudioEventDescriptor {
            variants: known.to_vec(),
            gain_min: 1.0,
            gain_max: 1.0,
            playback_rate_min: 1.0,
            playback_rate_max: 1.0,
            priority: 10,
            max_instances: 1,
            cooldown_seconds: 0.0,
            bus: AudioBus::Sfx,
            looping: false,
        };
        let mut registry = AudioEventRegistry::default();
        registry
            .register_variants("ambient_ping", descriptor, &known)
            .unwrap();
        let mut runtime = AudioEventRuntime::new(1);
        runtime.trigger(&registry, "ambient_ping", 0.0).unwrap();
        runtime.bind_voice("ambient_ping", VoiceId(99)).unwrap();

        let first_miss = runtime.finish_completions(&VoiceCompletions {
            voices: Vec::new(),
            dropped_notifications_total: 1,
            active_voices: Some(Vec::new()),
        });
        assert_eq!(
            first_miss, 0,
            "one stale snapshot must not race a queued Play"
        );
        let released = runtime.finish_completions(&VoiceCompletions {
            voices: Vec::new(),
            dropped_notifications_total: 1,
            active_voices: Some(Vec::new()),
        });
        assert_eq!(released, 1);
        assert_eq!(runtime.active_instances("ambient_ping"), 0);
        assert_eq!(runtime.bound_voice_count(), 0);
    }
}
