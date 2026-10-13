//! Platform-independent sound identifiers, mixing, and backend interface.

/// Opaque sound identifier owned by the application using the engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SoundId(pub u32);

/// Opaque ambience identifier owned by the application using the engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AmbienceId(pub u32);

#[derive(Clone, Copy)]
pub struct SoundAsset {
    pub id: SoundId,
    pub data: &'static [u8],
    pub gain: f32,
    pub speed: f32,
    pub spatial: bool,
}

#[derive(Clone, Copy)]
pub struct AmbienceAsset {
    pub id: AmbienceId,
    pub data: &'static [u8],
    pub gain: f32,
}

pub struct SoundBank {
    sounds: Vec<SoundAsset>,
    ambiences: Vec<AmbienceAsset>,
}

impl SoundBank {
    pub fn new(sounds: Vec<SoundAsset>, ambiences: Vec<AmbienceAsset>) -> Self {
        Self { sounds, ambiences }
    }

    pub fn sound(&self, id: SoundId) -> Option<SoundAsset> {
        self.sounds.iter().find(|asset| asset.id == id).copied()
    }

    pub fn ambience(&self, id: AmbienceId) -> Option<AmbienceAsset> {
        self.ambiences.iter().find(|asset| asset.id == id).copied()
    }
}

pub trait AudioBackend {
    fn play(&mut self, sound: SoundId, volume: f32);

    fn play_spatial(&mut self, sound: SoundId, volume: f32, _pan: f32) {
        self.play(sound, volume);
    }

    fn set_ambience(&mut self, _ambience: Option<AmbienceId>, _volume: f32) {}
}

pub struct NullBackend;

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
}

impl Audio {
    pub fn new(backend: Box<dyn AudioBackend>, master_volume: f32) -> Self {
        Self {
            backend,
            master_volume: master_volume.clamp(0.0, 1.0),
            effects_volume: 1.0,
            ambient_volume: 1.0,
            muted: false,
            ambience: None,
        }
    }

    pub fn null(master_volume: f32) -> Self {
        Self::new(Box::new(NullBackend), master_volume)
    }

    pub fn volume(&self) -> f32 {
        self.master_volume
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.master_volume = volume.clamp(0.0, 1.0);
        self.update_ambience();
    }

    pub fn set_effects_volume(&mut self, volume: f32) {
        self.effects_volume = volume.clamp(0.0, 1.0);
    }

    pub fn set_ambient_volume(&mut self, volume: f32) {
        self.ambient_volume = volume.clamp(0.0, 1.0);
        self.update_ambience();
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        self.update_ambience();
    }

    fn effective_master_volume(&self) -> f32 {
        if self.muted {
            0.0
        } else {
            self.master_volume
        }
    }

    fn update_ambience(&mut self) {
        let (ambience, relative_volume) = self
            .ambience
            .map(|(ambience, volume)| (Some(ambience), volume))
            .unwrap_or((None, 0.0));
        self.backend.set_ambience(
            ambience,
            relative_volume * self.ambient_volume * self.effective_master_volume(),
        );
    }

    pub fn set_ambience(&mut self, ambience: Option<AmbienceId>, volume: f32) {
        self.ambience = ambience.map(|ambience| (ambience, volume.clamp(0.0, 1.0)));
        self.update_ambience();
    }

    pub fn play(&mut self, sound: SoundId) {
        let volume = self.effective_master_volume() * self.effects_volume;
        if volume > 0.0 {
            self.backend.play(sound, volume);
        }
    }

    pub fn play_spatial(&mut self, sound: SoundId, pan: f32) {
        let volume = self.effective_master_volume() * self.effects_volume;
        if volume > 0.0 {
            self.backend
                .play_spatial(sound, volume, pan.clamp(-1.0, 1.0));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AmbienceAsset, AmbienceId, Audio, AudioBackend, SoundAsset, SoundBank, SoundId};
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
