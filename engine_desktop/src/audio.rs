//! Rodio playback for sound data supplied by the game package.

use std::io::Cursor;

use blocks_engine::audio::{AmbienceId, AudioBackend, SoundBank, SoundId};
use rodio::source::Source;

pub struct RodioBackend {
    output: rodio::MixerDeviceSink,
    ambience: Option<(AmbienceId, rodio::Player)>,
    bank: SoundBank,
}

impl RodioBackend {
    pub fn new(bank: SoundBank) -> Result<Self, rodio::DeviceSinkError> {
        Ok(Self {
            output: rodio::DeviceSinkBuilder::open_default_sink()?,
            ambience: None,
            bank,
        })
    }
}

impl AudioBackend for RodioBackend {
    fn play(&mut self, sound: SoundId, volume: f32) {
        let Some(asset) = self.bank.sound(sound) else {
            return;
        };
        match rodio::Decoder::try_from(Cursor::new(asset.data)) {
            Ok(source) => self
                .output
                .mixer()
                .add(source.speed(asset.speed).amplify(volume * asset.gain)),
            Err(error) => eprintln!("could not decode {sound:?} audio: {error}"),
        }
    }

    fn play_spatial(&mut self, sound: SoundId, volume: f32, pan: f32) {
        let Some(asset) = self.bank.sound(sound) else {
            return;
        };
        if !asset.spatial {
            self.play(sound, volume);
            return;
        }
        let player = rodio::SpatialPlayer::connect_new(
            self.output.mixer(),
            [pan.clamp(-1.0, 1.0) * 0.6, 0.8, 0.0],
            [-0.12, 0.0, 0.0],
            [0.12, 0.0, 0.0],
        );
        if let Ok(source) = rodio::Decoder::try_from(Cursor::new(asset.data)) {
            player.append(source.speed(asset.speed).amplify(volume * asset.gain));
            player.detach();
        }
    }

    fn set_ambience(&mut self, ambience: Option<AmbienceId>, volume: f32) {
        let ambience = ambience.filter(|_| volume > 0.0);
        let Some(ambience) = ambience else {
            if let Some((_, player)) = self.ambience.take() {
                player.stop();
            }
            return;
        };
        if let Some((current, player)) = &self.ambience {
            if *current == ambience {
                let gain = self.bank.ambience(ambience).map_or(1.0, |asset| asset.gain);
                player.set_volume(volume * gain);
                return;
            }
        }
        if let Some((_, player)) = self.ambience.take() {
            player.stop();
        }
        let Some(asset) = self.bank.ambience(ambience) else {
            return;
        };
        let player = rodio::Player::connect_new(self.output.mixer());
        player.set_volume(volume * asset.gain);
        match rodio::Decoder::try_from(Cursor::new(asset.data)) {
            Ok(source) => {
                player.append(source.repeat_infinite());
                self.ambience = Some((ambience, player));
            }
            Err(error) => eprintln!("could not decode {ambience:?} audio: {error}"),
        }
    }
}
