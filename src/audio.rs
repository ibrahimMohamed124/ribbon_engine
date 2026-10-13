//! Sound, decoupled from any audio library.
//!
//! Gameplay code only pushes [`AudioEvent`]s (see `Game::events`). `main`
//! drains them into an [`Audio`], which scales by the master volume and hands
//! them to an [`AudioBackend`]. To get real sound, implement `AudioBackend`
//! (e.g. on top of `rodio`, loading files from `assets/audio/`) and pass it to
//! `Audio::new`; nothing else in the game has to change.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioEvent {
    Jump,
    Land,
    PageCollected,
    Caught,
    Won,
}

pub trait AudioBackend {
    /// Play `event` at `volume` (already clamped to 0..=1 and > 0).
    fn play(&mut self, event: AudioEvent, volume: f32);
}

/// Backend that plays nothing. The default until a real one is plugged in.
pub struct NullBackend;

impl AudioBackend for NullBackend {
    fn play(&mut self, _event: AudioEvent, _volume: f32) {}
}

pub struct Audio {
    backend: Box<dyn AudioBackend>,
    master_volume: f32,
}

impl Audio {
    pub fn new(backend: Box<dyn AudioBackend>, master_volume: f32) -> Self {
        Audio {
            backend,
            master_volume: master_volume.clamp(0.0, 1.0),
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
