//! Sound, decoupled from any audio library.
//!
//! Gameplay code only pushes [`AudioEvent`]s (see `Game::events`). `main`
//! drains them into an [`Audio`], which scales by the master volume and hands
//! them to an [`AudioBackend`]. The executable uses Rodio to play embedded WAV
//! assets; the library remains independent of any particular audio device.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioEvent {
    Footstep,
    Jump,
    Land,
    PageCollected,
    EnemySting,
    Caught,
    Won,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioAmbience {
    Menu,
    Enemy,
}

pub trait AudioBackend {
    /// Play `event` at `volume` (already clamped to 0..=1 and > 0).
    fn play(&mut self, event: AudioEvent, volume: f32);

    /// Select and set the relative volume of a looping background track.
    fn set_ambience(&mut self, _ambience: Option<AudioAmbience>, _volume: f32) {}
}

/// Backend that plays nothing. The default until a real one is plugged in.
pub struct NullBackend;

impl AudioBackend for NullBackend {
    fn play(&mut self, _event: AudioEvent, _volume: f32) {}
}

pub struct Audio {
    backend: Box<dyn AudioBackend>,
    master_volume: f32,
    effects_volume: f32,
    ambient_volume: f32,
    muted: bool,
    ambience: Option<(AudioAmbience, f32)>,
}

impl Audio {
    pub fn new(backend: Box<dyn AudioBackend>, master_volume: f32) -> Self {
        Audio {
            backend,
            master_volume: master_volume.clamp(0.0, 1.0),
            effects_volume: 1.0,
            ambient_volume: 1.0,
            muted: false,
            ambience: None,
        }
    }

    pub fn null(master_volume: f32) -> Self {
        Audio::new(Box::new(NullBackend), master_volume)
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

    pub fn set_ambience(&mut self, ambience: Option<AudioAmbience>, volume: f32) {
        let volume = volume.clamp(0.0, 1.0);
        self.ambience = ambience.map(|ambience| (ambience, volume));
        self.update_ambience();
    }

    pub fn play(&mut self, event: AudioEvent) {
        let volume = self.effective_master_volume() * self.effects_volume;
        if volume > 0.0 {
            self.backend.play(event, volume);
        }
    }

    /// Play and remove every queued event.
    pub fn play_all(&mut self, events: &mut Vec<AudioEvent>) {
        for e in events.drain(..) {
            self.play(e);
        }
    }
}
