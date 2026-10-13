use std::cell::RefCell;
use std::rc::Rc;

use raycast_blocks::audio::{Audio, AudioAmbience, AudioBackend, AudioEvent};

type Log = Rc<RefCell<Vec<(AudioEvent, f32)>>>;

struct Recorder(Log);

impl AudioBackend for Recorder {
    fn play(&mut self, event: AudioEvent, volume: f32) {
        self.0.borrow_mut().push((event, volume));
    }
}

fn audio(volume: f32) -> (Audio, Log) {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    (Audio::new(Box::new(Recorder(log.clone())), volume), log)
}

#[test]
fn events_reach_the_backend_with_master_volume() {
    let (mut a, log) = audio(0.5);
    a.play(AudioEvent::Jump);
    assert_eq!(*log.borrow(), vec![(AudioEvent::Jump, 0.5)]);
}

#[test]
fn volume_is_clamped() {
    let (a, _) = audio(3.0);
    assert_eq!(a.volume(), 1.0);
    let (mut a, _) = audio(0.5);
    a.set_volume(-1.0);
    assert_eq!(a.volume(), 0.0);
}

#[test]
fn muted_audio_plays_nothing() {
    let (mut a, log) = audio(0.0);
    a.play(AudioEvent::Caught);
    assert!(log.borrow().is_empty());
}

#[test]
fn play_all_drains_the_queue() {
    let (mut a, log) = audio(1.0);
    let mut queue = vec![AudioEvent::Jump, AudioEvent::Land];
    a.play_all(&mut queue);
    assert!(queue.is_empty());
    assert_eq!(log.borrow().len(), 2);
}

#[test]
fn null_backend_is_safe() {
    let mut a = Audio::null(1.0);
    a.play(AudioEvent::Won);
}

type AmbienceLog = Rc<RefCell<Vec<(Option<AudioAmbience>, f32)>>>;

struct AmbienceRecorder(AmbienceLog);

impl AudioBackend for AmbienceRecorder {
    fn play(&mut self, _event: AudioEvent, _volume: f32) {}

    fn set_ambience(&mut self, ambience: Option<AudioAmbience>, volume: f32) {
        self.0.borrow_mut().push((ambience, volume));
    }
}

#[test]
fn ambience_volume_tracks_master_volume() {
    let log: AmbienceLog = Rc::new(RefCell::new(Vec::new()));
    let mut audio = Audio::new(Box::new(AmbienceRecorder(log.clone())), 0.5);
    audio.set_ambience(Some(AudioAmbience::Enemy), 0.2);
    audio.set_volume(0.25);
    audio.set_ambience(None, 0.0);

    assert_eq!(
        *log.borrow(),
        vec![
            (Some(AudioAmbience::Enemy), 0.1),
            (Some(AudioAmbience::Enemy), 0.05),
            (None, 0.0),
        ]
    );
}
