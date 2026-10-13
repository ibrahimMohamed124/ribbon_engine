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
    ambience: Option<(AudioAmbience, f32)>,
}

impl Audio {
    pub fn new(backend: Box<dyn AudioBackend>, master_volume: f32) -> Self {
        Audio {
            backend,
            master_volume: master_volume.clamp(0.0, 1.0),
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
        let (ambience, relative_volume) = self
            .ambience
            .map(|(ambience, volume)| (Some(ambience), volume))
            .unwrap_or((None, 0.0));
        self.backend
            .set_ambience(ambience, relative_volume * self.master_volume);
    }

    pub fn set_ambience(&mut self, ambience: Option<AudioAmbience>, volume: f32) {
        let volume = volume.clamp(0.0, 1.0);
        self.ambience = ambience.map(|ambience| (ambience, volume));
        self.backend
            .set_ambience(ambience, volume * self.master_volume);
    }

    pub fn play(&mut self, event: AudioEvent) {
        if self.master_volume > 0.0 {
            self.backend.play(event, self.master_volume);
        }
    }

    /// Play and remove every queued event.
    pub fn play_all(&mut self, events: &mut Vec<AudioEvent>) {
        for e in events.drain(..) {
            self.play(e);
        }
    }
}
