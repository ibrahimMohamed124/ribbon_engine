//! Game audio events translated to the reusable mixer and desktop sound bank.

use voxelworks_core::audio::{
    AmbienceAsset, AmbienceId, Audio as Mixer, AudioBackend as MixerBackend, SoundAsset, SoundBank,
    SoundId,
};

/// Game-specific sound IDs. Numeric values remain stable for saved/debugged data.
pub mod sounds {
    use voxelworks_core::audio::SoundId;

    pub const FOOTSTEP: SoundId = SoundId(0);
    pub const CHASE_FOOTSTEP: SoundId = SoundId(1);
    pub const JUMP: SoundId = SoundId(2);
    pub const LAND: SoundId = SoundId(3);
    pub const COLLECT: SoundId = SoundId(4);
    pub const STING: SoundId = SoundId(5);
    pub const CAUGHT: SoundId = SoundId(6);
    pub const VICTORY: SoundId = SoundId(7);
}

/// Game-specific ambience IDs.
pub mod ambiences {
    use voxelworks_core::audio::AmbienceId;

    pub const MENU: AmbienceId = AmbienceId(0);
    pub const THREAT: AmbienceId = AmbienceId(1);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioEvent {
    Footstep,
    Jump,
    Land,
    PageCollected,
    EnemySting,
    EnemyFootstep,
    Caught,
    Won,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpatialAudioEvent {
    pub event: AudioEvent,
    pub pan: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioAmbience {
    Menu,
    Enemy,
}

pub trait AudioBackend {
    fn play(&mut self, event: AudioEvent, volume: f32);

    fn play_spatial(&mut self, event: AudioEvent, volume: f32, _pan: f32) {
        self.play(event, volume);
    }

    fn set_ambience(&mut self, _ambience: Option<AudioAmbience>, _volume: f32) {}
}

struct EventBackend(Box<dyn AudioBackend>);

impl MixerBackend for EventBackend {
    fn play(&mut self, sound: SoundId, volume: f32) {
        if let Some(event) = event_for(sound) {
            self.0.play(event, volume);
        }
    }

    fn play_spatial(&mut self, sound: SoundId, volume: f32, pan: f32) {
        if let Some(event) = event_for(sound) {
            self.0.play_spatial(event, volume, pan);
        }
    }

    fn set_ambience(&mut self, ambience: Option<AmbienceId>, volume: f32) {
        self.0.set_ambience(ambience.and_then(ambience_for), volume);
    }
}

pub struct Audio {
    mixer: Mixer,
}

impl Audio {
    /// Adapter constructor retained for game and test backends.
    pub fn new(backend: Box<dyn AudioBackend>, master_volume: f32) -> Self {
        Self {
            mixer: Mixer::new(Box::new(EventBackend(backend)), master_volume),
        }
    }

    /// Construct with a backend that already accepts engine sound identifiers.
    pub fn with_engine_backend(backend: Box<dyn MixerBackend>, master_volume: f32) -> Self {
        Self {
            mixer: Mixer::new(backend, master_volume),
        }
    }

    pub fn null(master_volume: f32) -> Self {
        Self {
            mixer: Mixer::null(master_volume),
        }
    }

    pub fn volume(&self) -> f32 {
        self.mixer.volume()
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.mixer.set_volume(volume);
    }

    pub fn set_effects_volume(&mut self, volume: f32) {
        self.mixer.set_effects_volume(volume);
    }

    pub fn set_ambient_volume(&mut self, volume: f32) {
        self.mixer.set_ambient_volume(volume);
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.mixer.set_muted(muted);
    }

    pub fn set_ambience(&mut self, ambience: Option<AudioAmbience>, volume: f32) {
        self.mixer
            .set_ambience(ambience.map(sound_ambience), volume);
    }

    pub fn play(&mut self, event: AudioEvent) {
        self.mixer.play(sound_for(event));
    }

    pub fn play_spatial(&mut self, event: AudioEvent, pan: f32) {
        self.mixer.play_spatial(sound_for(event), pan);
    }

    pub fn play_all(&mut self, events: &mut Vec<AudioEvent>) {
        for event in events.drain(..) {
            self.play(event);
        }
    }

    pub fn play_spatial_all(&mut self, events: &mut Vec<SpatialAudioEvent>) {
        for event in events.drain(..) {
            self.play_spatial(event.event, event.pan);
        }
    }
}

fn sound_for(event: AudioEvent) -> SoundId {
    match event {
        AudioEvent::Footstep => sounds::FOOTSTEP,
        AudioEvent::Jump => sounds::JUMP,
        AudioEvent::Land => sounds::LAND,
        AudioEvent::PageCollected => sounds::COLLECT,
        AudioEvent::EnemySting => sounds::STING,
        AudioEvent::EnemyFootstep => sounds::CHASE_FOOTSTEP,
        AudioEvent::Caught => sounds::CAUGHT,
        AudioEvent::Won => sounds::VICTORY,
    }
}

fn event_for(sound: SoundId) -> Option<AudioEvent> {
    match sound {
        sounds::FOOTSTEP => Some(AudioEvent::Footstep),
        sounds::JUMP => Some(AudioEvent::Jump),
        sounds::LAND => Some(AudioEvent::Land),
        sounds::COLLECT => Some(AudioEvent::PageCollected),
        sounds::STING => Some(AudioEvent::EnemySting),
        sounds::CHASE_FOOTSTEP => Some(AudioEvent::EnemyFootstep),
        sounds::CAUGHT => Some(AudioEvent::Caught),
        sounds::VICTORY => Some(AudioEvent::Won),
        _ => None,
    }
}

fn sound_ambience(ambience: AudioAmbience) -> AmbienceId {
    match ambience {
        AudioAmbience::Menu => ambiences::MENU,
        AudioAmbience::Enemy => ambiences::THREAT,
    }
}

fn ambience_for(ambience: AmbienceId) -> Option<AudioAmbience> {
    match ambience {
        ambiences::MENU => Some(AudioAmbience::Menu),
        ambiences::THREAT => Some(AudioAmbience::Enemy),
        _ => None,
    }
}

fn sound(id: SoundId, data: &'static [u8], gain: f32, speed: f32, spatial: bool) -> SoundAsset {
    SoundAsset {
        id,
        data,
        gain,
        speed,
        spatial,
    }
}

pub fn sound_bank() -> SoundBank {
    SoundBank::new(
        vec![
            sound(
                sounds::FOOTSTEP,
                include_bytes!("../assets/audio/footstep.wav"),
                0.20,
                1.0,
                false,
            ),
            sound(
                sounds::CHASE_FOOTSTEP,
                include_bytes!("../assets/audio/footstep.wav"),
                0.44,
                0.72,
                true,
            ),
            sound(
                sounds::JUMP,
                include_bytes!("../assets/audio/jump.wav"),
                0.48,
                1.0,
                false,
            ),
            sound(
                sounds::LAND,
                include_bytes!("../assets/audio/footstep.wav"),
                0.28,
                1.0,
                false,
            ),
            sound(
                sounds::COLLECT,
                include_bytes!("../assets/audio/page_collect.wav"),
                0.62,
                1.0,
                false,
            ),
            sound(
                sounds::STING,
                include_bytes!("../assets/audio/enemy_sting.wav"),
                0.68,
                1.0,
                false,
            ),
            sound(
                sounds::CAUGHT,
                include_bytes!("../assets/audio/death.wav"),
                0.72,
                1.0,
                false,
            ),
            sound(
                sounds::VICTORY,
                include_bytes!("../assets/audio/victory.wav"),
                0.72,
                1.0,
                false,
            ),
        ],
        vec![
            AmbienceAsset {
                id: ambiences::MENU,
                data: include_bytes!("../assets/audio/menu_ambience.wav"),
                gain: 1.0,
            },
            AmbienceAsset {
                id: ambiences::THREAT,
                data: include_bytes!("../assets/audio/enemy_ambient.wav"),
                gain: 1.0,
            },
        ],
    )
}
