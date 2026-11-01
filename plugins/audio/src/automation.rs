//! Deterministic parameter-to-audio automation.
//!
//! Bindings are evaluated on the game thread using explicit delta time. The resulting command
//! batch is submitted to the bounded Kira worker queue in one operation, so a saturated queue
//! rejects the whole frame rather than a random prefix of its automation updates.

use std::collections::{HashMap, HashSet};

use crate::mixer::{validate_effect_chain, AudioEffect};
use crate::registry::{AudioBus, AudioParameters};
use crate::voices::VoiceId;

const MAX_AUTOMATION_BINDINGS: usize = 256;
const MAX_EFFECTS_PER_BUS: usize = 8;
const VALUE_EPSILON: f32 = 0.0001;

/// A controllable parameter target. Each target may be bound once to avoid order-dependent writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AutomationTarget {
    BusGain(AudioBus),
    VoiceGain(VoiceId),
    VoicePlaybackRate(VoiceId),
    VoicePan(VoiceId),
    BusLowPassCutoff(AudioBus),
    BusHighPassCutoff(AudioBus),
    BusReverbWet(AudioBus),
}

/// A normalized mapping from one registered audio parameter to a runtime control.
#[derive(Clone, Debug, PartialEq)]
pub struct AutomationBinding {
    pub parameter: String,
    pub target: AutomationTarget,
    pub input_min: f32,
    pub input_max: f32,
    pub output_min: f32,
    pub output_max: f32,
    /// Exponential smoothing time constant. Zero applies the target immediately.
    pub smoothing_seconds: f32,
}

impl AutomationBinding {
    pub fn validate(&self, parameters: &AudioParameters) -> Result<(), String> {
        if self.parameter.trim().is_empty() || self.parameter.len() > 128 {
            return Err("automation parameter name must contain 1..=128 bytes".into());
        }
        if parameters.get(&self.parameter).is_none() {
            return Err(format!("unknown automation parameter `{}`", self.parameter));
        }
        if !self.input_min.is_finite()
            || !self.input_max.is_finite()
            || self.input_min >= self.input_max
            || !self.output_min.is_finite()
            || !self.output_max.is_finite()
            || !self.smoothing_seconds.is_finite()
            || !(0.0..=60.0).contains(&self.smoothing_seconds)
        {
            return Err("automation ranges and smoothing must be finite and ordered".into());
        }
        let (minimum, maximum) = target_limits(self.target);
        if self.output_min < minimum
            || self.output_min > maximum
            || self.output_max < minimum
            || self.output_max > maximum
        {
            return Err(format!(
                "automation output for {:?} must stay within [{minimum}, {maximum}]",
                self.target
            ));
        }
        Ok(())
    }
}

/// Backend-independent automation operation produced by one evaluation step.
#[derive(Clone, Debug, PartialEq)]
pub enum AutomationCommand {
    SetBusGain {
        bus: AudioBus,
        gain: f32,
    },
    SetVoiceGain {
        voice: VoiceId,
        gain: f32,
    },
    SetVoicePlaybackRate {
        voice: VoiceId,
        rate: f32,
    },
    SetVoicePan {
        voice: VoiceId,
        pan: f32,
    },
    SetBusEffects {
        bus: AudioBus,
        effects: Vec<AudioEffect>,
    },
}

impl AutomationCommand {
    pub fn validate(&self) -> Result<(), String> {
        let valid = match self {
            Self::SetBusGain { gain, .. } => gain.is_finite() && (0.0..=2.0).contains(gain),
            Self::SetVoiceGain { gain, .. } => gain.is_finite() && (0.0..=4.0).contains(gain),
            Self::SetVoicePlaybackRate { rate, .. } => {
                rate.is_finite() && (0.25..=4.0).contains(rate)
            }
            Self::SetVoicePan { pan, .. } => pan.is_finite() && (-1.0..=1.0).contains(pan),
            Self::SetBusEffects { effects, .. } => validate_effect_chain(effects).is_ok(),
        };
        if valid {
            Ok(())
        } else {
            Err(format!("invalid audio automation command: {self:?}"))
        }
    }
}

/// Runtime automation registry with deterministic interpolation and bounded state.
#[derive(Clone)]
pub struct AudioAutomation {
    bindings: Vec<AutomationBinding>,
    targets: HashSet<AutomationTarget>,
    initialized: HashSet<AutomationTarget>,
    values: HashMap<AutomationTarget, f32>,
    bus_effects: HashMap<AudioBus, Vec<AudioEffect>>,
}

impl Default for AudioAutomation {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioAutomation {
    pub fn new() -> Self {
        Self {
            bindings: Vec::new(),
            targets: HashSet::new(),
            initialized: HashSet::new(),
            values: HashMap::new(),
            bus_effects: HashMap::new(),
        }
    }

    /// Set the known base effect chain for a bus before binding effect parameters.
    pub fn set_base_effects(
        &mut self,
        bus: AudioBus,
        effects: Vec<AudioEffect>,
    ) -> Result<(), String> {
        validate_effect_chain(&effects)
            .map_err(|error| format!("base effect chain for {bus:?} is invalid: {error}"))?;
        self.bus_effects.insert(bus, effects);
        Ok(())
    }

    pub fn bind(
        &mut self,
        binding: AutomationBinding,
        parameters: &AudioParameters,
    ) -> Result<(), String> {
        binding.validate(parameters)?;
        if self.bindings.len() >= MAX_AUTOMATION_BINDINGS {
            return Err(format!(
                "audio automation is limited to {MAX_AUTOMATION_BINDINGS} bindings"
            ));
        }
        if !self.targets.insert(binding.target) {
            return Err(format!(
                "automation target {:?} is already bound",
                binding.target
            ));
        }
        self.values.insert(binding.target, binding.output_min);
        self.bindings.push(binding);
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    /// Evaluate bindings using caller-supplied frame time. Negative/non-finite time is rejected.
    /// Output ranges may be ascending or descending (for example, increasing threat lowers a
    /// filter cutoff), while all values remain within the target's legal range.
    pub fn evaluate(
        &mut self,
        parameters: &AudioParameters,
        delta_seconds: f32,
    ) -> Result<Vec<AutomationCommand>, String> {
        // Evaluation updates smoothing and effect-chain state. Roll it back on any validation
        // error so a caller may correct the parameter registry and retry without a hidden jump.
        let checkpoint = self.clone();
        match self.evaluate_inner(parameters, delta_seconds) {
            Ok(commands) => Ok(commands),
            Err(error) => {
                *self = checkpoint;
                Err(error)
            }
        }
    }

    fn evaluate_inner(
        &mut self,
        parameters: &AudioParameters,
        delta_seconds: f32,
    ) -> Result<Vec<AutomationCommand>, String> {
        if !delta_seconds.is_finite() || delta_seconds < 0.0 {
            return Err("automation delta time must be finite and non-negative".into());
        }
        let dt = delta_seconds.min(1.0);
        let mut commands = Vec::with_capacity(self.bindings.len());
        let mut changed_buses = HashSet::new();

        for binding in &self.bindings {
            let input = parameters.get(&binding.parameter).ok_or_else(|| {
                format!(
                    "automation parameter `{}` is no longer registered",
                    binding.parameter
                )
            })?;
            let t = ((input - binding.input_min) / (binding.input_max - binding.input_min))
                .clamp(0.0, 1.0);
            let target = binding.output_min + (binding.output_max - binding.output_min) * t;
            let previous = self.values.get(&binding.target).copied().unwrap_or(target);
            let first_update = self.initialized.insert(binding.target);
            let current = if binding.smoothing_seconds <= f32::EPSILON || dt == 0.0 {
                if binding.smoothing_seconds <= f32::EPSILON {
                    target
                } else {
                    previous
                }
            } else {
                let alpha = 1.0 - (-dt / binding.smoothing_seconds).exp();
                previous + (target - previous) * alpha
            };
            self.values.insert(binding.target, current);
            if !first_update && (current - previous).abs() <= VALUE_EPSILON {
                continue;
            }

            match binding.target {
                AutomationTarget::BusGain(bus) => {
                    commands.push(AutomationCommand::SetBusGain { bus, gain: current });
                }
                AutomationTarget::VoiceGain(voice) => {
                    commands.push(AutomationCommand::SetVoiceGain {
                        voice,
                        gain: current,
                    });
                }
                AutomationTarget::VoicePlaybackRate(voice) => {
                    commands.push(AutomationCommand::SetVoicePlaybackRate {
                        voice,
                        rate: current,
                    });
                }
                AutomationTarget::VoicePan(voice) => {
                    commands.push(AutomationCommand::SetVoicePan {
                        voice,
                        pan: current,
                    });
                }
                AutomationTarget::BusLowPassCutoff(bus) => {
                    upsert_bus_effect(
                        &mut self.bus_effects,
                        bus,
                        AudioEffect::LowPass { cutoff_hz: current },
                    )?;
                    changed_buses.insert(bus);
                }
                AutomationTarget::BusHighPassCutoff(bus) => {
                    upsert_bus_effect(
                        &mut self.bus_effects,
                        bus,
                        AudioEffect::HighPass { cutoff_hz: current },
                    )?;
                    changed_buses.insert(bus);
                }
                AutomationTarget::BusReverbWet(bus) => {
                    upsert_bus_effect(
                        &mut self.bus_effects,
                        bus,
                        AudioEffect::Reverb {
                            wet: current,
                            decay_seconds: 1.8,
                        },
                    )?;
                    changed_buses.insert(bus);
                }
            }
        }

        let mut buses: Vec<AudioBus> = changed_buses.into_iter().collect();
        buses.sort_by_key(|bus| format!("{bus:?}"));
        for bus in buses {
            commands.push(AutomationCommand::SetBusEffects {
                bus,
                effects: self.bus_effects.get(&bus).cloned().unwrap_or_default(),
            });
        }
        Ok(commands)
    }

    /// Evaluate and enqueue one automation frame on the Kira worker. The batch occupies a single
    /// bounded queue slot; if the queue is full, no partial frame is submitted.
    #[cfg(feature = "audio")]
    pub fn submit(
        &mut self,
        parameters: &AudioParameters,
        delta_seconds: f32,
        transition_seconds: f32,
        sink: &crate::threaded::ThreadedAudioSink,
    ) -> Result<usize, String> {
        let checkpoint = self.clone();
        let commands = self.evaluate(parameters, delta_seconds)?;
        let count = commands.len();
        match sink.apply_automation_frame(commands, transition_seconds) {
            Ok(()) => Ok(count),
            Err(error) => {
                // Queue rejection means the backend did not observe the frame. Restore the
                // smoothing state so the next accepted frame does not skip toward an update
                // which was never submitted.
                *self = checkpoint;
                Err(error)
            }
        }
    }
}

fn upsert_bus_effect(
    bus_effects: &mut HashMap<AudioBus, Vec<AudioEffect>>,
    bus: AudioBus,
    effect: AudioEffect,
) -> Result<(), String> {
    let chain = bus_effects.entry(bus).or_default();
    let key = effect_key(effect);
    if let Some(existing) = chain
        .iter_mut()
        .find(|current| effect_key(**current) == key)
    {
        *existing = effect.sanitized();
        return Ok(());
    }
    if chain.len() >= MAX_EFFECTS_PER_BUS {
        return Err(format!("bus {bus:?} effect chain reached its limit"));
    }
    chain.push(effect.sanitized());
    Ok(())
}

fn effect_key(effect: AudioEffect) -> u8 {
    match effect {
        AudioEffect::LowPass { .. } => 0,
        AudioEffect::HighPass { .. } => 1,
        AudioEffect::Eq { .. } => 2,
        AudioEffect::Reverb { .. } => 3,
        AudioEffect::Delay { .. } => 4,
        AudioEffect::Distortion { .. } => 5,
        AudioEffect::Convolution { .. } => 6,
        AudioEffect::Compressor { .. } => 7,
    }
}

fn target_limits(target: AutomationTarget) -> (f32, f32) {
    match target {
        AutomationTarget::BusGain(_) => (0.0, 2.0),
        AutomationTarget::VoiceGain(_) => (0.0, 4.0),
        AutomationTarget::VoicePlaybackRate(_) => (0.25, 4.0),
        AutomationTarget::VoicePan(_) => (-1.0, 1.0),
        AutomationTarget::BusLowPassCutoff(_) | AutomationTarget::BusHighPassCutoff(_) => {
            (20.0, 20_000.0)
        }
        AutomationTarget::BusReverbWet(_) => (0.0, 1.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::AudioParameter;

    #[test]
    fn evaluates_parameter_mappings_and_smooths_toward_targets() {
        let mut parameters = AudioParameters::default();
        parameters
            .insert(
                "horror_intensity",
                AudioParameter::new(0.0, 0.0, 1.0).unwrap(),
            )
            .unwrap();
        let mut automation = AudioAutomation::new();
        automation
            .bind(
                AutomationBinding {
                    parameter: "horror_intensity".into(),
                    target: AutomationTarget::BusLowPassCutoff(AudioBus::Music),
                    input_min: 0.0,
                    input_max: 1.0,
                    output_min: 18_000.0,
                    output_max: 900.0,
                    smoothing_seconds: 0.5,
                },
                &parameters,
            )
            .unwrap();
        assert!(automation.evaluate(&parameters, 0.1).unwrap().len() <= 1);
        parameters.set("horror_intensity", 1.0);
        let first = automation.evaluate(&parameters, 0.1).unwrap();
        let cutoff = match &first[0] {
            AutomationCommand::SetBusEffects { effects, .. } => match effects[0] {
                AudioEffect::LowPass { cutoff_hz } => cutoff_hz,
                _ => panic!("expected low-pass automation"),
            },
            _ => panic!("expected bus effect command"),
        };
        assert!(cutoff < 18_000.0 && cutoff > 900.0);
        automation.evaluate(&parameters, 1.0).unwrap();
        let final_step = automation.evaluate(&parameters, 1.0).unwrap();
        let cutoff = match &final_step[0] {
            AutomationCommand::SetBusEffects { effects, .. } => match effects[0] {
                AudioEffect::LowPass { cutoff_hz } => cutoff_hz,
                _ => panic!("expected low-pass automation"),
            },
            _ => panic!("expected bus effect command"),
        };
        assert!(cutoff < 2_000.0);
    }

    #[test]
    fn rejects_duplicate_targets_unknown_parameters_and_oversized_ranges() {
        let mut parameters = AudioParameters::default();
        parameters
            .insert("stress", AudioParameter::new(0.0, 0.0, 1.0).unwrap())
            .unwrap();
        let mut automation = AudioAutomation::new();
        let binding = AutomationBinding {
            parameter: "stress".into(),
            target: AutomationTarget::BusGain(AudioBus::Music),
            input_min: 0.0,
            input_max: 1.0,
            output_min: 0.0,
            output_max: 1.0,
            smoothing_seconds: 0.0,
        };
        automation.bind(binding.clone(), &parameters).unwrap();
        assert!(automation.bind(binding, &parameters).is_err());
        assert!(AutomationBinding {
            parameter: "missing".into(),
            target: AutomationTarget::BusGain(AudioBus::Sfx),
            input_min: 0.0,
            input_max: 1.0,
            output_min: 0.0,
            output_max: 1.0,
            smoothing_seconds: 0.0,
        }
        .validate(&parameters)
        .is_err());
        assert!(automation.evaluate(&parameters, f32::NAN).is_err());
    }
}
