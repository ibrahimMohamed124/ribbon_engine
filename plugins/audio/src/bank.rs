//! Platform-independent sound identifiers and sound-bank descriptors.

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

    pub fn sounds(&self) -> impl Iterator<Item = SoundAsset> + '_ {
        self.sounds.iter().copied()
    }
}
