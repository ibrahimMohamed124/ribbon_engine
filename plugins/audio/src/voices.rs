//! Backend-independent voice admission/control bookkeeping.
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VoiceId(pub u64);

/// Backend-independent state reported for a live Kira playback handle. Completed voices are
/// removed from the active registry and queried as `None` rather than keeping stale records.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaybackStatus {
    Playing,
    Pausing,
    Paused,
    Stopping,
    Stopped,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VoiceSpec {
    pub gain: f32,
    pub priority: u8,
    pub looping: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VoiceState {
    pub spec: VoiceSpec,
    pub started_at: u64,
    pub target_gain: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceAdmission {
    Started(VoiceId),
    Stolen { old: VoiceId, new: VoiceId },
    Rejected,
}

pub struct VoiceManager {
    max_voices: usize,
    next_id: u64,
    clock: u64,
    voices: HashMap<VoiceId, VoiceState>,
}

impl VoiceManager {
    pub fn new(max_voices: usize) -> Self {
        Self {
            max_voices: max_voices.max(1),
            next_id: 1,
            clock: 0,
            voices: HashMap::new(),
        }
    }

    pub fn active_count(&self) -> usize {
        self.voices.len()
    }

    pub fn contains(&self, id: VoiceId) -> bool {
        self.voices.contains_key(&id)
    }

    pub fn start(&mut self, spec: VoiceSpec) -> VoiceAdmission {
        let gain = if spec.gain.is_finite() {
            spec.gain.clamp(0.0, 4.0)
        } else {
            0.0
        };
        let spec = VoiceSpec { gain, ..spec };
        self.clock = self.clock.saturating_add(1);
        let id = VoiceId(self.next_id);
        self.next_id = self.next_id.saturating_add(1).max(1);

        if self.voices.len() < self.max_voices {
            self.voices.insert(
                id,
                VoiceState {
                    spec,
                    started_at: self.clock,
                    target_gain: gain,
                },
            );
            return VoiceAdmission::Started(id);
        }

        let candidate = self
            .voices
            .iter()
            .min_by_key(|(_, voice)| (voice.spec.priority, voice.started_at))
            .map(|(voice_id, voice)| (*voice_id, voice.spec.priority));
        if let Some((old, old_priority)) = candidate {
            if spec.priority <= old_priority {
                return VoiceAdmission::Rejected;
            }
            self.voices.remove(&old);
            self.voices.insert(
                id,
                VoiceState {
                    spec,
                    started_at: self.clock,
                    target_gain: gain,
                },
            );
            VoiceAdmission::Stolen { old, new: id }
        } else {
            VoiceAdmission::Rejected
        }
    }

    pub fn stop(&mut self, id: VoiceId) -> bool {
        self.voices.remove(&id).is_some()
    }

    pub fn stop_all(&mut self) {
        self.voices.clear();
    }

    pub fn set_gain(&mut self, id: VoiceId, gain: f32) -> bool {
        if !gain.is_finite() {
            return false;
        }
        if let Some(voice) = self.voices.get_mut(&id) {
            voice.target_gain = gain.clamp(0.0, 4.0);
            voice.spec.gain = voice.target_gain;
            true
        } else {
            false
        }
    }

    pub fn state(&self, id: VoiceId) -> Option<VoiceState> {
        self.voices.get(&id).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn respects_voice_limit_and_priority_stealing() {
        let mut manager = VoiceManager::new(1);
        let first = match manager.start(VoiceSpec {
            gain: 1.0,
            priority: 1,
            looping: false,
        }) {
            VoiceAdmission::Started(id) => id,
            _ => panic!("first voice should start"),
        };
        assert_eq!(
            manager.start(VoiceSpec {
                gain: 1.0,
                priority: 1,
                looping: false
            }),
            VoiceAdmission::Rejected
        );
        let second = match manager.start(VoiceSpec {
            gain: 1.0,
            priority: 8,
            looping: false,
        }) {
            VoiceAdmission::Stolen { old, new } => {
                assert_eq!(old, first);
                new
            }
            _ => panic!("higher-priority voice should steal the old voice"),
        };
        assert!(!manager.contains(first));
        assert!(manager.contains(second));
    }

    #[test]
    fn sanitizes_gain_and_stops() {
        let mut manager = VoiceManager::new(2);
        let id = match manager.start(VoiceSpec {
            gain: f32::NAN,
            priority: 1,
            looping: false,
        }) {
            VoiceAdmission::Started(id) => id,
            _ => panic!("voice should start"),
        };
        assert_eq!(manager.state(id).unwrap().spec.gain, 0.0);
        assert!(manager.stop(id));
        assert!(!manager.stop(id));
    }
}
