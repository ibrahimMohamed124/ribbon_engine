//! Platform-independent sound identifiers, mixing, and backend interface.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SoundId {
    Footstep,
    ChaseFootstep,
    Jump,
    Land,
    Collect,
    Sting,
    Caught,
    Victory,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AmbienceId {
    Menu,
    Threat,
}

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
